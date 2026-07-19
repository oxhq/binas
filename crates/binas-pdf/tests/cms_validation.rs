use cms::{
    cert::{CertificateChoices, IssuerAndSerialNumber},
    content_info::{CmsVersion, ContentInfo},
    signed_data::{
        CertificateSet, DigestAlgorithmIdentifiers, EncapsulatedContentInfo, SignatureValue,
        SignedAttributes, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
    },
};
use der::{
    Any, AnyRef, DateTime, Encode,
    asn1::{BitString, GeneralizedTime, Int, ObjectIdentifier, OctetString, SetOfVec},
};
use rcgen::{
    BasicConstraints, CertificateParams, CertificateRevocationListParams, CertifiedIssuer,
    CustomExtension, DnType, IsCa, KeyIdMethod, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
    RevokedCertParams, SerialNumber, SigningKey, date_time_ymd,
};
use sha2::{Digest, Sha256};
use x509_cert::{Certificate, attr::Attribute, der::Decode, ext::pkix::ExtendedKeyUsage};
use x509_ocsp::{
    BasicOcspResponse, CertId, CertStatus, OcspGeneralizedTime, OcspResponse, ResponderId,
    ResponseData, RevokedInfo, SingleResponse, Version,
};
use x509_tsp::{MessageImprint, TspVersion, TstInfo};

use binas_pdf::{
    CmsParseStatus, DigestMatchStatus, EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode,
    RevocationStatus, SignatureCryptoStatus, SignatureTrustOptions, TimestampStatus, TrustStatus,
    inspect_signatures, inspect_signatures_with_options,
};

const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_TIMESTAMP_TOKEN: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.14");
const ID_TST_INFO: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4");
const ID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ID_KP_TIME_STAMPING: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.8");

struct Fixture {
    pdf: Vec<u8>,
    root_der: Vec<u8>,
    intermediate_der: Vec<u8>,
    crls_der: Vec<Vec<u8>>,
    ocsp_good_der: Vec<Vec<u8>>,
    ocsp_revoked_der: Vec<Vec<u8>>,
    ocsp_unknown_der: Vec<Vec<u8>>,
    ocsp_tampered_der: Vec<Vec<u8>>,
    ocsp_stale_der: Vec<Vec<u8>>,
    tsa_root_der: Vec<u8>,
    tsa_crl_der: Vec<u8>,
}

#[derive(Clone, Copy)]
enum OcspFixtureStatus {
    Good,
    Revoked,
    Unknown,
    Tampered,
    Stale,
}

fn fixture() -> Fixture {
    fixture_with_timestamp(true, true, false)
}

