use cms::{
    cert::CertificateChoices,
    content_info::ContentInfo,
    signed_data::{SignedData, SignerIdentifier, SignerInfo},
};
use der::{
    Decode, Encode, Reader, SliceReader,
    asn1::{ObjectIdentifier, OctetString},
    referenced::OwnedToRef,
};
use pkix_chain::{
    CrlChecker, DefaultVerifier, NoAiaFetcher, NoRevocation, OcspChecker, Profile,
    RevocationChecker, SignatureVerifier, TrustAnchor, ValidationPolicy, verify_chain,
    verify_time_stamper,
};
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};
use x509_cert::{Certificate, ext::pkix::SubjectKeyIdentifier};
use x509_tsp::TstInfo;

use crate::{
    PdfError,
    limits::Limits,
    signatures::{
        CmsParseStatus, CmsValidation, DigestMatchStatus, RevocationStatus, SignatureCryptoStatus,
        SignatureTrustOptions, TimestampInspection, TimestampStatus, TrustStatus,
    },
};

const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
const ID_TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const SHA256_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const SHA384_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const SHA512_WITH_RSA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_WITH_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const ID_KP_TIME_STAMPING: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8");

struct TimestampProfile;

impl Profile for TimestampProfile {
    fn id(&self) -> &'static str {
        "ietf.time-stamping-basic"
    }

    fn version(&self) -> &'static str {
        "RFC 3161"
    }

    fn policy(&self, now_unix: u64) -> ValidationPolicy {
        let mut policy = ValidationPolicy::new(now_unix);
        policy.required_leaf_eku = Some(vec![ID_KP_TIME_STAMPING]);
        policy
    }

    fn policy_oids(&self) -> &[ObjectIdentifier] {
        &[]
    }
}

struct TrustInputs<'a> {
    roots_der: &'a [Vec<u8>],
    os_roots_der: &'a [Vec<u8>],
    intermediates_der: &'a [Vec<u8>],
    fetched_intermediates_der: &'a [Vec<u8>],
    crls_der: &'a [Vec<u8>],
    ocsp_responses_der: &'a [Vec<u8>],
    time: Option<u64>,
    timestamp: bool,
}

struct RevocationEvidence {
    crls: Vec<CrlChecker<DefaultVerifier>>,
    ocsps: Vec<OcspChecker<DefaultVerifier>>,
}

impl RevocationChecker for RevocationEvidence {
    fn check_revocation(
        &self,
        cert: &Certificate,
        issuer: &Certificate,
    ) -> pkix_chain::pkix_revocation::Result<()> {
        let mut good = false;
        let mut last = None;
        for checker in &self.crls {
            match checker.check_revocation(cert, issuer) {
                Ok(()) => good = true,
                Err(error @ pkix_chain::pkix_revocation::Error::Revoked { .. }) => {
                    return Err(error);
                }
                Err(error) => last = Some(error),
            }
        }
        for checker in &self.ocsps {
            match checker.check_revocation(cert, issuer) {
                Ok(()) => good = true,
                Err(error @ pkix_chain::pkix_revocation::Error::Revoked { .. }) => {
                    return Err(error);
                }
                Err(error) => last = Some(error),
            }
        }
        if good {
            Ok(())
        } else {
            Err(last.expect("RevocationEvidence is constructed non-empty"))
        }
    }

    fn check_revocation_against_anchor(
        &self,
        cert: &Certificate,
        anchor: &TrustAnchor,
    ) -> pkix_chain::pkix_revocation::Result<()> {
        let mut good = false;
        let mut last = None;
        for checker in &self.crls {
            match checker.check_revocation_against_anchor(cert, anchor) {
                Ok(()) => good = true,
                Err(error @ pkix_chain::pkix_revocation::Error::Revoked { .. }) => {
                    return Err(error);
                }
                Err(error) => last = Some(error),
            }
        }
        for checker in &self.ocsps {
            match checker.check_revocation_against_anchor(cert, anchor) {
                Ok(()) => good = true,
                Err(error @ pkix_chain::pkix_revocation::Error::Revoked { .. }) => {
                    return Err(error);
                }
                Err(error) => last = Some(error),
            }
        }
        if good {
            Ok(())
        } else {
            Err(last.expect("RevocationEvidence is constructed non-empty"))
        }
    }
}

