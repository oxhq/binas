use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError,
    filters::{DecodeParams, PdfFilter, encode_pdf_stream},
    parser::{ObjectRef, ParseBudget, Value, decode_stream},
};

/// An indirect object that owns a stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamObjectRef {
    pub object_number: u32,
    pub object_generation: u16,
}

/// One filter in a stream's declared decode chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamFilterMetadata {
    pub filter: PdfFilter,
    pub decode_params: Option<DecodeParams>,
}

/// Read-only metadata for one parsed PDF stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamInventoryEntry {
    pub object: StreamObjectRef,
    pub encoded_length: usize,
    pub filter_chain: Vec<StreamFilterMetadata>,
    pub image_xobject: bool,
}

/// Lists parsed stream objects without decoding or copying stream bytes.
pub fn list_streams(document: &PdfDocument) -> Result<Vec<StreamInventoryEntry>, PdfError> {
    let parsed = document.parsed();
    let mut streams = Vec::new();
    for (reference, object) in &parsed.objects {
        let Some(encoded) = object.stream.as_ref() else {
            continue;
        };
        streams.push(StreamInventoryEntry {
            object: StreamObjectRef {
                object_number: reference.number,
                object_generation: reference.generation,
            },
            encoded_length: encoded.len(),
            filter_chain: stream_filter_chain(&object.value, parsed.limits.max_container_items)?,
            image_xobject: is_image_xobject(&object.value),
        });
    }
    Ok(streams)
}

/// Decodes one stream selected by its stable indirect object reference.
///
/// The encoded input and returned bytes are both bounded by the document's
/// configured stream and decoded-byte limits. Unsupported or malformed filter
/// chains fail closed.
pub fn read_decoded_stream(
    document: &PdfDocument,
    object: StreamObjectRef,
) -> Result<Vec<u8>, PdfError> {
    let reference = ObjectRef {
        number: object.object_number,
        generation: object.object_generation,
    };
    let parsed = document.parsed();
    let stream = parsed.objects.get(&reference).ok_or_else(|| {
        stream_error_at(
            PdfError::selection("stream object was not found"),
            reference,
        )
    })?;
    let encoded = stream.stream.as_deref().ok_or_else(|| {
        stream_error_at(
            PdfError::selection("selected object is not a stream"),
            reference,
        )
    })?;
    let mut budget = ParseBudget::default();
    decode_stream(&stream.value, encoded, &parsed.limits, &mut budget)
        .map_err(|error| stream_error_at(error, reference))
}

fn stream_error_at(mut error: PdfError, reference: ObjectRef) -> PdfError {
    error.object = Some((reference.number, reference.generation));
    error
}

pub(crate) fn stream_filter_chain(
    value: &Value,
    max_items: usize,
) -> Result<Vec<StreamFilterMetadata>, PdfError> {
    let filters = stream_filters(value, max_items)?;
    let params = stream_decode_params(value, filters.len(), max_items)?;
    if filters.len() != params.len() {
        return Err(PdfError::syntax(
            "stream /Filter and /DecodeParms arrays must have equal lengths",
            0,
        ));
    }
    Ok(filters
        .into_iter()
        .zip(params)
        .map(|(filter, decode_params)| StreamFilterMetadata {
            filter,
            decode_params,
        })
        .collect())
}

fn stream_filters(value: &Value, max_items: usize) -> Result<Vec<PdfFilter>, PdfError> {
    match dictionary_value(value, b"Filter") {
        None => Ok(Vec::new()),
        Some(Value::Name(name)) => Ok(vec![pdf_filter(name)]),
        Some(Value::Array(values)) => {
            if values.len() > max_items {
                return Err(PdfError::limit("stream filter count exceeds limit"));
            }
            values
                .iter()
                .map(|value| match value {
                    Value::Name(name) => Ok(pdf_filter(name)),
                    _ => Err(PdfError::syntax(
                        "stream /Filter array contains a non-name",
                        0,
                    )),
                })
                .collect()
        }
        Some(_) => Err(PdfError::syntax(
            "stream /Filter must be a name or array",
            0,
        )),
    }
}