fn fixture_with_timestamp(
    timestamp_matches: bool,
    timestamp_signature_valid: bool,
    revoke_tsa: bool,
) -> Fixture {
    let root_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut root_params = CertificateParams::default();
    root_params
        .distinguished_name
        .push(DnType::CommonName, "Binas Test Root");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    root_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let root = CertifiedIssuer::self_signed(root_params, root_key).unwrap();

    let intermediate_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut intermediate_params = CertificateParams::default();
    intermediate_params.serial_number = Some(SerialNumber::from(6_u64));
    intermediate_params
        .distinguished_name
        .push(DnType::CommonName, "Binas Test Intermediate");
    intermediate_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    intermediate_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let intermediate =
        CertifiedIssuer::signed_by(intermediate_params, intermediate_key, &root).unwrap();
    let leaf_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut leaf_params = CertificateParams::default();
    leaf_params.serial_number = Some(SerialNumber::from(7_u64));
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "Binas Test Signer");
    leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let leaf = leaf_params.signed_by(&leaf_key, &intermediate).unwrap();
    let leaf_crl_der = CertificateRevocationListParams {
        this_update: date_time_ymd(2026, 1, 1),
        next_update: date_time_ymd(2028, 1, 1),
        crl_number: SerialNumber::from(1_u64),
        issuing_distribution_point: None,
        revoked_certs: vec![],
        key_identifier_method: KeyIdMethod::Sha256,
    }
    .signed_by(&intermediate)
    .unwrap()
    .der()
    .to_vec();
    let intermediate_crl_der = CertificateRevocationListParams {
        this_update: date_time_ymd(2026, 1, 1),
        next_update: date_time_ymd(2028, 1, 1),
        crl_number: SerialNumber::from(2_u64),
        issuing_distribution_point: None,
        revoked_certs: vec![],
        key_identifier_method: KeyIdMethod::Sha256,
    }
    .signed_by(&root)
    .unwrap()
    .der()
    .to_vec();
    let root_certificate = Certificate::from_der(root.der()).unwrap();
    let intermediate_certificate = Certificate::from_der(intermediate.der()).unwrap();
    let leaf_certificate = Certificate::from_der(leaf.der()).unwrap();
    let intermediate_ocsp = ocsp_response(
        &root_certificate,
        &intermediate_certificate,
        root.key(),
        OcspFixtureStatus::Good,
    );
    let ocsp_responses = |status| {
        vec![
            ocsp_response(
                &intermediate_certificate,
                &leaf_certificate,
                intermediate.key(),
                status,
            ),
            intermediate_ocsp.clone(),
        ]
    };

    let tsa_root_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut tsa_root_params = CertificateParams::default();
    tsa_root_params
        .distinguished_name
        .push(DnType::CommonName, "Binas TSA Root");
    tsa_root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    tsa_root_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::CrlSign,
    ];
    let tsa_root = CertifiedIssuer::self_signed(tsa_root_params, tsa_root_key).unwrap();
    let tsa_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut tsa_params = CertificateParams::default();
    tsa_params.serial_number = Some(SerialNumber::from(42_u64));
    tsa_params
        .distinguished_name
        .push(DnType::CommonName, "Binas Test TSA");
    tsa_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    let mut eku = CustomExtension::from_oid_content(
        &[2, 5, 29, 37],
        ExtendedKeyUsage(vec![ID_KP_TIME_STAMPING])
            .to_der()
            .unwrap(),
    );
    eku.set_criticality(true);
    tsa_params.custom_extensions.push(eku);
    let tsa = tsa_params.signed_by(&tsa_key, &tsa_root).unwrap();
    let tsa_crl_der = CertificateRevocationListParams {
        this_update: date_time_ymd(2026, 1, 1),
        next_update: date_time_ymd(2028, 1, 1),
        crl_number: SerialNumber::from(1_u64),
        issuing_distribution_point: None,
        revoked_certs: if revoke_tsa {
            vec![RevokedCertParams {
                serial_number: SerialNumber::from(42_u64),
                revocation_time: date_time_ymd(2026, 1, 2),
                reason_code: None,
                invalidity_date: None,
            }]
        } else {
            vec![]
        },
        key_identifier_method: KeyIdMethod::Sha256,
    }
    .signed_by(&tsa_root)
    .unwrap()
    .der()
    .to_vec();

    let mut pdf = signature_pdf(4096);
    let ranges = ranges(&pdf);
    let digest = digest_ranges(&pdf, ranges);
    let cms = detached_cms(
        &digest,
        leaf.der().as_ref(),
        &leaf_key,
        timestamp_matches,
        timestamp_signature_valid,
        tsa.der().as_ref(),
        &tsa_key,
    );
    let marker = b"/Contents <";
    let start = pdf
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    let end = pdf[start..].iter().position(|byte| *byte == b'>').unwrap() + start;
    let encoded = hex(&cms);
    assert!(encoded.len() <= end - start);
    pdf[start..start + encoded.len()].copy_from_slice(encoded.as_bytes());

    Fixture {
        pdf,
        root_der: root.der().to_vec(),
        intermediate_der: intermediate.der().to_vec(),
        crls_der: vec![leaf_crl_der, intermediate_crl_der],
        ocsp_good_der: ocsp_responses(OcspFixtureStatus::Good),
        ocsp_revoked_der: ocsp_responses(OcspFixtureStatus::Revoked),
        ocsp_unknown_der: ocsp_responses(OcspFixtureStatus::Unknown),
        ocsp_tampered_der: ocsp_responses(OcspFixtureStatus::Tampered),
        ocsp_stale_der: ocsp_responses(OcspFixtureStatus::Stale),
        tsa_root_der: tsa_root.der().to_vec(),
        tsa_crl_der,
    }
}