pub(crate) fn validate_cms(
    contents: &[u8],
    signed_ranges: &[&[u8]],
    trust: &SignatureTrustOptions,
    limits: &Limits,
) -> Result<CmsValidation, PdfError> {
    if contents.len() > limits.max_stream_bytes {
        return Err(PdfError::limit("CMS contents exceed max_stream_bytes"));
    }
    if contents.is_empty() || contents.iter().all(|byte| *byte == 0) {
        return Ok(CmsValidation::default());
    }
    let der = match first_der_element(contents) {
        Ok(value) => value,
        Err(error) => return Ok(CmsValidation::malformed(error)),
    };
    let content_info = match ContentInfo::from_der(der) {
        Ok(value) => value,
        Err(error) => return Ok(CmsValidation::malformed(error.to_string())),
    };
    if content_info.content_type != ID_SIGNED_DATA {
        return Ok(CmsValidation {
            parse_status: CmsParseStatus::UnsupportedContentType,
            ..CmsValidation::default()
        });
    }
    let signed_der = match content_info.content.to_der() {
        Ok(value) => value,
        Err(error) => return Ok(CmsValidation::malformed(error.to_string())),
    };
    let signed_data = match SignedData::from_der(&signed_der) {
        Ok(value) => value,
        Err(error) => return Ok(CmsValidation::malformed(error.to_string())),
    };
    let certificates = certificates(&signed_data, limits)?;
    let signers: Vec<_> = signed_data.signer_infos.0.iter().collect();
    if signers.len() > limits.max_container_items {
        return Err(PdfError::limit("CMS signer count exceeds container limit"));
    }
    let Some(signer) = signers.first().copied() else {
        return Ok(CmsValidation::malformed("CMS SignedData has no signer"));
    };
    let signer_certificate = signer_certificate(signer, &certificates);
    let digest_algorithm = digest_name(signer.digest_alg.oid).map(str::to_owned);
    let signed_digest = message_digest(signer, limits)?;
    let byte_range_digest = digest_algorithm
        .as_deref()
        .and_then(|algorithm| digest_ranges(algorithm, signed_ranges));
    let digest_status = match (&signed_digest, &byte_range_digest, &digest_algorithm) {
        (Some(expected), Some(actual), _) if expected.as_slice() == actual => {
            DigestMatchStatus::Match
        }
        (Some(_), Some(_), _) => DigestMatchStatus::Mismatch,
        (None, _, _) => DigestMatchStatus::MissingAttribute,
        (_, None, None) => DigestMatchStatus::UnsupportedAlgorithm,
        _ => DigestMatchStatus::NotPerformed,
    };
    let signature_status = verify_signature(signer, signer_certificate, limits)?;
    let inputs = TrustInputs {
        roots_der: &trust.roots_der,
        os_roots_der: &trust.os_roots_der,
        intermediates_der: &trust.intermediates_der,
        fetched_intermediates_der: &trust.fetched_intermediates_der,
        crls_der: &trust.crls_der,
        ocsp_responses_der: &trust.ocsp_responses_der,
        time: trust.validation_time_unix,
        timestamp: false,
    };
    let trust_status = validate_trust(signer_certificate, &certificates, &inputs, limits);
    let revocation_status = validate_revocation(signer_certificate, &certificates, &inputs, limits);
    let timestamp = inspect_timestamp(signer, trust, limits)?;

    Ok(CmsValidation {
        parse_status: CmsParseStatus::Parsed,
        error: None,
        signer_count: signers.len(),
        certificate_count: certificates.len(),
        digest_algorithm,
        digest_status,
        signature_status,
        trust_status,
        revocation_status,
        signer_certificate_subject: signer_certificate
            .map(|certificate| certificate.tbs_certificate.subject.to_string()),
        signer_certificate_issuer: signer_certificate
            .map(|certificate| certificate.tbs_certificate.issuer.to_string()),
        signer_certificate_serial: signer_certificate
            .map(|certificate| hex(certificate.tbs_certificate.serial_number.as_bytes())),
        timestamp,
    })
}

