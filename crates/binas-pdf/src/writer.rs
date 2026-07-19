use std::{
    collections::BTreeMap,
    fmt::{self, Write},
};

use crate::{
    document::PdfDocument,
    error::PdfError,
    parser::{IndirectObject, ObjectRef, ParsedDocument, Value},
};

pub(crate) fn append_object_revision(
    document: &PdfDocument,
    reference: ObjectRef,
    value: &Value,
    stream: Option<&[u8]>,
) -> Result<Vec<u8>, PdfError> {
    append_object_revisions(document, &[(reference, value, stream)])
}

pub(crate) fn append_object_revisions(
    document: &PdfDocument,
    replacements: &[(ObjectRef, &Value, Option<&[u8]>)],
) -> Result<Vec<u8>, PdfError> {
    if replacements.is_empty() {
        return Err(PdfError::unsafe_rewrite(
            "incremental revision requires at least one replacement object",
        ));
    }
    if replacements.len() > document.parsed().limits.max_container_items
        || replacements.len() > document.parsed().limits.max_objects
    {
        return Err(PdfError::limit(
            "incremental replacement count exceeds limit",
        ));
    }
    refuse_security_boundaries(document.parsed())?;
    let size = dict_integer(&document.parsed().trailer, b"Size")
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| PdfError::unsafe_rewrite("trailer has no direct non-negative /Size"))?;
    let mut sorted = BTreeMap::new();
    let mut generations = BTreeMap::new();
    let mut new_objects = 0usize;
    let mut output_size = size;
    for &(reference, value, stream) in replacements {
        let number = usize::try_from(reference.number)
            .map_err(|_| PdfError::limit("replacement object number exceeds usize"))?;
        let existing = document.parsed().objects.contains_key(&reference);
        if existing && number >= size {
            return Err(PdfError::unsafe_rewrite(
                "existing replacement object is outside trailer /Size",
            ));
        }
        if !existing {
            if number < size || reference.generation != 0 {
                return Err(PdfError::unsafe_rewrite(
                    "new objects require generation zero at or above trailer /Size",
                ));
            }
            if document
                .parsed()
                .objects
                .keys()
                .any(|existing| existing.number == reference.number)
            {
                return Err(PdfError::unsafe_rewrite(
                    "new object number collides with an existing generation",
                ));
            }
            if !sorted.contains_key(&reference) {
                new_objects = new_objects
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("new object count overflows"))?;
            }
            output_size = output_size.max(
                number
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("trailer /Size overflows"))?,
            );
        }
        if generations
            .insert(reference.number, reference.generation)
            .is_some_and(|generation| generation != reference.generation)
        {
            return Err(PdfError::unsafe_rewrite(
                "one revision cannot replace multiple generations of an object number",
            ));
        }
        sorted.insert(reference, (value, stream));
    }
    if document
        .parsed()
        .objects
        .len()
        .checked_add(new_objects)
        .is_none_or(|count| count > document.parsed().limits.max_objects)
        || (new_objects != 0 && output_size > document.parsed().limits.max_xref_entries)
    {
        return Err(PdfError::limit(
            "incremental object allocation exceeds object or xref limits",
        ));
    }
    let previous_xref = previous_xref_offset(document.source())?;
    let mut output = Output::from_bytes(
        document.source(),
        document.engine_config().limits.max_output_bytes,
    )?;
    output.push(b"\n")?;
    let mut offsets = BTreeMap::new();
    for (&reference, &(value, stream)) in &sorted {
        let object_offset = output.len();
        require_classic_offset(object_offset)?;
        output.formatted(format_args!(
            "{} {} obj\n",
            reference.number, reference.generation
        ))?;
        write_object(
            &mut output,
            &IndirectObject {
                value: value.clone(),
                stream: stream.map(<[u8]>::to_vec),
                stream_offset: 0,
                offset: object_offset,
            },
            document.parsed().limits.max_parser_depth,
        )?;
        output.push(b"\nendobj\n")?;
        offsets.insert(reference, object_offset);
    }
    let xref_offset = output.len();
    require_classic_offset(xref_offset)?;
    output.push(b"xref\n")?;
    let entries = offsets.into_iter().collect::<Vec<_>>();
    let mut start = 0usize;
    while start < entries.len() {
        let mut end = start + 1;
        while end < entries.len()
            && entries[end].0.number == entries[end - 1].0.number.saturating_add(1)
        {
            end += 1;
        }
        output.formatted(format_args!(
            "{} {}\n",
            entries[start].0.number,
            end - start
        ))?;
        for (reference, object_offset) in &entries[start..end] {
            output.formatted(format_args!(
                "{object_offset:010} {:05} n \n",
                reference.generation
            ))?;
        }
        start = end;
    }
    output.formatted(format_args!("trailer\n<< /Size {output_size}"))?;
    for key in [b"Root".as_slice(), b"Info".as_slice(), b"ID".as_slice()] {
        if let Some(value) = dict_get(&document.parsed().trailer, key) {
            output.push(b" ")?;
            write_name(&mut output, key)?;
            output.push(b" ")?;
            write_value(
                &mut output,
                value,
                0,
                document.parsed().limits.max_parser_depth,
            )?;
        }
    }
    output.formatted(format_args!(
        " /Prev {previous_xref} >>\nstartxref\n{xref_offset}\n%%EOF\n"
    ))?;
    Ok(output.into_bytes())
}

