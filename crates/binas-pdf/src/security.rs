use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    PdfDocument, PdfError,
    parser::{ObjectRef, ParsedDocument, Value},
};

type ResolvedDictionary<'a> = (&'a BTreeMap<Vec<u8>, Value>, Option<ObjectRef>);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EncryptionMetadata {
    pub encrypted: bool,
    pub object_number: Option<u32>,
    pub object_generation: Option<u16>,
    pub filter: Option<String>,
    pub sub_filter: Option<String>,
    pub version: Option<i64>,
    pub revision: Option<i64>,
    pub key_length_bits: Option<i64>,
    pub permissions: Option<i64>,
    pub encrypt_metadata: Option<bool>,
    pub stream_filter: Option<String>,
    pub string_filter: Option<String>,
}

pub fn inspect_encryption(document: &PdfDocument) -> Result<EncryptionMetadata, PdfError> {
    let trailer = dictionary(&document.parsed().trailer, "trailer")?;
    let Some(encrypt) = trailer.get(b"Encrypt".as_slice()) else {
        return Ok(EncryptionMetadata {
            encrypted: false,
            object_number: None,
            object_generation: None,
            filter: None,
            sub_filter: None,
            version: None,
            revision: None,
            key_length_bits: None,
            permissions: None,
            encrypt_metadata: None,
            stream_filter: None,
            string_filter: None,
        });
    };
    let (encrypt, reference) = resolve_dict(document.parsed(), encrypt, "trailer /Encrypt")?;
    Ok(EncryptionMetadata {
        encrypted: true,
        object_number: reference.map(|value| value.number),
        object_generation: reference.map(|value| value.generation),
        filter: optional_name(encrypt, b"Filter", reference)?,
        sub_filter: optional_name(encrypt, b"SubFilter", reference)?,
        version: optional_integer(encrypt, b"V", reference)?,
        revision: optional_integer(encrypt, b"R", reference)?,
        key_length_bits: optional_integer(encrypt, b"Length", reference)?,
        permissions: optional_integer(encrypt, b"P", reference)?,
        encrypt_metadata: optional_bool(encrypt, b"EncryptMetadata", reference)?,
        stream_filter: optional_name(encrypt, b"StmF", reference)?,
        string_filter: optional_name(encrypt, b"StrF", reference)?,
    })
}

fn resolve_dict<'a>(
    parsed: &'a ParsedDocument,
    mut value: &'a Value,
    label: &str,
) -> Result<ResolvedDictionary<'a>, PdfError> {
    let mut seen = BTreeSet::new();
    let mut reference = None;
    while let Value::Ref(next) = value {
        if seen.len() >= parsed.limits.max_parser_depth {
            return Err(PdfError::limit(format!(
                "{label} reference depth exceeds limit"
            )));
        }
        if !seen.insert(*next) {
            return Err(PdfError::syntax(format!("cycle resolving {label}"), 0));
        }
        value = &parsed.object(*next)?.value;
        reference = Some(*next);
    }
    match value {
        Value::Dict(value) => Ok((value, reference)),
        _ => Err(with_object(
            PdfError::syntax(format!("{label} is not a dictionary"), 0),
            reference,
        )),
    }
}

fn dictionary<'a>(value: &'a Value, label: &str) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(value) => Ok(value),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn optional_name(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    reference: Option<ObjectRef>,
) -> Result<Option<String>, PdfError> {
    match dictionary.get(key) {
        None => Ok(None),
        Some(Value::Name(value)) => Ok(Some(pdf_name(value))),
        Some(_) => Err(with_object(
            PdfError::syntax(format!("encryption /{} is not a name", pdf_name(key)), 0),
            reference,
        )),
    }
}

fn optional_integer(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    reference: Option<ObjectRef>,
) -> Result<Option<i64>, PdfError> {
    match dictionary.get(key) {
        None => Ok(None),
        Some(Value::Integer(value)) => Ok(Some(*value)),
        Some(_) => Err(with_object(
            PdfError::syntax(
                format!("encryption /{} is not an integer", pdf_name(key)),
                0,
            ),
            reference,
        )),
    }
}

fn optional_bool(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    reference: Option<ObjectRef>,
) -> Result<Option<bool>, PdfError> {
    match dictionary.get(key) {
        None => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(with_object(
            PdfError::syntax(format!("encryption /{} is not a Boolean", pdf_name(key)), 0),
            reference,
        )),
    }
}

fn with_object(mut error: PdfError, reference: Option<ObjectRef>) -> PdfError {
    error.object = reference.map(|value| (value.number, value.generation));
    error
}

fn pdf_name(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::new();
    for &byte in value {
        if (33..=126).contains(&byte) && !b"()<>[]{}/%#".contains(&byte) {
            output.push(char::from(byte));
        } else {
            output.push('#');
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    output
}