fn first_der_element(input: &[u8]) -> Result<&[u8], String> {
    let mut reader = SliceReader::new(input).map_err(|error| error.to_string())?;
    let value = reader.tlv_bytes().map_err(|error| error.to_string())?;
    let remaining = reader
        .read_slice(reader.remaining_len())
        .map_err(|error| error.to_string())?;
    if remaining.iter().any(|byte| *byte != 0) {
        return Err("CMS contents have non-zero trailing data".into());
    }
    Ok(value)
}

fn certificates(signed_data: &SignedData, limits: &Limits) -> Result<Vec<Certificate>, PdfError> {
    let mut output = Vec::new();
    if let Some(certificates) = &signed_data.certificates {
        for choice in certificates.0.iter() {
            if output.len() >= limits.max_container_items {
                return Err(PdfError::limit(
                    "CMS certificate count exceeds container limit",
                ));
            }
            if let CertificateChoices::Certificate(certificate) = choice {
                let length = certificate
                    .to_der()
                    .map_err(|error| {
                        PdfError::syntax(format!("invalid CMS certificate: {error}"), 0)
                    })?
                    .len();
                if length > limits.max_stream_bytes {
                    return Err(PdfError::limit("CMS certificate exceeds max_stream_bytes"));
                }
                output.push(certificate.clone());
            }
        }
    }
    Ok(output)
}

fn signer_certificate<'a>(
    signer: &SignerInfo,
    certificates: &'a [Certificate],
) -> Option<&'a Certificate> {
    certificates.iter().find(|certificate| match &signer.sid {
        SignerIdentifier::IssuerAndSerialNumber(identifier) => {
            certificate.tbs_certificate.issuer == identifier.issuer
                && certificate.tbs_certificate.serial_number == identifier.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(identifier) => certificate
            .tbs_certificate
            .get::<SubjectKeyIdentifier>()
            .ok()
            .flatten()
            .is_some_and(|(_, value)| value == *identifier),
    })
}

fn message_digest(signer: &SignerInfo, limits: &Limits) -> Result<Option<Vec<u8>>, PdfError> {
    let Some(attributes) = &signer.signed_attrs else {
        return Ok(None);
    };
    if attributes.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "CMS signed attribute count exceeds container limit",
        ));
    }
    for attribute in attributes.iter() {
        if attribute.oid != ID_MESSAGE_DIGEST {
            continue;
        }
        if attribute.values.len() != 1 {
            return Ok(None);
        }
        let value = attribute
            .values
            .get(0)
            .ok_or_else(|| PdfError::syntax("CMS messageDigest attribute is empty", 0))?;
        let digest = value.decode_as::<OctetString>().map_err(|error| {
            PdfError::syntax(format!("invalid CMS messageDigest attribute: {error}"), 0)
        })?;
        return Ok(Some(digest.as_bytes().to_vec()));
    }
    Ok(None)
}

fn verify_signature(
    signer: &SignerInfo,
    certificate: Option<&Certificate>,
    limits: &Limits,
) -> Result<SignatureCryptoStatus, PdfError> {
    let Some(certificate) = certificate else {
        return Ok(SignatureCryptoStatus::SignerCertificateMissing);
    };
    let Some(attributes) = &signer.signed_attrs else {
        return Ok(SignatureCryptoStatus::NotPerformed);
    };
    let signed = attributes
        .to_der()
        .map_err(|error| PdfError::syntax(format!("invalid CMS signed attributes: {error}"), 0))?;
    if signed.len() > limits.max_stream_bytes {
        return Err(PdfError::limit(
            "CMS signed attributes exceed max_stream_bytes",
        ));
    }
    let mut algorithm = signer.signature_algorithm.clone();
    if algorithm.oid == RSA_ENCRYPTION {
        algorithm.oid = match digest_name(signer.digest_alg.oid) {
            Some("sha256") => SHA256_WITH_RSA,
            Some("sha384") => SHA384_WITH_RSA,
            Some("sha512") => SHA512_WITH_RSA,
            _ => return Ok(SignatureCryptoStatus::UnsupportedAlgorithm),
        };
    }
    if !matches!(
        algorithm.oid,
        SHA256_WITH_RSA | SHA384_WITH_RSA | SHA512_WITH_RSA | ECDSA_WITH_SHA256 | ECDSA_WITH_SHA384
    ) {
        return Ok(SignatureCryptoStatus::UnsupportedAlgorithm);
    }
    let signature = signer.signature.as_bytes();
    Ok(
        if DefaultVerifier
            .verify_signature(
                algorithm.owned_to_ref(),
                certificate
                    .tbs_certificate
                    .subject_public_key_info
                    .owned_to_ref(),
                &signed,
                signature,
            )
            .is_ok()
        {
            SignatureCryptoStatus::Valid
        } else {
            SignatureCryptoStatus::Invalid
        },
    )
}

