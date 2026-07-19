use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    PdfDocument, PdfError,
    cms_validation::validate_cms,
    parser::{ObjectRef, Value},
};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CmsParseStatus {
    #[default]
    NotPresent,
    Parsed,
    Malformed,
    UnsupportedContentType,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestMatchStatus {
    #[default]
    NotPerformed,
    Match,
    Mismatch,
    MissingAttribute,
    UnsupportedAlgorithm,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureCryptoStatus {
    #[default]
    NotPerformed,
    Valid,
    Invalid,
    UnsupportedAlgorithm,
    SignerCertificateMissing,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustStatus {
    #[default]
    NotRequested,
    Trusted,
    Untrusted,
    SignerCertificateMissing,
    ValidationTimeMissing,
    InvalidInput,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationStatus {
    #[default]
    NotRequested,
    Good,
    Revoked,
    Indeterminate,
    InvalidInput,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampStatus {
    #[default]
    NotPresent,
    ImprintMatch,
    ImprintMismatch,
    UnsupportedAlgorithm,
    Malformed,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TimestampInspection {
    pub status: TimestampStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest_algorithm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hashed_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_time_unix: Option<u64>,
    pub digest_status: DigestMatchStatus,
    pub signature_status: SignatureCryptoStatus,
    pub trust_status: TrustStatus,
    pub revocation_status: RevocationStatus,
}

impl TimestampInspection {
    pub(crate) fn malformed(error: impl Into<String>) -> Self {
        Self {
            status: TimestampStatus::Malformed,
            error: Some(error.into()),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CmsValidation {
    pub parse_status: CmsParseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub signer_count: usize,
    pub certificate_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest_algorithm: Option<String>,
    pub digest_status: DigestMatchStatus,
    pub signature_status: SignatureCryptoStatus,
    pub trust_status: TrustStatus,
    pub revocation_status: RevocationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_certificate_subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_certificate_issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer_certificate_serial: Option<String>,
    pub timestamp: TimestampInspection,
}

impl CmsValidation {
    pub(crate) fn malformed(error: impl Into<String>) -> Self {
        Self {
            parse_status: CmsParseStatus::Malformed,
            error: Some(error.into()),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SignatureTrustOptions {
    #[serde(default)]
    pub roots_der: Vec<Vec<u8>>,
    /// Caller-supplied snapshot of OS trust roots. Binas never reads an OS store itself.
    #[serde(default)]
    pub os_roots_der: Vec<Vec<u8>>,
    #[serde(default)]
    pub intermediates_der: Vec<Vec<u8>>,
    /// Caller-fetched AIA intermediates. Binas never performs AIA network requests.
    #[serde(default)]
    pub fetched_intermediates_der: Vec<Vec<u8>>,
    #[serde(default)]
    pub crls_der: Vec<Vec<u8>>,
    /// Caller-fetched OCSP responses. Binas validates them offline.
    #[serde(default)]
    pub ocsp_responses_der: Vec<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_time_unix: Option<u64>,
    #[serde(default)]
    pub tsa_roots_der: Vec<Vec<u8>>,
    #[serde(default)]
    pub tsa_os_roots_der: Vec<Vec<u8>>,
    #[serde(default)]
    pub tsa_intermediates_der: Vec<Vec<u8>>,
    #[serde(default)]
    pub tsa_fetched_intermediates_der: Vec<Vec<u8>>,
    #[serde(default)]
    pub tsa_crls_der: Vec<Vec<u8>>,
    #[serde(default)]
    pub tsa_ocsp_responses_der: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SignatureInspection {
    pub index: usize,
    pub object_number: u32,
    pub object_generation: u16,
    pub byte_range: [u64; 4],
    pub gap_start: u64,
    pub gap_end: u64,
    pub covered_end: u64,
    pub later_bytes: u64,
    pub covers_current_file: bool,
    pub contents_bytes: usize,
    pub signed_bytes_sha256: String,
    pub filter: Option<String>,
    pub sub_filter: Option<String>,
    pub signer_name: Option<String>,
    pub signing_time: Option<String>,
    pub cms_verified: bool,
    pub cms: CmsValidation,
}

pub fn inspect_signatures(document: &PdfDocument) -> Result<Vec<SignatureInspection>, PdfError> {
    inspect_signatures_with_options(document, &SignatureTrustOptions::default())
}

pub fn inspect_signatures_with_options(
    document: &PdfDocument,
    trust: &SignatureTrustOptions,
) -> Result<Vec<SignatureInspection>, PdfError> {
    let mut output = Vec::new();
    for (reference, object) in &document.parsed().objects {
        let Value::Dict(dictionary) = &object.value else {
            continue;
        };
        let is_signature = matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Sig")
            || dictionary.contains_key(b"ByteRange".as_slice());
        if !is_signature {
            continue;
        }
        if output.len() >= document.engine_config().limits.max_container_items {
            return Err(PdfError::limit("signature count exceeds container limit"));
        }
        output.push(inspect_signature(
            document,
            *reference,
            dictionary,
            output.len(),
            trust,
        )?);
    }
    Ok(output)
}

fn inspect_signature(
    document: &PdfDocument,
    reference: ObjectRef,
    dictionary: &std::collections::BTreeMap<Vec<u8>, Value>,
    index: usize,
    trust: &SignatureTrustOptions,
) -> Result<SignatureInspection, PdfError> {
    let byte_range = byte_range(dictionary.get(b"ByteRange".as_slice()), reference)?;
    let [first_start, first_len, second_start, second_len] = byte_range;
    if first_start != 0 {
        return Err(signature_error(
            reference,
            "signature ByteRange must start at byte zero",
        ));
    }
    let first_end = first_start
        .checked_add(first_len)
        .ok_or_else(|| signature_error(reference, "signature ByteRange first range overflows"))?;
    let second_end = second_start
        .checked_add(second_len)
        .ok_or_else(|| signature_error(reference, "signature ByteRange second range overflows"))?;
    let source_len = u64::try_from(document.source().len())
        .map_err(|_| PdfError::limit("source length exceeds u64"))?;
    if first_end > second_start || second_end > source_len {
        return Err(signature_error(
            reference,
            "signature ByteRange overlaps or exceeds the source",
        ));
    }
    let first = checked_slice(document.source(), first_start, first_len, reference)?;
    let second = checked_slice(document.source(), second_start, second_len, reference)?;
    let mut digest = Sha256::new();
    digest.update(first);
    digest.update(second);

    let contents = match dictionary.get(b"Contents".as_slice()) {
        Some(Value::String(value)) => value,
        _ => {
            return Err(signature_error(
                reference,
                "signature /Contents is missing or not a string",
            ));
        }
    };
    let contents_bytes = contents
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |index| index + 1);
    let cms = validate_cms(
        contents,
        &[first, second],
        trust,
        &document.engine_config().limits,
    )?;
    let cms_verified = cms.digest_status == DigestMatchStatus::Match
        && cms.signature_status == SignatureCryptoStatus::Valid
        && cms.signer_count == 1;
    Ok(SignatureInspection {
        index,
        object_number: reference.number,
        object_generation: reference.generation,
        byte_range,
        gap_start: first_end,
        gap_end: second_start,
        covered_end: second_end,
        later_bytes: source_len - second_end,
        covers_current_file: second_end == source_len,
        contents_bytes,
        signed_bytes_sha256: hex(&digest.finalize()),
        filter: optional_name(dictionary, b"Filter", reference)?,
        sub_filter: optional_name(dictionary, b"SubFilter", reference)?,
        signer_name: optional_text(dictionary, b"Name", reference)?,
        signing_time: optional_text(dictionary, b"M", reference)?,
        cms_verified,
        cms,
    })
}

fn byte_range(value: Option<&Value>, reference: ObjectRef) -> Result<[u64; 4], PdfError> {
    let Some(Value::Array(values)) = value else {
        return Err(signature_error(
            reference,
            "signature /ByteRange is missing or not an array",
        ));
    };
    if values.len() != 4 {
        return Err(signature_error(
            reference,
            "signature /ByteRange must contain four integers",
        ));
    }
    let mut output = [0_u64; 4];
    for (index, value) in values.iter().enumerate() {
        let Value::Integer(value) = value else {
            return Err(signature_error(
                reference,
                "signature /ByteRange contains a non-integer",
            ));
        };
        output[index] = u64::try_from(*value).map_err(|_| {
            signature_error(reference, "signature /ByteRange contains a negative value")
        })?;
    }
    Ok(output)
}

fn checked_slice(
    source: &[u8],
    start: u64,
    len: u64,
    reference: ObjectRef,
) -> Result<&[u8], PdfError> {
    let start = usize::try_from(start)
        .map_err(|_| signature_error(reference, "signature range start exceeds usize"))?;
    let end = start
        .checked_add(
            usize::try_from(len)
                .map_err(|_| signature_error(reference, "signature range length exceeds usize"))?,
        )
        .ok_or_else(|| signature_error(reference, "signature range overflows usize"))?;
    source
        .get(start..end)
        .ok_or_else(|| signature_error(reference, "signature range exceeds source"))
}

fn optional_name(
    dictionary: &std::collections::BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    reference: ObjectRef,
) -> Result<Option<String>, PdfError> {
    match dictionary.get(key) {
        None => Ok(None),
        Some(Value::Name(value)) => Ok(Some(String::from_utf8_lossy(value).into_owned())),
        Some(_) => Err(signature_error(
            reference,
            "signature name entry has the wrong type",
        )),
    }
}

fn optional_text(
    dictionary: &std::collections::BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    reference: ObjectRef,
) -> Result<Option<String>, PdfError> {
    match dictionary.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(String::from_utf8_lossy(value).into_owned())),
        Some(_) => Err(signature_error(
            reference,
            "signature text entry has the wrong type",
        )),
    }
}

fn signature_error(reference: ObjectRef, message: impl Into<String>) -> PdfError {
    let mut error = PdfError::syntax(message, 0);
    error.object = Some((reference.number, reference.generation));
    error
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