pub(crate) fn next_object_reference(document: &PdfDocument) -> Result<ObjectRef, PdfError> {
    let size = dict_integer(&document.parsed().trailer, b"Size")
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| PdfError::unsafe_rewrite("trailer has no direct u32 /Size"))?;
    let mut number = size;
    while document
        .parsed()
        .objects
        .keys()
        .any(|reference| reference.number == number)
    {
        number = number
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("new object number overflows u32"))?;
    }
    let output_size = usize::try_from(number)
        .ok()
        .and_then(|number| number.checked_add(1))
        .ok_or_else(|| PdfError::limit("new trailer /Size overflows"))?;
    if output_size > document.parsed().limits.max_xref_entries
        || document.parsed().objects.len() >= document.parsed().limits.max_objects
    {
        return Err(PdfError::limit("new object allocation exceeds limits"));
    }
    Ok(ObjectRef {
        number,
        generation: 0,
    })
}

pub(crate) struct Output {
    bytes: Vec<u8>,
    limit: usize,
}

impl Output {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    fn from_bytes(bytes: &[u8], limit: usize) -> Result<Self, PdfError> {
        if bytes.len() > limit {
            return Err(PdfError::limit("output exceeds max_output_bytes"));
        }
        Ok(Self {
            bytes: bytes.to_vec(),
            limit,
        })
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<(), PdfError> {
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| PdfError::limit("output size overflows"))?;
        if length > self.limit {
            return Err(PdfError::limit("output exceeds max_output_bytes"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn byte(&mut self, byte: u8) -> Result<(), PdfError> {
        self.push(&[byte])
    }

    pub(crate) fn formatted(&mut self, args: fmt::Arguments<'_>) -> Result<(), PdfError> {
        fmt::write(self, args).map_err(|_| PdfError::limit("output exceeds max_output_bytes"))
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for Output {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.push(value.as_bytes()).map_err(|_| fmt::Error)
    }
}

pub(crate) fn write_object(
    output: &mut Output,
    object: &IndirectObject,
    max_depth: usize,
) -> Result<(), PdfError> {
    match object.stream.as_deref() {
        Some(stream) => {
            let Value::Dict(dictionary) = &object.value else {
                return Err(PdfError::unsafe_rewrite(
                    "stream object does not have a dictionary",
                ));
            };
            write_stream_dict(output, dictionary, stream.len(), max_depth)?;
            output.push(b"\nstream\n")?;
            output.push(stream)?;
            output.push(b"\nendstream")
        }
        None => write_value(output, &object.value, 0, max_depth),
    }
}

fn write_stream_dict(
    output: &mut Output,
    dictionary: &BTreeMap<Vec<u8>, Value>,
    stream_len: usize,
    max_depth: usize,
) -> Result<(), PdfError> {
    output.push(b"<<")?;
    let mut length_written = false;
    for (key, value) in dictionary {
        if !length_written && key.as_slice() > b"Length" {
            output.formatted(format_args!(" /Length {stream_len}"))?;
            length_written = true;
        }
        output.push(b" ")?;
        write_name(output, key)?;
        output.push(b" ")?;
        if key == b"Length" {
            output.formatted(format_args!("{stream_len}"))?;
            length_written = true;
        } else {
            write_value(output, value, 1, max_depth)?;
        }
    }
    if !length_written {
        output.formatted(format_args!(" /Length {stream_len}"))?;
    }
    output.push(b" >>")
}

pub(crate) fn write_value(
    output: &mut Output,
    value: &Value,
    depth: usize,
    max_depth: usize,
) -> Result<(), PdfError> {
    if depth > max_depth {
        return Err(PdfError::limit(
            "serialized value depth exceeds max_parser_depth",
        ));
    }
    match value {
        Value::Null => output.push(b"null"),
        Value::Bool(value) => output.push(if *value { b"true" } else { b"false" }),
        Value::Integer(value) => output.formatted(format_args!("{value}")),
        Value::Real(value) => write_real(output, *value),
        Value::Name(value) => write_name(output, value),
        Value::String(value) => write_hex_string(output, value),
        Value::Array(values) => {
            output.push(b"[")?;
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b" ")?;
                }
                write_value(output, value, depth + 1, max_depth)?;
            }
            output.push(b"]")
        }
        Value::Dict(dictionary) => {
            output.push(b"<<")?;
            for (key, value) in dictionary {
                output.push(b" ")?;
                write_name(output, key)?;
                output.push(b" ")?;
                write_value(output, value, depth + 1, max_depth)?;
            }
            output.push(b" >>")
        }
        Value::Ref(reference) => output.formatted(format_args!(
            "{} {} R",
            reference.number, reference.generation
        )),
    }
}

fn write_real(output: &mut Output, value: f64) -> Result<(), PdfError> {
    if !value.is_finite() {
        return Err(PdfError::unsafe_rewrite(
            "output cannot serialize a non-finite real",
        ));
    }
    let value = if value == 0.0 { 0.0 } else { value };
    let encoded = value.to_string();
    output.push(encoded.as_bytes())?;
    if !encoded.contains('.') {
        output.push(b".0")?;
    }
    Ok(())
}

pub(crate) fn write_name(output: &mut Output, value: &[u8]) -> Result<(), PdfError> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    output.byte(b'/')?;
    for &byte in value {
        if (33..=126).contains(&byte) && !b"()<>[]{}/%#".contains(&byte) {
            output.byte(byte)?;
        } else {
            output.push(&[
                b'#',
                HEX[usize::from(byte >> 4)],
                HEX[usize::from(byte & 15)],
            ])?;
        }
    }
    Ok(())
}

fn write_hex_string(output: &mut Output, value: &[u8]) -> Result<(), PdfError> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    output.byte(b'<')?;
    for &byte in value {
        output.push(&[HEX[usize::from(byte >> 4)], HEX[usize::from(byte & 15)]])?;
    }
    output.byte(b'>')
}