fn validate_trust(
    signer: Option<&Certificate>,
    embedded: &[Certificate],
    inputs: &TrustInputs<'_>,
    limits: &Limits,
) -> TrustStatus {
    if inputs.roots_der.is_empty() && inputs.os_roots_der.is_empty() {
        return TrustStatus::NotRequested;
    }
    let Some(signer) = signer else {
        return TrustStatus::SignerCertificateMissing;
    };
    let Some(time) = inputs.time else {
        return TrustStatus::ValidationTimeMissing;
    };
    let Some((chain, anchors)) = chain_inputs(signer, embedded, inputs, limits) else {
        return TrustStatus::InvalidInput;
    };
    let valid = if inputs.timestamp {
        verify_time_stamper(
            &chain,
            &anchors,
            &TimestampProfile,
            time,
            &DefaultVerifier,
            &NoRevocation,
            &NoAiaFetcher,
        )
        .is_ok()
    } else {
        verify_chain(
            &chain,
            &anchors,
            &ValidationPolicy::new(time),
            &DefaultVerifier,
            &NoRevocation,
            &NoAiaFetcher,
        )
        .is_ok()
    };
    if valid {
        TrustStatus::Trusted
    } else {
        TrustStatus::Untrusted
    }
}

fn validate_revocation(
    signer: Option<&Certificate>,
    embedded: &[Certificate],
    inputs: &TrustInputs<'_>,
    limits: &Limits,
) -> RevocationStatus {
    if inputs.crls_der.is_empty() && inputs.ocsp_responses_der.is_empty() {
        return RevocationStatus::NotRequested;
    }
    let (Some(signer), Some(time)) = (signer, inputs.time) else {
        return RevocationStatus::InvalidInput;
    };
    if inputs
        .crls_der
        .len()
        .checked_add(inputs.ocsp_responses_der.len())
        .is_none_or(|count| count > limits.max_container_items)
        || inputs
            .crls_der
            .iter()
            .chain(inputs.ocsp_responses_der)
            .any(|der| der.len() > limits.max_stream_bytes)
    {
        return RevocationStatus::InvalidInput;
    }
    let Some((chain, anchors)) = chain_inputs(signer, embedded, inputs, limits) else {
        return RevocationStatus::InvalidInput;
    };
    let Some(crls) = inputs
        .crls_der
        .iter()
        .map(|der| CrlChecker::new(der, time, DefaultVerifier).ok())
        .collect::<Option<Vec<_>>>()
    else {
        return RevocationStatus::InvalidInput;
    };
    let Some(ocsps) = inputs
        .ocsp_responses_der
        .iter()
        .map(|der| OcspChecker::new(der, time, DefaultVerifier).ok())
        .collect::<Option<Vec<_>>>()
    else {
        return RevocationStatus::InvalidInput;
    };
    let evidence = RevocationEvidence { crls, ocsps };
    let result = if inputs.timestamp {
        verify_time_stamper(
            &chain,
            &anchors,
            &TimestampProfile,
            time,
            &DefaultVerifier,
            &evidence,
            &NoAiaFetcher,
        )
    } else {
        verify_chain(
            &chain,
            &anchors,
            &ValidationPolicy::new(time),
            &DefaultVerifier,
            &evidence,
            &NoAiaFetcher,
        )
    };
    match result {
        Ok(_) => RevocationStatus::Good,
        Err(pkix_chain::Error::Revocation(pkix_chain::pkix_revocation::Error::Revoked {
            ..
        })) => RevocationStatus::Revoked,
        Err(pkix_chain::Error::Revocation(_)) => RevocationStatus::Indeterminate,
        Err(_) => RevocationStatus::Indeterminate,
    }
}