fn stream_decode_params(
    value: &Value,
    filter_count: usize,
    max_items: usize,
) -> Result<Vec<Option<DecodeParams>>, PdfError> {
    match dictionary_value(value, b"DecodeParms") {
        None | Some(Value::Null) => Ok(vec![None; filter_count]),
        Some(params @ Value::Dict(_)) => Ok(vec![Some(decode_params(params)?)]),
        Some(Value::Array(values)) => {
            if values.len() > max_items {
                return Err(PdfError::limit("stream DecodeParms count exceeds limit"));
            }
            values
                .iter()
                .map(|value| match value {
                    Value::Null => Ok(None),
                    Value::Dict(_) => decode_params(value).map(Some),
                    _ => Err(PdfError::syntax(
                        "stream /DecodeParms array contains an invalid value",
                        0,
                    )),
                })
                .collect()
        }
        Some(_) => Err(PdfError::syntax(
            "stream /DecodeParms must be a dictionary or array",
            0,
        )),
    }
}

fn decode_params(value: &Value) -> Result<DecodeParams, PdfError> {
    let Value::Dict(values) = value else {
        return Err(PdfError::syntax(
            "stream /DecodeParms must be a dictionary",
            0,
        ));
    };
    Ok(DecodeParams {
        predictor: decode_u8(values.get(b"Predictor".as_slice()), b"Predictor", 1)?,
        colors: decode_usize(values.get(b"Colors".as_slice()), b"Colors", 1)?,
        bits_per_component: decode_u8(
            values.get(b"BitsPerComponent".as_slice()),
            b"BitsPerComponent",
            8,
        )?,
        columns: decode_usize(values.get(b"Columns".as_slice()), b"Columns", 1)?,
        early_change: decode_u8(values.get(b"EarlyChange".as_slice()), b"EarlyChange", 1)?,
    })
}

fn decode_u8(value: Option<&Value>, name: &[u8], default: u8) -> Result<u8, PdfError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let Value::Integer(value) = value else {
        return Err(PdfError::syntax(
            format!(
                "stream /DecodeParms /{} must be an integer",
                String::from_utf8_lossy(name)
            ),
            0,
        ));
    };
    u8::try_from(*value).map_err(|_| {
        PdfError::syntax(
            format!(
                "stream /DecodeParms /{} exceeds u8",
                String::from_utf8_lossy(name)
            ),
            0,
        )
    })
}

fn decode_usize(value: Option<&Value>, name: &[u8], default: usize) -> Result<usize, PdfError> {
    let Some(value) = value else {
        return Ok(default);
    };
    let Value::Integer(value) = value else {
        return Err(PdfError::syntax(
            format!(
                "stream /DecodeParms /{} must be an integer",
                String::from_utf8_lossy(name)
            ),
            0,
        ));
    };
    usize::try_from(*value).map_err(|_| {
        PdfError::syntax(
            format!(
                "stream /DecodeParms /{} must be non-negative",
                String::from_utf8_lossy(name)
            ),
            0,
        )
    })
}

fn dictionary_value<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    let Value::Dict(values) = value else {
        return None;
    };
    values.get(key)
}

fn pdf_filter(name: &[u8]) -> PdfFilter {
    match name {
        b"FlateDecode" => PdfFilter::FlateDecode,
        b"ASCIIHexDecode" => PdfFilter::ASCIIHexDecode,
        b"ASCII85Decode" => PdfFilter::ASCII85Decode,
        b"RunLengthDecode" => PdfFilter::RunLengthDecode,
        b"LZWDecode" => PdfFilter::LzwDecode,
        name => PdfFilter::Unsupported(String::from_utf8_lossy(name).into_owned()),
    }
}