fn detached_cms(
    digest: &[u8],
    certificate_der: &[u8],
    key: &KeyPair,
    timestamp_matches: bool,
    timestamp_signature_valid: bool,
    tsa_certificate_der: &[u8],
    tsa_key: &KeyPair,
) -> Vec<u8> {
    let certificate = Certificate::from_der(certificate_der).unwrap();
    let digest_algorithm = algorithm(ID_SHA256);
    let attributes = SignedAttributes::try_from(vec![
        Attribute {
            oid: ID_CONTENT_TYPE,
            values: SetOfVec::try_from(vec![Any::encode_from(&ID_DATA).unwrap()]).unwrap(),
        },
        Attribute {
            oid: ID_MESSAGE_DIGEST,
            values: SetOfVec::try_from(vec![
                Any::encode_from(&OctetString::new(digest).unwrap()).unwrap(),
            ])
            .unwrap(),
        },
    ])
    .unwrap();
    let signature = key.sign(&attributes.to_der().unwrap()).unwrap();
    let timestamp = timestamp_token(
        &signature,
        timestamp_matches,
        timestamp_signature_valid,
        tsa_certificate_der,
        tsa_key,
    );
    let signer = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: certificate.tbs_certificate.issuer.clone(),
            serial_number: certificate.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: digest_algorithm.clone(),
        signed_attrs: Some(attributes),
        signature_algorithm: algorithm(ECDSA_WITH_SHA256),
        signature: SignatureValue::new(signature).unwrap(),
        unsigned_attrs: Some(
            SignedAttributes::try_from(vec![Attribute {
                oid: ID_TIMESTAMP_TOKEN,
                values: SetOfVec::try_from(vec![Any::from(
                    AnyRef::try_from(timestamp.as_slice()).unwrap(),
                )])
                .unwrap(),
            }])
            .unwrap(),
        ),
    };
    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: DigestAlgorithmIdentifiers::try_from(vec![digest_algorithm]).unwrap(),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: ID_DATA,
            econtent: None,
        },
        certificates: Some(
            CertificateSet::try_from(vec![CertificateChoices::Certificate(certificate)]).unwrap(),
        ),
        crls: None,
        signer_infos: SignerInfos::try_from(vec![signer]).unwrap(),
    };
    let signed_der = signed_data.to_der().unwrap();
    ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: Any::from(AnyRef::try_from(signed_der.as_slice()).unwrap()),
    }
    .to_der()
    .unwrap()
}

fn timestamp_token(
    signature: &[u8],
    matches: bool,
    signature_valid: bool,
    certificate_der: &[u8],
    key: &KeyPair,
) -> Vec<u8> {
    let mut imprint = Sha256::digest(signature);
    if !matches {
        imprint[0] ^= 1;
    }
    let tst_info = TstInfo {
        version: TspVersion::V1,
        policy: ObjectIdentifier::new_unwrap("1.2.3.4.1"),
        message_imprint: MessageImprint {
            hash_algorithm: algorithm(ID_SHA256),
            hashed_message: OctetString::new(imprint.as_slice()).unwrap(),
        },
        serial_number: Int::new(&[1]).unwrap(),
        gen_time: GeneralizedTime::from_unix_duration(std::time::Duration::from_secs(
            1_800_000_000,
        ))
        .unwrap(),
        accuracy: None,
        ordering: false,
        nonce: None,
        tsa: None,
        extensions: None,
    };
    let content = tst_info.to_der().unwrap();
    let attributes = SignedAttributes::try_from(vec![
        Attribute {
            oid: ID_CONTENT_TYPE,
            values: SetOfVec::try_from(vec![Any::encode_from(&ID_TST_INFO).unwrap()]).unwrap(),
        },
        Attribute {
            oid: ID_MESSAGE_DIGEST,
            values: SetOfVec::try_from(vec![
                Any::encode_from(&OctetString::new(Sha256::digest(&content).as_slice()).unwrap())
                    .unwrap(),
            ])
            .unwrap(),
        },
    ])
    .unwrap();
    let mut token_signature = key.sign(&attributes.to_der().unwrap()).unwrap();
    if !signature_valid {
        token_signature[0] ^= 1;
    }
    let certificate = Certificate::from_der(certificate_der).unwrap();
    let signer = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: certificate.tbs_certificate.issuer.clone(),
            serial_number: certificate.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: algorithm(ID_SHA256),
        signed_attrs: Some(attributes),
        signature_algorithm: algorithm(ECDSA_WITH_SHA256),
        signature: SignatureValue::new(token_signature).unwrap(),
        unsigned_attrs: None,
    };
    let timestamp_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: DigestAlgorithmIdentifiers::try_from(vec![algorithm(ID_SHA256)])
            .unwrap(),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: ID_TST_INFO,
            econtent: Some(Any::encode_from(&OctetString::new(content).unwrap()).unwrap()),
        },
        certificates: Some(
            CertificateSet::try_from(vec![CertificateChoices::Certificate(certificate)]).unwrap(),
        ),
        crls: None,
        signer_infos: SignerInfos::try_from(vec![signer]).unwrap(),
    };
    let timestamp_der = timestamp_data.to_der().unwrap();
    ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: Any::from(AnyRef::try_from(timestamp_der.as_slice()).unwrap()),
    }
    .to_der()
    .unwrap()
}