fn chain_inputs(
    signer: &Certificate,
    embedded: &[Certificate],
    inputs: &TrustInputs<'_>,
    limits: &Limits,
) -> Option<(Vec<Certificate>, Vec<TrustAnchor>)> {
    if (inputs.roots_der.is_empty() && inputs.os_roots_der.is_empty())
        || inputs
            .roots_der
            .len()
            .checked_add(inputs.os_roots_der.len())
            .and_then(|count| count.checked_add(inputs.intermediates_der.len()))
            .and_then(|count| count.checked_add(inputs.fetched_intermediates_der.len()))
            .and_then(|count| count.checked_add(embedded.len()))
            .is_none_or(|count| count > limits.max_container_items)
    {
        return None;
    }
    let anchors = parse_certificate_sources(&[inputs.roots_der, inputs.os_roots_der], limits)?
        .into_iter()
        .map(TrustAnchor::from_cert)
        .collect();
    let mut chain = vec![signer.clone()];
    chain.extend(
        embedded
            .iter()
            .filter(|certificate| *certificate != signer)
            .cloned(),
    );
    chain.extend(parse_certificate_sources(
        &[inputs.intermediates_der, inputs.fetched_intermediates_der],
        limits,
    )?);
    Some((chain, anchors))
}

fn parse_certificate_sources(sources: &[&[Vec<u8>]], limits: &Limits) -> Option<Vec<Certificate>> {
    sources
        .iter()
        .flat_map(|source| source.iter())
        .map(|der| {
            if der.len() > limits.max_stream_bytes {
                None
            } else {
                Certificate::from_der(der).ok()
            }
        })
        .collect()
}