pub(crate) fn is_image_xobject(value: &Value) -> bool {
    matches!(
        value,
        Value::Dict(values)
            if matches!(values.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"XObject")
                && matches!(values.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Image")
    )
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamMutationRequest {
    pub object_number: u32,
    #[serde(default)]
    pub object_generation: u16,
    pub decoded_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamMutationReport {
    pub operation: String,
    pub object_number: u32,
    pub object_generation: u16,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub decoded_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StreamMutationVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub decoded_stream_matches: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamMutationOutcome {
    pub bytes: Vec<u8>,
    pub report: StreamMutationReport,
    pub verification: StreamMutationVerification,
}

impl PdfDocument {
    pub fn mutate_stream(
        &self,
        request: StreamMutationRequest,
    ) -> Result<StreamMutationOutcome, PdfError> {
        if request.decoded_bytes.len() > self.parsed().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "replacement stream exceeds max_stream_bytes",
            ));
        }
        let reference = ObjectRef {
            number: request.object_number,
            generation: request.object_generation,
        };
        let old_pages = self.page_count()?;
        let mut parsed = self.parsed().clone();
        let object = parsed
            .objects
            .get_mut(&reference)
            .ok_or_else(|| PdfError::selection("stream object was not found"))?;
        if object.stream.is_none() {
            return Err(PdfError::selection("selected object is not a stream"));
        }
        if is_type(&object.value, b"XRef") || is_type(&object.value, b"ObjStm") {
            return Err(PdfError::unsafe_rewrite(
                "xref and object streams cannot be mutated directly",
            ));
        }
        let encoded = encode_pdf_stream(&object.value, &request.decoded_bytes, &parsed.limits)?;
        let Value::Dict(dictionary) = &mut object.value else {
            return Err(PdfError::syntax("stream value must be a dictionary", 0));
        };
        if matches!(dictionary.get(b"Length".as_slice()), Some(Value::Ref(_))) {
            return Err(PdfError::unsupported(
                "stream mutation does not support indirect /Length",
            ));
        }
        dictionary.insert(
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(encoded.len())
                    .map_err(|_| PdfError::limit("encoded stream length exceeds i64"))?,
            ),
        );
        object.stream = Some(encoded);
        let canonical = self.with_parsed(parsed).canonicalize()?;
        let reopened = PdfEngine::new(self.engine_config().clone())
            .open(&canonical.bytes, OpenOptions::default())?;
        let object = reopened.parsed().object(reference)?;
        let mut budget = ParseBudget::default();
        let decoded = decode_stream(
            &object.value,
            object
                .stream
                .as_deref()
                .ok_or_else(|| PdfError::verification("mutated object is no longer a stream"))?,
            &reopened.parsed().limits,
            &mut budget,
        )?;
        let page_count_unchanged = reopened.page_count()? == old_pages;
        let decoded_stream_matches = decoded == request.decoded_bytes;
        let no_dangling_references = verify_references(reopened.parsed())?;
        let verification = StreamMutationVerification {
            passed: page_count_unchanged && decoded_stream_matches && no_dangling_references,
            reparsed: true,
            page_count_unchanged,
            decoded_stream_matches,
            no_dangling_references,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "stream mutation verification failed",
            ));
        }
        Ok(StreamMutationOutcome {
            report: StreamMutationReport {
                operation: "mutate_stream".into(),
                object_number: reference.number,
                object_generation: reference.generation,
                input_bytes: self.source_len(),
                output_bytes: canonical.bytes.len(),
                decoded_bytes: request.decoded_bytes.len(),
            },
            bytes: canonical.bytes,
            verification,
        })
    }
}

fn is_type(value: &Value, expected: &[u8]) -> bool {
    matches!(value, Value::Dict(values) if matches!(values.get(b"Type".as_slice()), Some(Value::Name(name)) if name == expected))
}

fn verify_references(parsed: &crate::parser::ParsedDocument) -> Result<bool, PdfError> {
    fn walk(
        value: &Value,
        parsed: &crate::parser::ParsedDocument,
        depth: usize,
    ) -> Result<bool, PdfError> {
        if depth > parsed.limits.max_parser_depth {
            return Err(PdfError::limit(
                "reference verification depth exceeds limit",
            ));
        }
        match value {
            Value::Ref(reference) => Ok(parsed.objects.contains_key(reference)),
            Value::Array(values) => values.iter().try_fold(true, |valid, value| {
                Ok(valid && walk(value, parsed, depth + 1)?)
            }),
            Value::Dict(values) => values.values().try_fold(true, |valid, value| {
                Ok(valid && walk(value, parsed, depth + 1)?)
            }),
            _ => Ok(true),
        }
    }
    if !walk(&parsed.trailer, parsed, 0)? {
        return Ok(false);
    }
    parsed.objects.values().try_fold(true, |valid, object| {
        Ok(valid && walk(&object.value, parsed, 0)?)
    })
}