fn algorithm(oid: ObjectIdentifier) -> x509_cert::spki::AlgorithmIdentifierOwned {
    x509_cert::spki::AlgorithmIdentifierOwned {
        oid,
        parameters: None,
    }
}

fn ocsp_response(
    issuer: &Certificate,
    certificate: &Certificate,
    issuer_key: &KeyPair,
    status: OcspFixtureStatus,
) -> Vec<u8> {
    let cert_status = match status {
        OcspFixtureStatus::Revoked => CertStatus::revoked(RevokedInfo {
            revocation_time: ocsp_time(2026, 1, 2),
            revocation_reason: None,
        }),
        OcspFixtureStatus::Unknown => CertStatus::unknown(),
        _ => CertStatus::good(),
    };
    let response_data = ResponseData {
        version: Version::V1,
        responder_id: ResponderId::ByName(issuer.tbs_certificate.subject.clone()),
        produced_at: ocsp_time(2027, 1, 10),
        responses: vec![SingleResponse {
            cert_id: CertId {
                hash_algorithm: algorithm(ID_SHA256),
                issuer_name_hash: OctetString::new(
                    Sha256::digest(issuer.tbs_certificate.subject.to_der().unwrap()).as_slice(),
                )
                .unwrap(),
                issuer_key_hash: OctetString::new(
                    Sha256::digest(
                        issuer
                            .tbs_certificate
                            .subject_public_key_info
                            .subject_public_key
                            .raw_bytes(),
                    )
                    .as_slice(),
                )
                .unwrap(),
                serial_number: certificate.tbs_certificate.serial_number.clone(),
            },
            cert_status,
            this_update: ocsp_time(2027, 1, 1),
            next_update: Some(if matches!(status, OcspFixtureStatus::Stale) {
                ocsp_time(2027, 1, 5)
            } else {
                ocsp_time(2027, 12, 31)
            }),
            single_extensions: None,
        }],
        response_extensions: None,
    };
    let mut signature = issuer_key.sign(&response_data.to_der().unwrap()).unwrap();
    if matches!(status, OcspFixtureStatus::Tampered) {
        signature[0] ^= 1;
    }
    OcspResponse::successful(BasicOcspResponse {
        tbs_response_data: response_data,
        signature_algorithm: algorithm(ECDSA_WITH_SHA256),
        signature: BitString::from_bytes(&signature).unwrap(),
        certs: None,
    })
    .unwrap()
    .to_der()
    .unwrap()
}

fn ocsp_time(year: u16, month: u8, day: u8) -> OcspGeneralizedTime {
    DateTime::new(year, month, day, 0, 0, 0).unwrap().into()
}

fn signature_pdf(placeholder_bytes: usize) -> Vec<u8> {
    let placeholder = "9999999999";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 {placeholder} {placeholder} {placeholder}] /Contents <{}> >>",
            "00".repeat(placeholder_bytes)
        ),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let marker = b"/Contents <";
    let contents = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    let gap_start = contents + b"/Contents ".len();
    let gap_end = gap_start
        + bytes[gap_start..]
            .iter()
            .position(|byte| *byte == b'>')
            .unwrap()
        + 1;
    let values = [gap_start, gap_end, bytes.len() - gap_end];
    let mut search = 0;
    for value in values {
        let position = bytes[search..]
            .windows(placeholder.len())
            .position(|window| window == placeholder.as_bytes())
            .unwrap()
            + search;
        bytes[position..position + placeholder.len()]
            .copy_from_slice(format!("{value:010}").as_bytes());
        search = position + placeholder.len();
    }
    bytes
}