fn inspect_timestamp(
    signer: &SignerInfo,
    trust: &SignatureTrustOptions,
    limits: &Limits,
) -> Result<TimestampInspection, PdfError> {
    let Some(attributes) = &signer.unsigned_attrs else {
        return Ok(TimestampInspection::default());
    };
    if attributes.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "CMS unsigned attribute count exceeds container limit",
        ));
    }
    let Some(attribute) = attributes
        .iter()
        .find(|attribute| attribute.oid == ID_TIMESTAMP_TOKEN)
    else {
        return Ok(TimestampInspection::default());
    };
    if attribute.values.len() != 1 {
        return Ok(TimestampInspection::malformed(
            "timestamp attribute must contain one token",
        ));
    }
    let value = attribute
        .values
        .get(0)
        .ok_or_else(|| PdfError::syntax("timestamp attribute is empty", 0))?;
    let token = match value.decode_as::<ContentInfo>() {
        Ok(value) => value,
        Err(error) => return Ok(TimestampInspection::malformed(error.to_string())),
    };
    if token.content_type != ID_SIGNED_DATA {
        return Ok(TimestampInspection::malformed(
            "timestamp token is not CMS SignedData",
        ));
    }
    let signed_der = match token.content.to_der() {
        Ok(value) => value,
        Err(error) => return Ok(TimestampInspection::malformed(error.to_string())),
    };
    let signed_data = match SignedData::from_der(&signed_der) {
        Ok(value) => value,
        Err(error) => return Ok(TimestampInspection::malformed(error.to_string())),
    };
    if signed_data.encap_content_info.econtent_type != ID_TST_INFO {
        return Ok(TimestampInspection::malformed(
            "timestamp token content is not TSTInfo",
        ));
    }
    let Some(ref content) = signed_data.encap_content_info.econtent else {
        return Ok(TimestampInspection::malformed(
            "timestamp token has no TSTInfo content",
        ));
    };
    let info = match TstInfo::from_der(content.value()) {
        Ok(value) => value,
        Err(error) => return Ok(TimestampInspection::malformed(error.to_string())),
    };
    let algorithm = digest_name(info.message_imprint.hash_algorithm.oid).map(str::to_owned);
    let expected = info.message_imprint.hashed_message.as_bytes();
    let actual = algorithm
        .as_deref()
        .and_then(|algorithm| digest_ranges(algorithm, &[signer.signature.as_bytes()]));
    let status = match actual {
        Some(actual) if actual == expected => TimestampStatus::ImprintMatch,
        Some(_) => TimestampStatus::ImprintMismatch,
        None => TimestampStatus::UnsupportedAlgorithm,
    };
    let certificates = certificates(&signed_data, limits)?;
    let timestamp_signers: Vec<_> = signed_data.signer_infos.0.iter().collect();
    if timestamp_signers.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "timestamp signer count exceeds container limit",
        ));
    }
    let timestamp_signer = (timestamp_signers.len() == 1).then(|| timestamp_signers[0]);
    let token_error = (timestamp_signers.len() != 1)
        .then(|| "timestamp token must contain exactly one signer".to_owned());
    let timestamp_certificate =
        timestamp_signer.and_then(|signer| signer_certificate(signer, &certificates));
    let signed_digest = match timestamp_signer {
        Some(signer) => message_digest(signer, limits)?,
        None => None,
    };
    let content_digest = timestamp_signer
        .and_then(|signer| digest_name(signer.digest_alg.oid))
        .and_then(|algorithm| digest_ranges(algorithm, &[content.value()]));
    let digest_status = match (signed_digest, content_digest) {
        (Some(expected), Some(actual)) if expected == actual => DigestMatchStatus::Match,
        (Some(_), Some(_)) => DigestMatchStatus::Mismatch,
        (None, _) => DigestMatchStatus::MissingAttribute,
        (_, None) => DigestMatchStatus::UnsupportedAlgorithm,
    };
    let signature_status = match timestamp_signer {
        Some(signer) => verify_signature(signer, timestamp_certificate, limits)?,
        None => SignatureCryptoStatus::NotPerformed,
    };
    let generation_time_unix = info.gen_time.to_unix_duration().as_secs();
    let inputs = TrustInputs {
        roots_der: &trust.tsa_roots_der,
        os_roots_der: &trust.tsa_os_roots_der,
        intermediates_der: &trust.tsa_intermediates_der,
        fetched_intermediates_der: &trust.tsa_fetched_intermediates_der,
        crls_der: &trust.tsa_crls_der,
        ocsp_responses_der: &trust.tsa_ocsp_responses_der,
        time: Some(generation_time_unix),
        timestamp: true,
    };
    let trust_status = validate_trust(timestamp_certificate, &certificates, &inputs, limits);
    let revocation_status =
        validate_revocation(timestamp_certificate, &certificates, &inputs, limits);
    Ok(TimestampInspection {
        status,
        error: token_error,
        digest_algorithm: algorithm,
        hashed_message: Some(hex(expected)),
        generation_time_unix: Some(generation_time_unix),
        digest_status,
        signature_status,
        trust_status,
        revocation_status,
    })
}

fn digest_name(oid: ObjectIdentifier) -> Option<&'static str> {
    match oid.to_string().as_str() {
        "1.3.14.3.2.26" => Some("sha1"),
        "2.16.840.1.101.3.4.2.4" => Some("sha224"),
        "2.16.840.1.101.3.4.2.1" => Some("sha256"),
        "2.16.840.1.101.3.4.2.2" => Some("sha384"),
        "2.16.840.1.101.3.4.2.3" => Some("sha512"),
        _ => None,
    }
}

fn digest_ranges(algorithm: &str, ranges: &[&[u8]]) -> Option<Vec<u8>> {
    macro_rules! hash {
        ($algorithm:ty) => {{
            let mut digest = <$algorithm>::new();
            for range in ranges {
                digest.update(range);
            }
            Some(digest.finalize().to_vec())
        }};
    }
    match algorithm {
        "sha1" => hash!(Sha1),
        "sha224" => hash!(Sha224),
        "sha256" => hash!(Sha256),
        "sha384" => hash!(Sha384),
        "sha512" => hash!(Sha512),
        _ => None,
    }
}

fn hex(input: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(input.len() * 2);
    for byte in input {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    output
}