pub(crate) fn require_classic_offset(offset: usize) -> Result<(), PdfError> {
    if u64::try_from(offset).map_err(|_| PdfError::limit("offset exceeds u64"))? > 9_999_999_999 {
        return Err(PdfError::unsafe_rewrite(
            "object offset exceeds classic xref width",
        ));
    }
    Ok(())
}

pub(crate) fn dict_get<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    match value {
        Value::Dict(dictionary) => dictionary.get(key),
        _ => None,
    }
}

pub(crate) fn dict_integer(value: &Value, key: &[u8]) -> Option<i64> {
    match dict_get(value, key)? {
        Value::Integer(value) => Some(*value),
        _ => None,
    }
}

pub(crate) fn refuse_security_boundaries(parsed: &ParsedDocument) -> Result<(), PdfError> {
    if dict_get(&parsed.trailer, b"Encrypt").is_some() {
        return Err(PdfError::unsafe_rewrite(
            "rewriting encrypted PDFs is not implemented",
        ));
    }
    if contains_signature(&parsed.trailer)
        || parsed
            .objects
            .values()
            .any(|object| contains_signature(&object.value))
    {
        return Err(PdfError::unsafe_rewrite(
            "rewriting signed PDFs requires an explicit signature policy",
        ));
    }
    Ok(())
}

fn contains_signature(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_signature),
        Value::Dict(dictionary) => {
            dictionary.contains_key(b"ByteRange".as_slice())
                || matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Sig")
                || matches!(dictionary.get(b"FT".as_slice()), Some(Value::Name(name)) if name == b"Sig")
                || dictionary.values().any(contains_signature)
        }
        _ => false,
    }
}