fn ranges(pdf: &[u8]) -> [usize; 4] {
    let marker = b"/ByteRange [";
    let start = pdf
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    let end = pdf[start..].iter().position(|byte| *byte == b']').unwrap() + start;
    let values: Vec<_> = std::str::from_utf8(&pdf[start..end])
        .unwrap()
        .split_ascii_whitespace()
        .map(|value| value.parse().unwrap())
        .collect();
    values.try_into().unwrap()
}

fn digest_ranges(pdf: &[u8], ranges: [usize; 4]) -> Vec<u8> {
    let [a, b, c, d] = ranges;
    let mut digest = Sha256::new();
    digest.update(&pdf[a..a + b]);
    digest.update(&pdf[c..c + d]);
    digest.finalize().to_vec()
}

fn open(pdf: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(pdf, OpenOptions::default())
        .unwrap()
}

fn hex(input: &[u8]) -> String {
    input.iter().map(|byte| format!("{byte:02X}")).collect()
}

#[test]
fn validates_cms_digest_signature_and_explicit_trust_independently() {
    let primary = fixture();
    let inspected = inspect_signatures(&open(&primary.pdf)).unwrap().remove(0);
    assert_eq!(
        inspected.cms.parse_status,
        CmsParseStatus::Parsed,
        "{:?}",
        inspected.cms
    );
    assert_eq!(inspected.cms.digest_algorithm.as_deref(), Some("sha256"));
    assert_eq!(inspected.cms.digest_status, DigestMatchStatus::Match);
    assert_eq!(inspected.cms.signature_status, SignatureCryptoStatus::Valid);
    assert!(inspected.cms_verified);
    assert_eq!(inspected.cms.trust_status, TrustStatus::NotRequested);
    assert_eq!(
        inspected.cms.revocation_status,
        RevocationStatus::NotRequested
    );
    assert_eq!(
        inspected.cms.timestamp.status,
        TimestampStatus::ImprintMatch
    );
    assert_eq!(
        inspected.cms.timestamp.generation_time_unix,
        Some(1_800_000_000)
    );
    assert_eq!(
        inspected.cms.timestamp.digest_status,
        DigestMatchStatus::Match
    );
    assert_eq!(
        inspected.cms.timestamp.signature_status,
        SignatureCryptoStatus::Valid
    );
    assert_eq!(
        inspected.cms.timestamp.trust_status,
        TrustStatus::NotRequested
    );
    assert!(
        inspected
            .cms
            .signer_certificate_subject
            .as_deref()
            .unwrap()
            .contains("Binas Test Signer")
    );

    let trusted = inspect_signatures_with_options(
        &open(&primary.pdf),
        &SignatureTrustOptions {
            os_roots_der: vec![primary.root_der.clone()],
            fetched_intermediates_der: vec![primary.intermediate_der.clone()],
            crls_der: primary.crls_der.clone(),
            validation_time_unix: Some(1_800_000_000),
            tsa_roots_der: vec![primary.tsa_root_der.clone()],
            tsa_crls_der: vec![primary.tsa_crl_der.clone()],
            ..SignatureTrustOptions::default()
        },
    )
    .unwrap()
    .remove(0);
    assert_eq!(trusted.cms.trust_status, TrustStatus::Trusted);
    assert_eq!(trusted.cms.revocation_status, RevocationStatus::Good);
    assert_eq!(trusted.cms.timestamp.trust_status, TrustStatus::Trusted);
    assert_eq!(
        trusted.cms.timestamp.revocation_status,
        RevocationStatus::Good
    );

    let timestamp_mismatch = fixture_with_timestamp(false, true, false);
    assert_eq!(
        inspect_signatures(&open(&timestamp_mismatch.pdf))
            .unwrap()
            .remove(0)
            .cms
            .timestamp
            .status,
        TimestampStatus::ImprintMismatch
    );

    let tampered = fixture_with_timestamp(true, false, false);
    let tampered = inspect_signatures(&open(&tampered.pdf)).unwrap().remove(0);
    assert_eq!(tampered.cms.timestamp.status, TimestampStatus::ImprintMatch);
    assert_eq!(
        tampered.cms.timestamp.signature_status,
        SignatureCryptoStatus::Invalid
    );

    let wrong_tsa_root = fixture().tsa_root_der;
    let untrusted_tsa = inspect_signatures_with_options(
        &open(&primary.pdf),
        &SignatureTrustOptions {
            tsa_roots_der: vec![wrong_tsa_root],
            ..SignatureTrustOptions::default()
        },
    )
    .unwrap()
    .remove(0);
    assert_eq!(
        untrusted_tsa.cms.timestamp.signature_status,
        SignatureCryptoStatus::Valid
    );
    assert_eq!(
        untrusted_tsa.cms.timestamp.trust_status,
        TrustStatus::Untrusted
    );

    let revoked = fixture_with_timestamp(true, true, true);
    let revoked_tsa = inspect_signatures_with_options(
        &open(&revoked.pdf),
        &SignatureTrustOptions {
            tsa_roots_der: vec![revoked.tsa_root_der.clone()],
            tsa_crls_der: vec![revoked.tsa_crl_der.clone()],
            ..SignatureTrustOptions::default()
        },
    )
    .unwrap()
    .remove(0);
    assert_eq!(revoked_tsa.cms.timestamp.trust_status, TrustStatus::Trusted);
    assert_eq!(
        revoked_tsa.cms.timestamp.revocation_status,
        RevocationStatus::Revoked
    );

    let wrong_root = fixture().root_der;
    let untrusted = inspect_signatures_with_options(
        &open(&primary.pdf),
        &SignatureTrustOptions {
            roots_der: vec![wrong_root],
            fetched_intermediates_der: vec![primary.intermediate_der.clone()],
            validation_time_unix: Some(1_800_000_000),
            ..SignatureTrustOptions::default()
        },
    )
    .unwrap()
    .remove(0);
    assert_eq!(untrusted.cms.trust_status, TrustStatus::Untrusted);
}

