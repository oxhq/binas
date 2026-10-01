use binas_core::OperationContext;
use std::{
    collections::BTreeMap,
    fmt::{self, Write},
};

mod revision;
pub use revision::{PdfRevisionExport, PdfWriterMode};
pub(crate) use revision::{export_staged_document, write_compact_document};

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
    append_object_revisions_with_context(document, replacements, None)
}

pub(crate) fn append_object_revisions_with_context(
    document: &PdfDocument,
    replacements: &[(ObjectRef, &Value, Option<&[u8]>)],
    context: Option<&OperationContext>,
) -> Result<Vec<u8>, PdfError> {
    if let Some(context) = context {
        context.checkpoint().map_err(PdfError::from_context)?;
    }
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
    refuse_security_boundaries_with_context(document.parsed(), context)?;
    refuse_stale_linearization_with_context(document.parsed(), context)?;
    let size = document
        .parsed()
        .source_size
        .ok_or_else(|| PdfError::unsafe_rewrite("trailer has no direct non-negative /Size"))?;
    let mut sorted = BTreeMap::new();
    let mut generations = BTreeMap::new();
    let mut new_objects = 0usize;
    let mut output_size = size;
    for &(reference, value, stream) in replacements {
        if let Some(context) = context {
            context.charge_work(1).map_err(PdfError::from_context)?;
        }
        let number = usize::try_from(reference.number)
            .map_err(|_| PdfError::limit("replacement object number exceeds usize"))?;
        let existing = document.parsed().objects.contains_base_key(&reference);
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
            for existing in document
                .parsed()
                .objects
                .keys()
                .chain(document.parsed().objects.base_keys())
            {
                if let Some(context) = context {
                    context.charge_work(1).map_err(PdfError::from_context)?;
                }
                if existing.number == reference.number && *existing != reference {
                    return Err(PdfError::unsafe_rewrite(
                        "new object number collides with an existing generation",
                    ));
                }
            }
            if !document.parsed().objects.contains_key(&reference)
                && !sorted.contains_key(&reference)
            {
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
        if sorted.insert(reference, (value, stream)).is_some() {
            return Err(PdfError::unsafe_rewrite(
                "one revision cannot replace the same object twice",
            ));
        }
    }
    if document
        .parsed()
        .objects
        .len()
        .checked_add(new_objects)
        .is_none_or(|count| count > document.parsed().limits.max_objects)
        || output_size > document.parsed().limits.max_xref_entries
    {
        return Err(PdfError::limit(
            "incremental object allocation exceeds object or xref limits",
        ));
    }
    let previous_xref = previous_xref_offset(document.source())?;
    let mut output = Output::from_bytes_with_context(
        document.source(),
        document.engine_config().limits.max_output_bytes,
        context,
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
        write_object_parts(
            &mut output,
            value,
            stream,
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
    write_classic_trailer(
        &mut output,
        &document.parsed().trailer,
        output_size,
        Some(previous_xref),
        document.parsed().limits.max_parser_depth,
    )?;
    output.formatted(format_args!("\nstartxref\n{xref_offset}\n%%EOF\n"))?;
    Ok(output.into_bytes())
}

/// Excludes only fields describing the old xref representation. Remaining
/// trailer values (including opaque extensions) survive without interpretation.
pub(crate) fn is_xref_layout_key(trailer: &Value, key: &[u8]) -> bool {
    matches!(key, b"Size" | b"Prev" | b"XRefStm")
        || (matches!(dict_get(trailer, b"Type"), Some(Value::Name(kind)) if kind == b"XRef")
            && matches!(
                key,
                b"Type"
                    | b"W"
                    | b"Index"
                    | b"Length"
                    | b"Filter"
                    | b"DecodeParms"
                    | b"F"
                    | b"FFilter"
                    | b"FDecodeParms"
                    | b"DL"
            ))
}

pub(crate) fn refuse_stale_linearization(parsed: &ParsedDocument) -> Result<(), PdfError> {
    scan_linearization(parsed, None)
}

pub(crate) fn refuse_stale_linearization_with_context(
    parsed: &ParsedDocument,
    context: Option<&OperationContext>,
) -> Result<(), PdfError> {
    match context {
        Some(_) => scan_linearization(parsed, context),
        None => refuse_stale_linearization(parsed),
    }
}

fn charge_scan(context: Option<&OperationContext>) -> Result<(), PdfError> {
    if let Some(context) = context {
        context.charge_work(1).map_err(PdfError::from_context)?;
    }
    Ok(())
}

fn scan_linearization(
    parsed: &ParsedDocument,
    context: Option<&OperationContext>,
) -> Result<(), PdfError> {
    charge_scan(context)?;
    for object in parsed.objects.values() {
        charge_scan(context)?;
        if matches!(dict_get(&object.value, b"Linearized"), Some(value) if !matches!(value, Value::Null))
        {
            return Err(PdfError::unsupported(
                "writer cannot rebuild existing /Linearized byte offsets and hint tables",
            ));
        }
    }
    Ok(())
}

pub(crate) fn write_classic_trailer(
    output: &mut Output,
    trailer: &Value,
    size: usize,
    previous: Option<usize>,
    max_depth: usize,
) -> Result<(), PdfError> {
    let Value::Dict(values) = trailer else {
        return Err(PdfError::unsafe_rewrite("trailer is not a dictionary"));
    };
    output.formatted(format_args!("trailer\n<< /Size {size}"))?;
    for (key, value) in values {
        if is_xref_layout_key(trailer, key) {
            continue;
        }
        output.push(b" ")?;
        write_name(output, key)?;
        output.push(b" ")?;
        write_value(output, value, 0, max_depth)?;
    }
    if let Some(previous) = previous {
        output.formatted(format_args!(" /Prev {previous}"))?;
    }
    output.push(b" >>")
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
    context: Option<OperationContext>,
    pending_error: Option<PdfError>,
}

impl Output {
    pub(crate) fn new(limit: usize) -> Self {
        Self::new_with_context(limit, None)
    }

    /// Context work is charged before each byte append/allocation. This bounds
    /// logical output length, not allocator/process memory or encoder scratch.
    pub(crate) fn new_with_context(limit: usize, context: Option<&OperationContext>) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            context: context.cloned(),
            pending_error: None,
        }
    }

    pub(crate) fn from_bytes_with_context(
        bytes: &[u8],
        limit: usize,
        context: Option<&OperationContext>,
    ) -> Result<Self, PdfError> {
        let mut output = Self::new_with_context(limit, context);
        output.push(bytes)?;
        Ok(output)
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<(), PdfError> {
        if let Some(context) = &self.context {
            context.checkpoint().map_err(PdfError::from_context)?;
        }
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or_else(|| PdfError::limit("output size overflows"))?;
        if length > self.limit {
            return Err(PdfError::limit("output exceeds max_output_bytes"));
        }
        if let Some(context) = &self.context {
            context
                .charge_work(bytes.len() as u64)
                .map_err(PdfError::from_context)?;
        }
        self.bytes
            .try_reserve(bytes.len())
            .map_err(|_| PdfError::limit("output allocation failed"))?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn byte(&mut self, byte: u8) -> Result<(), PdfError> {
        self.push(&[byte])
    }

    pub(crate) fn formatted(&mut self, args: fmt::Arguments<'_>) -> Result<(), PdfError> {
        if let Some(error) = self.pending_error.take() {
            return Err(error);
        }
        if let Some(context) = &self.context {
            context.checkpoint().map_err(PdfError::from_context)?;
        }
        match fmt::write(self, args) {
            Ok(()) => Ok(()),
            Err(_) => Err(self
                .pending_error
                .take()
                .unwrap_or_else(|| PdfError::unsafe_rewrite("output formatting failed"))),
        }
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for Output {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.pending_error.is_some() {
            return Err(fmt::Error);
        }
        self.push(value.as_bytes()).map_err(|error| {
            self.pending_error = Some(error);
            fmt::Error
        })
    }
}

pub(crate) fn write_object(
    output: &mut Output,
    object: &IndirectObject,
    max_depth: usize,
) -> Result<(), PdfError> {
    write_object_parts(output, &object.value, object.stream.as_deref(), max_depth)
}

fn write_object_parts(
    output: &mut Output,
    value: &Value,
    stream: Option<&[u8]>,
    max_depth: usize,
) -> Result<(), PdfError> {
    match stream {
        Some(stream) => {
            let Value::Dict(dictionary) = value else {
                return Err(PdfError::unsafe_rewrite(
                    "stream object does not have a dictionary",
                ));
            };
            write_stream_dict(output, dictionary, stream.len(), max_depth)?;
            output.push(b"\nstream\n")?;
            output.push(stream)?;
            output.push(b"\nendstream")
        }
        None => write_value(output, value, 0, max_depth),
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
    scan_security(parsed, None)
}

/// Bounds the security census with the caller's cumulative work and cancellation.
/// Each visited object/value costs one unit; encoded stream bytes are not scanned.
pub(crate) fn refuse_security_boundaries_with_context(
    parsed: &ParsedDocument,
    context: Option<&OperationContext>,
) -> Result<(), PdfError> {
    match context {
        Some(_) => scan_security(parsed, context),
        None => refuse_security_boundaries(parsed),
    }
}

fn scan_security(
    parsed: &ParsedDocument,
    context: Option<&OperationContext>,
) -> Result<(), PdfError> {
    charge_scan(context)?;
    if dict_get(&parsed.trailer, b"Encrypt").is_some() {
        return Err(PdfError::unsafe_rewrite(
            "rewriting encrypted PDFs is not implemented",
        ));
    }
    if contains_signature(&parsed.trailer, context)? {
        return Err(PdfError::unsafe_rewrite(
            "rewriting signed PDFs requires an explicit signature policy",
        ));
    }
    for object in parsed.objects.values() {
        charge_scan(context)?;
        if contains_signature(&object.value, context)? {
            return Err(PdfError::unsafe_rewrite(
                "rewriting signed PDFs requires an explicit signature policy",
            ));
        }
    }
    Ok(())
}

fn contains_signature(value: &Value, context: Option<&OperationContext>) -> Result<bool, PdfError> {
    charge_scan(context)?;
    match value {
        Value::Array(values) => {
            for value in values {
                if contains_signature(value, context)? {
                    return Ok(true);
                }
            }
        }
        Value::Dict(dictionary) => {
            if dictionary.contains_key(b"ByteRange".as_slice())
                || matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Sig")
                || matches!(dictionary.get(b"FT".as_slice()), Some(Value::Name(name)) if name == b"Sig")
            {
                return Ok(true);
            }
            for value in dictionary.values() {
                if contains_signature(value, context)? {
                    return Ok(true);
                }
            }
        }
        _ => {}
    }
    Ok(false)
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
    #[test]
    fn preflight_scans_preserve_typed_limits_cancellation_and_source() {
        let document = crate::PdfEngine::default()
            .open(
                include_bytes!("../tests/fixtures/preservation/metadata.pdf"),
                crate::OpenOptions::default(),
            )
            .unwrap();
        let original = document.input_source().clone();
        for scan in [
            super::refuse_security_boundaries_with_context,
            super::refuse_stale_linearization_with_context,
        ] {
            let limited = output_context(2);
            assert_eq!(
                scan(document.parsed(), Some(&limited)).unwrap_err().code,
                crate::PdfErrorCode::ResourceLimit
            );
            assert_eq!(limited.budget().snapshot().work_units, 2);
            let cancelled = output_context(1000);
            cancelled.cancellation().cancel();
            assert_eq!(
                scan(document.parsed(), Some(&cancelled)).unwrap_err().code,
                crate::PdfErrorCode::Cancelled
            );
            assert_eq!(cancelled.budget().snapshot().work_units, 0);
            scan(document.parsed(), Some(&output_context(1000))).unwrap();
        }
        assert_eq!(document.input_source().as_bytes(), original.as_bytes());
        assert_eq!(document.source_id(), original.id());
    }

    #[test]
    fn security_scan_charges_nested_values_before_visiting_them() {
        let nested = super::Value::Array(vec![super::Value::Array(vec![super::Value::Null; 20])]);
        let limited = output_context(5);
        assert_eq!(
            super::contains_signature(&nested, Some(&limited))
                .unwrap_err()
                .code,
            crate::PdfErrorCode::ResourceLimit
        );
        assert_eq!(limited.budget().snapshot().work_units, 5);
        let complete = output_context(22);
        assert!(!super::contains_signature(&nested, Some(&complete)).unwrap());
        assert_eq!(complete.budget().snapshot().work_units, 22);
    }

    fn output_context(work: u64) -> binas_core::OperationContext {
        binas_core::OperationContext::new(
            binas_core::ResourceBudget::new(binas_core::BudgetLimits {
                max_live_bytes: 0,
                max_work_units: work,
            }),
            binas_core::CancellationToken::new(),
        )
    }

    #[test]
    fn output_work_rejection_precedes_growth_and_drop_does_not_refund() {
        let context = output_context(3);
        let mut output = super::Output::new_with_context(100, Some(&context));
        assert_eq!(
            output.push(b"four").unwrap_err().code,
            crate::PdfErrorCode::ResourceLimit
        );
        assert_eq!((output.len(), output.bytes.capacity()), (0, 0));
        output.push(b"abc").unwrap();
        let capacity = output.bytes.capacity();
        assert_eq!(
            output.push(b"d").unwrap_err().code,
            crate::PdfErrorCode::ResourceLimit
        );
        assert_eq!((output.len(), output.bytes.capacity()), (3, capacity));
        drop(output);
        assert_eq!(context.budget().snapshot().work_units, 3);
        assert_eq!(context.budget().snapshot().live_bytes, 0);
    }

    #[test]
    fn formatted_output_preserves_cancelled_and_budget_error_codes() {
        let context = output_context(100);
        let mut output = super::Output::new_with_context(100, Some(&context));
        context.cancellation().cancel();
        assert_eq!(
            output.formatted(format_args!("{}", 42)).unwrap_err().code,
            crate::PdfErrorCode::Cancelled
        );
        assert_eq!(output.len(), 0);
        // Exercise cancellation raised inside Display, after formatted's initial
        // checkpoint, to verify fmt::Error does not erase the typed cause.
        struct CancelDuringDisplay(binas_core::CancellationToken);
        impl std::fmt::Display for CancelDuringDisplay {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.cancel();
                formatter.write_str("cancelled")
            }
        }
        let active = output_context(100);
        let mut output = super::Output::new_with_context(100, Some(&active));
        assert_eq!(
            output
                .formatted(format_args!(
                    "{}",
                    CancelDuringDisplay(active.cancellation().clone())
                ))
                .unwrap_err()
                .code,
            crate::PdfErrorCode::Cancelled
        );
        assert_eq!(active.budget().snapshot().work_units, 0);
        let limited = output_context(0);
        let mut output = super::Output::new_with_context(100, Some(&limited));
        assert_eq!(
            output.formatted(format_args!("{}", 42)).unwrap_err().code,
            crate::PdfErrorCode::ResourceLimit
        );
        assert_eq!(output.len(), 0);
    }

    #[test]
    fn output_length_cap_is_checked_before_copy_and_work_charge() {
        let context = output_context(100);
        let error = super::Output::from_bytes_with_context(b"four", 3, Some(&context))
            .err()
            .unwrap();
        assert_eq!(error.code, crate::PdfErrorCode::ResourceLimit);
        assert_eq!(context.budget().snapshot().work_units, 0);
    }

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
    fn batch_revision_rejects_conflicts_and_groups_sorted_xref_subsections() {
        let source = pdf();
        let document = PdfEngine::default()
            .open(&source, OpenOptions::default())
            .unwrap();
        let first = Value::String(b"ignored".to_vec());
        let fourth = Value::String(b"FOUR".to_vec());
        let fifth = Value::String(b"FIVE".to_vec());
        assert!(
            append_object_revisions(
                &document,
                &[
                    (
                        ObjectRef {
                            number: 4,
                            generation: 0
                        },
                        &first,
                        None
                    ),
                    (
                        ObjectRef {
                            number: 4,
                            generation: 0
                        },
                        &fourth,
                        None
                    ),
                ]
            )
            .is_err()
        );
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