fn previous_xref_offset(input: &[u8]) -> Result<usize, PdfError> {
    let marker = b"startxref";
    let start = input
        .windows(marker.len())
        .rposition(|window| window == marker)
        .ok_or_else(|| PdfError::unsafe_rewrite("missing startxref"))?;
    let mut rest = &input[start + marker.len()..];
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = &rest[1..];
    }
    let digits = rest
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits == 0 {
        return Err(PdfError::unsafe_rewrite("startxref has no numeric offset"));
    }
    std::str::from_utf8(&rest[..digits])
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value < input.len())
        .ok_or_else(|| PdfError::unsafe_rewrite("startxref offset is invalid"))
}

#[cfg(test)]
mod tests {
    use crate::{OpenOptions, PdfEngine, parser::Value};

    use super::{ObjectRef, append_object_revisions};

    fn pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R >>",
            "(four)",
            "(five)",
            "(six)",
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        bytes
    }

    #[test]
    fn batch_revision_sorts_deduplicates_and_groups_xref_subsections() {
        let source = pdf();
        let document = PdfEngine::default()
            .open(&source, OpenOptions::default())
            .unwrap();
        let first = Value::String(b"ignored".to_vec());
        let fourth = Value::String(b"FOUR".to_vec());
        let fifth = Value::String(b"FIVE".to_vec());
        let output = append_object_revisions(
            &document,
            &[
                (
                    ObjectRef {
                        number: 5,
                        generation: 0,
                    },
                    &fifth,
                    None,
                ),
                (
                    ObjectRef {
                        number: 4,
                        generation: 0,
                    },
                    &first,
                    None,
                ),
                (
                    ObjectRef {
                        number: 4,
                        generation: 0,
                    },
                    &fourth,
                    None,
                ),
            ],
        )
        .unwrap();
        assert!(output.starts_with(&source));
        assert!(
            output
                .windows(b"xref\n4 2\n".len())
                .any(|value| value == b"xref\n4 2\n")
        );
        let rewritten = PdfEngine::default()
            .open(&output, OpenOptions::default())
            .unwrap();
        assert_eq!(
            rewritten
                .parsed()
                .object(ObjectRef {
                    number: 4,
                    generation: 0
                })
                .unwrap()
                .value,
            fourth
        );
        assert_eq!(rewritten.parsed().xref_revisions, 2);
    }

    #[test]
    fn batch_revision_emits_disjoint_subsections_and_rejects_empty_input() {
        let source = pdf();
        let document = PdfEngine::default()
            .open(&source, OpenOptions::default())
            .unwrap();
        let fourth = Value::String(b"FOUR".to_vec());
        let sixth = Value::String(b"SIX".to_vec());
        let output = append_object_revisions(
            &document,
            &[
                (
                    ObjectRef {
                        number: 6,
                        generation: 0,
                    },
                    &sixth,
                    None,
                ),
                (
                    ObjectRef {
                        number: 4,
                        generation: 0,
                    },
                    &fourth,
                    None,
                ),
            ],
        )
        .unwrap();
        let appended = &output[source.len()..];
        assert!(
            appended
                .windows(b"xref\n4 1\n".len())
                .any(|value| value == b"xref\n4 1\n")
        );
        assert!(
            appended
                .windows(b"\n6 1\n".len())
                .any(|value| value == b"\n6 1\n")
        );
        assert!(append_object_revisions(&document, &[]).is_err());
    }

    #[test]
    fn batch_revision_allocates_generation_zero_across_number_gaps() {
        let source = pdf();
        let document = PdfEngine::default()
            .open(&source, OpenOptions::default())
            .unwrap();
        let value = Value::String(b"NEW".to_vec());
        let reference = ObjectRef {
            number: 8,
            generation: 0,
        };
        let output = append_object_revisions(&document, &[(reference, &value, None)]).unwrap();
        let rewritten = PdfEngine::default()
            .open(&output, OpenOptions::default())
            .unwrap();
        assert_eq!(rewritten.parsed().object(reference).unwrap().value, value);
        assert_eq!(
            super::dict_integer(&rewritten.parsed().trailer, b"Size"),
            Some(9)
        );

        let collision = ObjectRef {
            number: 6,
            generation: 1,
        };
        assert!(append_object_revisions(&document, &[(collision, &value, None)]).is_err());
    }
}