#[test]
fn validates_caller_fetched_ocsp_without_network_or_ambient_trust() {
    let fixture = fixture();
    let inspect = |responses: Vec<Vec<u8>>| {
        inspect_signatures_with_options(
            &open(&fixture.pdf),
            &SignatureTrustOptions {
                os_roots_der: vec![fixture.root_der.clone()],
                fetched_intermediates_der: vec![fixture.intermediate_der.clone()],
                ocsp_responses_der: responses,
                validation_time_unix: Some(1_800_000_000),
                ..SignatureTrustOptions::default()
            },
        )
        .unwrap()
        .remove(0)
        .cms
        .revocation_status
    };

    assert_eq!(
        inspect(fixture.ocsp_good_der.clone()),
        RevocationStatus::Good
    );
    assert_eq!(
        inspect(fixture.ocsp_revoked_der.clone()),
        RevocationStatus::Revoked
    );
    assert_eq!(
        inspect(fixture.ocsp_unknown_der.clone()),
        RevocationStatus::Indeterminate
    );
    assert_eq!(
        inspect(fixture.ocsp_tampered_der.clone()),
        RevocationStatus::Indeterminate
    );
    assert_eq!(
        inspect(fixture.ocsp_stale_der.clone()),
        RevocationStatus::Indeterminate
    );
}

#[test]
fn reports_digest_mismatch_malformed_cms_and_limits_without_false_claims() {
    let fixture = fixture();
    let mut changed = fixture.pdf.clone();
    changed[6] = b'2';
    let mismatch = inspect_signatures(&open(&changed)).unwrap().remove(0);
    assert_eq!(mismatch.cms.digest_status, DigestMatchStatus::Mismatch);
    assert_eq!(mismatch.cms.signature_status, SignatureCryptoStatus::Valid);

    let mut malformed = signature_pdf(64);
    let marker = b"/Contents <";
    let start = malformed
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    malformed[start..start + 2].copy_from_slice(b"01");
    let malformed = inspect_signatures(&open(&malformed)).unwrap().remove(0);
    assert_eq!(malformed.cms.parse_status, CmsParseStatus::Malformed);

    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_stream_bytes: 64,
            ..Limits::default()
        },
    });
    assert_eq!(
        inspect_signatures(&engine.open(&fixture.pdf, OpenOptions::default()).unwrap())
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
}
