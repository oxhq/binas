use std::collections::{BTreeMap, BTreeSet};

use crate::{
    error::PdfError,
    filters::{DecodeParams, PdfFilter, decode_filter_chain},
    limits::Limits,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ObjectRef {
    pub number: u32,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug)]
enum XrefEntry {
    Free,
    InUse { offset: usize, generation: u16 },
    Compressed { stream: u32, index: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Value>),
    Dict(BTreeMap<Vec<u8>, Value>),
    Ref(ObjectRef),
}

#[derive(Clone, Debug)]
pub(crate) struct IndirectObject {
    pub value: Value,
    pub stream: Option<Vec<u8>>,
    pub stream_offset: usize,
    pub offset: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct ParsedDocument {
    pub version: String,
    pub objects: BTreeMap<ObjectRef, IndirectObject>,
    pub trailer: Value,
    pub xref_revisions: usize,
    pub limits: Limits,
    xref_entries: BTreeMap<u32, XrefEntry>,
    decoded_stream_bytes: usize,
    compressed_finished: bool,
}

impl ParsedDocument {
    pub fn object(&self, reference: ObjectRef) -> Result<&IndirectObject, PdfError> {
        self.objects.get(&reference).ok_or_else(|| {
            PdfError::syntax(
                format!(
                    "missing object {} {} R",
                    reference.number, reference.generation
                ),
                0,
            )
        })
    }
}

pub(crate) fn parse_document(input: &[u8], limits: &Limits) -> Result<ParsedDocument, PdfError> {
    let mut parsed = parse_document_skeleton(input, limits)?;
    if dict_get(&parsed.trailer, b"Encrypt").is_some()
        && parsed
            .xref_entries
            .values()
            .any(|entry| matches!(entry, XrefEntry::Compressed { .. }))
    {
        return Err(PdfError::unsupported(
            "encrypted object streams require authenticated parsing",
        ));
    }
    finish_compressed_objects(&mut parsed)?;
    Ok(parsed)
}

pub(crate) fn parse_document_repair(
    input: &[u8],
    limits: &Limits,
) -> Result<ParsedDocument, PdfError> {
    if input.len() > limits.max_input_bytes {
        return Err(PdfError::limit("input exceeds max_input_bytes"));
    }
    let version = pdf_version(input)?;
    let trailer_offset = rfind(input, b"trailer")
        .ok_or_else(|| PdfError::syntax("repair could not find a trailer", input.len()))?;
    let mut trailer_cursor = Cursor::at(input, trailer_offset + b"trailer".len(), limits);
    let trailer = trailer_cursor.value(0)?;
    if !matches!(trailer, Value::Dict(_)) {
        return Err(PdfError::syntax(
            "repair trailer is not a dictionary",
            trailer_offset,
        ));
    }
    if dict_get(&trailer, b"Encrypt").is_some() {
        return Err(PdfError::unsupported(
            "repair mode does not support encrypted PDFs",
        ));
    }

    let mut objects = BTreeMap::new();
    let mut entries = BTreeMap::new();
    let mut offset = 0usize;
    let mut candidates = 0usize;
    while offset < trailer_offset {
        let boundary = offset == 0 || is_delimiter(input[offset - 1]);
        if boundary && input[offset].is_ascii_digit() {
            candidates = candidates
                .checked_add(1)
                .ok_or_else(|| PdfError::limit("repair candidate count overflows"))?;
            if candidates > limits.max_xref_entries || candidates > limits.max_container_items {
                return Err(PdfError::limit("repair candidate count exceeds limit"));
            }
            if let Ok((reference, object, end)) =
                parse_indirect_with_end(input, offset, limits, true, None)
            {
                entries.insert(
                    reference.number,
                    XrefEntry::InUse {
                        offset,
                        generation: reference.generation,
                    },
                );
                objects.insert(reference, object);
                if objects.len() > limits.max_objects {
                    return Err(PdfError::limit("repair object count exceeds limit"));
                }
                offset = end;
                continue;
            }
        }
        offset += 1;
    }
    if objects.is_empty() {
        return Err(PdfError::syntax(
            "repair could not recover any indirect objects",
            0,
        ));
    }
    Ok(ParsedDocument {
        version,
        objects,
        trailer,
        xref_revisions: 1,
        limits: limits.clone(),
        xref_entries: entries,
        decoded_stream_bytes: 0,
        compressed_finished: true,
    })
}

pub(crate) fn parse_document_skeleton(
    input: &[u8],
    limits: &Limits,
) -> Result<ParsedDocument, PdfError> {
    if input.len() > limits.max_input_bytes {
        return Err(PdfError::limit("input exceeds max_input_bytes"));
    }
    let header = input
        .windows(5)
        .take(1024)
        .position(|value| value == b"%PDF-")
        .ok_or_else(|| PdfError::syntax("missing PDF header", 0))?;
    let version_end = input[header + 5..]
        .iter()
        .position(|byte| byte.is_ascii_whitespace())
        .map(|position| header + 5 + position)
        .unwrap_or(input.len());
    let version = std::str::from_utf8(&input[header + 5..version_end])
        .map_err(|_| PdfError::syntax("PDF version is not ASCII", header + 5))?
        .to_owned();

    let startxref = rfind(input, b"startxref")
        .ok_or_else(|| PdfError::syntax("missing startxref", input.len()))?;
    let mut cursor = Cursor::at(input, startxref + b"startxref".len(), limits);
    let mut xref_offset = cursor.unsigned()?;
    let mut entries = BTreeMap::new();
    let mut trailer = None;
    let mut revisions = 0usize;
    let mut total_entries = 0usize;
    let mut xref_offsets = BTreeSet::new();
    let mut budget = ParseBudget::default();

    loop {
        if !xref_offsets.insert(xref_offset) {
            return Err(PdfError::syntax("cycle in xref /Prev chain", xref_offset));
        }
        revisions += 1;
        if revisions > limits.max_xref_revisions {
            return Err(PdfError::limit("xref revision count exceeds limit"));
        }
        let (revision_entries, revision_trailer) =
            parse_xref(input, xref_offset, limits, &mut budget)?;
        if dict_integer(&revision_trailer, b"XRefStm").is_some() {
            return Err(PdfError::unsupported(
                "hybrid-reference /XRefStm files are not implemented",
            ));
        }
        total_entries = total_entries
            .checked_add(revision_entries.len())
            .ok_or_else(|| PdfError::limit("xref entry count overflows"))?;
        if total_entries > limits.max_xref_entries || total_entries > limits.max_container_items {
            return Err(PdfError::limit("xref entry count exceeds limit"));
        }
        for (number, entry) in revision_entries {
            entries.entry(number).or_insert(entry);
        }
        if trailer.is_none() {
            trailer = Some(revision_trailer.clone());
        }
        let Some(previous) = dict_integer(&revision_trailer, b"Prev") else {
            break;
        };
        xref_offset = usize::try_from(previous)
            .map_err(|_| PdfError::syntax("invalid negative /Prev offset", xref_offset))?;
    }

    let live_objects = entries
        .values()
        .filter(|entry| !matches!(entry, XrefEntry::Free))
        .count();
    if live_objects > limits.max_objects {
        return Err(PdfError::limit("object count exceeds limit"));
    }
    let mut objects = BTreeMap::new();
    for (number, entry) in &entries {
        let XrefEntry::InUse { offset, generation } = *entry else {
            continue;
        };
        let reference = ObjectRef {
            number: *number,
            generation,
        };
        let (actual, object, _) =
            parse_indirect_with_end(input, offset, limits, false, Some(&entries))?;
        if actual != reference {
            return Err(PdfError::syntax(
                "xref entry points to a different object",
                offset,
            ));
        }
        objects.insert(reference, object);
    }
    let trailer = trailer.ok_or_else(|| PdfError::syntax("missing trailer", xref_offset))?;
    Ok(ParsedDocument {
        version,
        objects,
        trailer,
        xref_revisions: revisions,
        limits: limits.clone(),
        xref_entries: entries,
        decoded_stream_bytes: budget.decoded_bytes,
        compressed_finished: false,
    })
}

pub(crate) fn finish_compressed_objects(parsed: &mut ParsedDocument) -> Result<(), PdfError> {
    if parsed.compressed_finished {
        return Ok(());
    }
    let mut budget = ParseBudget {
        decoded_bytes: parsed.decoded_stream_bytes,
    };
    parse_compressed_objects(
        &parsed.xref_entries,
        &mut parsed.objects,
        &parsed.limits,
        &mut budget,
    )?;
    parsed.decoded_stream_bytes = budget.decoded_bytes;
    parsed.compressed_finished = true;
    Ok(())
}

fn parse_xref(
    input: &[u8],
    offset: usize,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<(BTreeMap<u32, XrefEntry>, Value), PdfError> {
    let mut cursor = Cursor::at(input, offset, limits);
    cursor.skip_ws();
    if !cursor.consume_word(b"xref") {
        return parse_xref_stream(input, offset, limits, budget);
    }
    let mut entries = BTreeMap::new();
    let mut walked_entries = 0usize;
    loop {
        cursor.skip_ws();
        if cursor.consume(b"trailer") {
            let trailer = cursor.value(0)?;
            if !matches!(trailer, Value::Dict(_)) {
                return Err(PdfError::syntax("trailer is not a dictionary", cursor.pos));
            }
            validate_xref_size(&entries, &trailer, cursor.pos, limits)?;
            return Ok((entries, trailer));
        }
        let first = cursor.unsigned()?;
        let count = cursor.unsigned()?;
        walked_entries = walked_entries
            .checked_add(count)
            .ok_or_else(|| PdfError::limit("xref subsection entry count overflows"))?;
        if count > limits.max_xref_entries
            || count > limits.max_container_items
            || walked_entries > limits.max_xref_entries
            || walked_entries > limits.max_container_items
        {
            return Err(PdfError::limit("xref subsection exceeds limit"));
        }
        for index in 0..count {
            let object_offset = cursor.unsigned()?;
            let generation = cursor.unsigned()?;
            let state = cursor.word()?;
            if state == b"n" {
                let number = first
                    .checked_add(index)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| PdfError::limit("object number exceeds u32"))?;
                let generation = u16::try_from(generation)
                    .map_err(|_| PdfError::syntax("generation exceeds u16", cursor.pos))?;
                if object_offset >= input.len() {
                    return Err(PdfError::syntax(
                        "xref object offset is out of bounds",
                        cursor.pos,
                    ));
                }
                entries.insert(
                    number,
                    XrefEntry::InUse {
                        offset: object_offset,
                        generation,
                    },
                );
            } else if state == b"f" {
                let number = first
                    .checked_add(index)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| PdfError::limit("object number exceeds u32"))?;
                entries.insert(number, XrefEntry::Free);
            } else {
                return Err(PdfError::syntax("invalid xref entry state", cursor.pos));
            }
        }
    }
}

fn parse_xref_stream(
    input: &[u8],
    offset: usize,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<(BTreeMap<u32, XrefEntry>, Value), PdfError> {
    let (_, object) = parse_indirect(input, offset, limits)?;
    if dict_name(&object.value, b"Type") != Some(b"XRef".as_slice()) {
        return Err(PdfError::syntax(
            "startxref does not point to an xref table or stream",
            offset,
        ));
    }
    let encoded_stream = object
        .stream
        .as_deref()
        .ok_or_else(|| PdfError::syntax("xref stream object has no stream", offset))?;
    let stream = decode_stream(&object.value, encoded_stream, limits, budget)?;
    let size = required_nonnegative_usize(&object.value, b"Size", offset)?;
    if size > limits.max_xref_entries {
        return Err(PdfError::limit("xref /Size exceeds max_xref_entries"));
    }
    let widths = integer_array(
        &object.value,
        b"W",
        limits.max_container_items,
        "xref stream /W",
    )?
    .ok_or_else(|| PdfError::syntax("xref stream has no /W array", offset))?;
    if widths.len() != 3 {
        return Err(PdfError::syntax(
            "xref stream /W must have three entries",
            offset,
        ));
    }
    let mut width = [0usize; 3];
    for (index, value) in widths.iter().enumerate() {
        width[index] = usize::try_from(*value)
            .map_err(|_| PdfError::syntax("xref stream /W contains a negative width", offset))?;
        if width[index] > 8 {
            return Err(PdfError::unsupported(
                "xref stream field widths greater than eight bytes are not implemented",
            ));
        }
    }
    let row_width = width
        .iter()
        .try_fold(0usize, |sum, value| sum.checked_add(*value))
        .ok_or_else(|| PdfError::limit("xref stream row width overflows"))?;
    if row_width == 0 {
        return Err(PdfError::syntax("xref stream /W row is empty", offset));
    }

    let index = match integer_array(
        &object.value,
        b"Index",
        limits.max_container_items,
        "xref stream /Index",
    )? {
        Some(values) => values.to_vec(),
        None => vec![
            0,
            i64::try_from(size).map_err(|_| PdfError::limit("xref /Size overflows"))?,
        ],
    };
    if index.len() % 2 != 0 {
        return Err(PdfError::syntax(
            "xref stream /Index must contain start/count pairs",
            offset,
        ));
    }
    let mut entries = BTreeMap::new();
    let mut stream_pos = 0usize;
    for pair in index.chunks_exact(2) {
        let first = usize::try_from(pair[0])
            .map_err(|_| PdfError::syntax("xref stream /Index start is negative", offset))?;
        let count = usize::try_from(pair[1])
            .map_err(|_| PdfError::syntax("xref stream /Index count is negative", offset))?;
        let end = first
            .checked_add(count)
            .ok_or_else(|| PdfError::limit("xref stream /Index range overflows"))?;
        if end > size {
            return Err(PdfError::syntax(
                "xref stream /Index range exceeds /Size",
                offset,
            ));
        }
        let entry_count = entries
            .len()
            .checked_add(count)
            .ok_or_else(|| PdfError::limit("xref stream entry count overflows"))?;
        if entry_count > limits.max_xref_entries || entry_count > limits.max_container_items {
            return Err(PdfError::limit("xref stream entry count exceeds limit"));
        }
        for number in first..end {
            let row_end = stream_pos
                .checked_add(row_width)
                .ok_or_else(|| PdfError::limit("xref stream range overflows"))?;
            let row = stream
                .get(stream_pos..row_end)
                .ok_or_else(|| PdfError::syntax("xref stream data is truncated", offset))?;
            stream_pos = row_end;
            let kind = if width[0] == 0 {
                1
            } else {
                read_be(&row[..width[0]])?
            };
            let second_end = width[0] + width[1];
            let second = read_be(&row[width[0]..second_end])?;
            let third = read_be(&row[second_end..])?;
            let number = u32::try_from(number)
                .map_err(|_| PdfError::limit("xref object number exceeds u32"))?;
            let entry = match kind {
                0 => XrefEntry::Free,
                1 => {
                    let object_offset = usize::try_from(second)
                        .map_err(|_| PdfError::limit("xref object offset exceeds usize"))?;
                    if object_offset >= input.len() {
                        return Err(PdfError::syntax(
                            "xref object offset is out of bounds",
                            offset,
                        ));
                    }
                    XrefEntry::InUse {
                        offset: object_offset,
                        generation: u16::try_from(third)
                            .map_err(|_| PdfError::syntax("xref generation exceeds u16", offset))?,
                    }
                }
                2 => XrefEntry::Compressed {
                    stream: u32::try_from(second)
                        .map_err(|_| PdfError::limit("object stream number exceeds u32"))?,
                    index: usize::try_from(third)
                        .map_err(|_| PdfError::limit("object stream index exceeds usize"))?,
                },
                _ => {
                    return Err(PdfError::syntax("unknown xref stream entry type", offset));
                }
            };
            if entries.insert(number, entry).is_some() {
                return Err(PdfError::syntax(
                    "xref stream /Index ranges overlap",
                    offset,
                ));
            }
        }
    }
    if stream_pos != stream.len() {
        return Err(PdfError::syntax(
            "xref stream length does not match /W and /Index",
            offset,
        ));
    }
    Ok((entries, object.value))
}

fn validate_xref_size(
    entries: &BTreeMap<u32, XrefEntry>,
    trailer: &Value,
    offset: usize,
    limits: &Limits,
) -> Result<(), PdfError> {
    let size = required_nonnegative_usize(trailer, b"Size", offset)?;
    if size > limits.max_xref_entries {
        return Err(PdfError::limit("trailer /Size exceeds max_xref_entries"));
    }
    if entries
        .keys()
        .next_back()
        .is_some_and(|number| usize::try_from(*number).is_err() || *number as usize >= size)
    {
        return Err(PdfError::syntax("xref entry exceeds trailer /Size", offset));
    }
    Ok(())
}

fn parse_compressed_objects(
    entries: &BTreeMap<u32, XrefEntry>,
    objects: &mut BTreeMap<ObjectRef, IndirectObject>,
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<(), PdfError> {
    let mut groups: BTreeMap<u32, Vec<(u32, usize)>> = BTreeMap::new();
    for (number, entry) in entries {
        if let XrefEntry::Compressed { stream, index } = entry {
            groups.entry(*stream).or_default().push((*number, *index));
        }
    }
    for (stream_number, wanted) in groups {
        let stream_entry = entries.get(&stream_number).ok_or_else(|| {
            PdfError::syntax("compressed object references a missing object stream", 0)
        })?;
        let XrefEntry::InUse { generation, .. } = stream_entry else {
            return Err(PdfError::syntax(
                "compressed object references a non-file object stream",
                0,
            ));
        };
        let stream_ref = ObjectRef {
            number: stream_number,
            generation: *generation,
        };
        let object_stream = objects
            .get(&stream_ref)
            .ok_or_else(|| PdfError::syntax("compressed object stream was not parsed", 0))?;
        let object_stream_offset = object_stream.offset;
        if dict_name(&object_stream.value, b"Type") != Some(b"ObjStm".as_slice()) {
            return Err(PdfError::syntax(
                "compressed object container is not an /ObjStm",
                object_stream_offset,
            ));
        }
        let encoded_stream = object_stream.stream.as_deref().ok_or_else(|| {
            PdfError::syntax("object stream has no stream data", object_stream_offset)
        })?;
        let stream = decode_stream(&object_stream.value, encoded_stream, limits, budget)?;
        let count = required_nonnegative_usize(&object_stream.value, b"N", object_stream_offset)?;
        let first =
            required_nonnegative_usize(&object_stream.value, b"First", object_stream_offset)?;
        if count > limits.max_objects || count.saturating_mul(2) > limits.max_container_items {
            return Err(PdfError::limit("object stream object count exceeds limit"));
        }
        if first > stream.len() {
            return Err(PdfError::syntax(
                "object stream /First exceeds stream length",
                object_stream_offset,
            ));
        }
        let mut header = Cursor::at(&stream, 0, limits);
        let mut offsets = Vec::with_capacity(count);
        for _ in 0..count {
            let number = u32::try_from(header.unsigned()?)
                .map_err(|_| PdfError::limit("compressed object number exceeds u32"))?;
            let relative = header.unsigned()?;
            offsets.push((number, relative));
        }
        if header.pos > first {
            return Err(PdfError::syntax(
                "object stream header exceeds /First",
                object_stream_offset,
            ));
        }
        let mut previous = None;
        for (_, relative) in &offsets {
            let value_offset = first
                .checked_add(*relative)
                .ok_or_else(|| PdfError::limit("compressed object offset overflows"))?;
            if value_offset >= stream.len() {
                return Err(PdfError::syntax(
                    "compressed object offset exceeds stream length",
                    object_stream_offset,
                ));
            }
            if previous.is_some_and(|previous| *relative <= previous) {
                return Err(PdfError::syntax(
                    "compressed object offsets are not increasing",
                    object_stream_offset,
                ));
            }
            previous = Some(*relative);
        }
        let mut walked_items = count
            .checked_mul(2)
            .ok_or_else(|| PdfError::limit("object stream item count overflows"))?;
        for (number, index) in wanted {
            let (header_number, relative) = *offsets.get(index).ok_or_else(|| {
                PdfError::syntax("compressed object index exceeds /N", object_stream_offset)
            })?;
            if header_number != number {
                return Err(PdfError::syntax(
                    "compressed object number disagrees with object stream header",
                    object_stream_offset,
                ));
            }
            let value_offset = first
                .checked_add(relative)
                .ok_or_else(|| PdfError::limit("compressed object offset overflows"))?;
            if value_offset >= stream.len() {
                return Err(PdfError::syntax(
                    "compressed object offset exceeds stream length",
                    object_stream_offset,
                ));
            }
            let mut cursor = Cursor::at(&stream, value_offset, limits);
            let value = cursor.value(0)?;
            walked_items = walked_items
                .checked_add(cursor.items)
                .ok_or_else(|| PdfError::limit("object stream item count overflows"))?;
            if walked_items > limits.max_container_items {
                return Err(PdfError::limit("object stream item count exceeds limit"));
            }
            objects.insert(
                ObjectRef {
                    number,
                    generation: 0,
                },
                IndirectObject {
                    value,
                    stream: None,
                    stream_offset: 0,
                    offset: object_stream_offset,
                },
            );
        }
    }
    Ok(())
}

fn read_be(bytes: &[u8]) -> Result<u64, PdfError> {
    bytes.iter().try_fold(0u64, |value, byte| {
        value
            .checked_mul(256)
            .and_then(|value| value.checked_add(u64::from(*byte)))
            .ok_or_else(|| PdfError::limit("xref stream field overflows u64"))
    })
}

fn parse_indirect(
    input: &[u8],
    offset: usize,
    limits: &Limits,
) -> Result<(ObjectRef, IndirectObject), PdfError> {
    let (reference, object, _) = parse_indirect_with_end(input, offset, limits, false, None)?;
    Ok((reference, object))
}

fn parse_indirect_with_end(
    input: &[u8],
    offset: usize,
    limits: &Limits,
    repair: bool,
    xref_entries: Option<&BTreeMap<u32, XrefEntry>>,
) -> Result<(ObjectRef, IndirectObject, usize), PdfError> {
    let mut cursor = Cursor::at(input, offset, limits);
    let number = u32::try_from(cursor.unsigned()?)
        .map_err(|_| PdfError::syntax("object number exceeds u32", offset))?;
    let generation = u16::try_from(cursor.unsigned()?)
        .map_err(|_| PdfError::syntax("generation exceeds u16", offset))?;
    cursor.keyword(b"obj")?;
    let value = cursor.value(0)?;
    cursor.skip_ws();
    let mut stream = None;
    let mut stream_offset = 0;
    if cursor.consume(b"stream") {
        match input.get(cursor.pos) {
            Some(b'\r') if input.get(cursor.pos + 1) == Some(&b'\n') => cursor.pos += 2,
            Some(b'\r' | b'\n') => cursor.pos += 1,
            _ => {
                return Err(PdfError::syntax(
                    "stream keyword must be followed by EOL",
                    cursor.pos,
                ));
            }
        }
        let length = match dict_integer(&value, b"Length") {
            Some(length) => usize::try_from(length)
                .map_err(|_| PdfError::syntax("invalid stream length", cursor.pos))?,
            None => match (dict_get(&value, b"Length"), xref_entries) {
                (Some(Value::Ref(reference)), Some(entries)) => {
                    resolve_indirect_stream_length(input, *reference, offset, limits, entries)?
                }
                _ if repair => repair_stream_length(input, cursor.pos, limits)?,
                _ => {
                    return Err(PdfError::unsupported(
                        "indirect stream lengths are not implemented",
                    ));
                }
            },
        };
        if length > limits.max_stream_bytes {
            return Err(PdfError::limit("stream exceeds max_stream_bytes"));
        }
        let end = cursor
            .pos
            .checked_add(length)
            .ok_or_else(|| PdfError::limit("stream range overflows"))?;
        let bytes = input
            .get(cursor.pos..end)
            .ok_or_else(|| PdfError::syntax("stream range exceeds input", cursor.pos))?;
        stream_offset = cursor.pos;
        stream = Some(bytes.to_vec());
        cursor.pos = end;
        cursor.skip_ws();
        cursor.keyword(b"endstream")?;
    }
    cursor.skip_ws();
    cursor.keyword(b"endobj")?;
    let end = cursor.pos;
    Ok((
        ObjectRef { number, generation },
        IndirectObject {
            value,
            stream,
            stream_offset,
            offset,
        },
        end,
    ))
}

fn resolve_indirect_stream_length(
    input: &[u8],
    reference: ObjectRef,
    stream_offset: usize,
    limits: &Limits,
    entries: &BTreeMap<u32, XrefEntry>,
) -> Result<usize, PdfError> {
    let Some(XrefEntry::InUse { offset, generation }) = entries.get(&reference.number) else {
        return Err(PdfError::syntax(
            "stream /Length reference is not an in-use object",
            stream_offset,
        ));
    };
    if *generation != reference.generation || *offset == stream_offset {
        return Err(PdfError::syntax(
            "stream /Length reference does not match its xref entry",
            stream_offset,
        ));
    }
    let (actual, object) = parse_indirect(input, *offset, limits)?;
    if actual != reference {
        return Err(PdfError::syntax(
            "stream /Length xref entry points to a different object",
            *offset,
        ));
    }
    let Value::Integer(length) = object.value else {
        return Err(PdfError::syntax(
            "stream /Length reference is not an integer",
            *offset,
        ));
    };
    usize::try_from(length).map_err(|_| PdfError::syntax("invalid stream length", *offset))
}

fn repair_stream_length(input: &[u8], start: usize, limits: &Limits) -> Result<usize, PdfError> {
    let scan_end = start
        .checked_add(limits.max_stream_bytes)
        .and_then(|end| end.checked_add(b"endstream".len()))
        .map(|end| end.min(input.len()))
        .ok_or_else(|| PdfError::limit("repair stream scan range overflows"))?;
    for relative in input[start..scan_end]
        .windows(b"endstream".len())
        .enumerate()
        .filter_map(|(index, value)| (value == b"endstream").then_some(index))
    {
        let candidate = start + relative;
        if !input
            .get(candidate.wrapping_sub(1))
            .is_some_and(|byte| matches!(byte, b'\r' | b'\n'))
        {
            continue;
        }
        let mut cursor = Cursor::at(input, candidate, limits);
        if cursor.keyword(b"endstream").is_err() {
            continue;
        }
        cursor.skip_ws();
        if !cursor.consume_word(b"endobj") {
            continue;
        }
        return candidate
            .checked_sub(start)
            .ok_or_else(|| PdfError::limit("repair stream length underflows"));
    }
    Err(PdfError::syntax(
        "repair could not find a bounded endstream marker",
        start,
    ))
}

fn pdf_version(input: &[u8]) -> Result<String, PdfError> {
    let header = input
        .windows(5)
        .take(1024)
        .position(|value| value == b"%PDF-")
        .ok_or_else(|| PdfError::syntax("missing PDF header", 0))?;
    let version_end = input[header + 5..]
        .iter()
        .position(|byte| byte.is_ascii_whitespace())
        .map(|position| header + 5 + position)
        .unwrap_or(input.len());
    std::str::from_utf8(&input[header + 5..version_end])
        .map(str::to_owned)
        .map_err(|_| PdfError::syntax("PDF version is not ASCII", header + 5))
}

struct Cursor<'a> {
    input: &'a [u8],
    pos: usize,
    limits: &'a Limits,
    items: usize,
}

impl<'a> Cursor<'a> {
    fn at(input: &'a [u8], pos: usize, limits: &'a Limits) -> Self {
        Self {
            input,
            pos,
            limits,
            items: 0,
        }
    }

    fn skip_ws(&mut self) {
        loop {
            while self.input.get(self.pos).is_some_and(|byte| is_ws(*byte)) {
                self.pos += 1;
            }
            if self.input.get(self.pos) != Some(&b'%') {
                break;
            }
            while self
                .input
                .get(self.pos)
                .is_some_and(|byte| *byte != b'\r' && *byte != b'\n')
            {
                self.pos += 1;
            }
        }
    }

    fn consume(&mut self, token: &[u8]) -> bool {
        if self.input.get(self.pos..self.pos + token.len()) == Some(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    fn consume_word(&mut self, token: &[u8]) -> bool {
        let start = self.pos;
        if self.consume(token)
            && self
                .input
                .get(self.pos)
                .is_none_or(|byte| is_delimiter(*byte))
        {
            true
        } else {
            self.pos = start;
            false
        }
    }

    fn keyword(&mut self, token: &[u8]) -> Result<(), PdfError> {
        self.skip_ws();
        if self.consume_word(token) {
            Ok(())
        } else {
            Err(PdfError::syntax(
                format!("expected {}", String::from_utf8_lossy(token)),
                self.pos,
            ))
        }
    }

    fn word(&mut self) -> Result<&'a [u8], PdfError> {
        self.skip_ws();
        let start = self.pos;
        while self
            .input
            .get(self.pos)
            .is_some_and(|byte| !is_delimiter(*byte))
        {
            self.pos += 1;
        }
        if self.pos == start {
            return Err(PdfError::syntax("expected token", self.pos));
        }
        if self.pos - start > self.limits.max_token_bytes {
            return Err(PdfError::limit("token exceeds max_token_bytes"));
        }
        Ok(&self.input[start..self.pos])
    }

    fn unsigned(&mut self) -> Result<usize, PdfError> {
        let offset = self.pos;
        let word = self.word()?;
        let text = std::str::from_utf8(word)
            .map_err(|_| PdfError::syntax("number is not ASCII", offset))?;
        text.parse()
            .map_err(|_| PdfError::syntax("expected unsigned integer", offset))
    }

    fn value(&mut self, depth: usize) -> Result<Value, PdfError> {
        if depth > self.limits.max_parser_depth {
            return Err(PdfError::limit("parser depth exceeds limit"));
        }
        self.items += 1;
        if self.items > self.limits.max_container_items {
            return Err(PdfError::limit("parsed value count exceeds limit"));
        }
        self.skip_ws();
        match self.input.get(self.pos).copied() {
            Some(b'/') => self.name().map(Value::Name),
            Some(b'(') => self.literal_string().map(Value::String),
            Some(b'[') => self.array(depth + 1),
            Some(b'<') if self.input.get(self.pos + 1) == Some(&b'<') => self.dict(depth + 1),
            Some(b'<') => self.hex_string().map(Value::String),
            Some(b't') if self.consume_word(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.consume_word(b"false") => Ok(Value::Bool(false)),
            Some(b'n') if self.consume_word(b"null") => Ok(Value::Null),
            Some(b'+' | b'-' | b'.' | b'0'..=b'9') => self.number_or_ref(),
            _ => Err(PdfError::syntax("expected PDF value", self.pos)),
        }
    }

    fn name(&mut self) -> Result<Vec<u8>, PdfError> {
        self.pos += 1;
        let mut value = Vec::new();
        while let Some(byte) = self.input.get(self.pos).copied() {
            if is_delimiter(byte) {
                break;
            }
            if value.len() >= self.limits.max_token_bytes {
                return Err(PdfError::limit("name exceeds max_token_bytes"));
            }
            if byte == b'#' {
                let hi = *self
                    .input
                    .get(self.pos + 1)
                    .ok_or_else(|| PdfError::syntax("truncated name escape", self.pos))?;
                let lo = *self
                    .input
                    .get(self.pos + 2)
                    .ok_or_else(|| PdfError::syntax("truncated name escape", self.pos))?;
                value.push(
                    hex_pair(hi, lo)
                        .ok_or_else(|| PdfError::syntax("invalid name escape", self.pos))?,
                );
                self.pos += 3;
            } else {
                value.push(byte);
                self.pos += 1;
            }
        }
        Ok(value)
    }

    fn literal_string(&mut self) -> Result<Vec<u8>, PdfError> {
        let start = self.pos;
        self.pos += 1;
        let mut depth = 1usize;
        let mut value = Vec::new();
        while let Some(byte) = self.input.get(self.pos).copied() {
            self.pos += 1;
            match byte {
                b'(' => {
                    depth += 1;
                    value.push(byte);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(value);
                    }
                    value.push(byte);
                }
                b'\\' => self.literal_escape(&mut value)?,
                _ => value.push(byte),
            }
            if value.len() > self.limits.max_token_bytes {
                return Err(PdfError::limit("string exceeds max_token_bytes"));
            }
            if depth > self.limits.max_parser_depth {
                return Err(PdfError::limit("literal string nesting exceeds limit"));
            }
        }
        Err(PdfError::syntax("unterminated literal string", start))
    }

    fn literal_escape(&mut self, value: &mut Vec<u8>) -> Result<(), PdfError> {
        let byte = *self
            .input
            .get(self.pos)
            .ok_or_else(|| PdfError::syntax("truncated string escape", self.pos))?;
        self.pos += 1;
        match byte {
            b'n' => value.push(b'\n'),
            b'r' => value.push(b'\r'),
            b't' => value.push(b'\t'),
            b'b' => value.push(8),
            b'f' => value.push(12),
            b'(' | b')' | b'\\' => value.push(byte),
            b'\r' => {
                if self.input.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                }
            }
            b'\n' => {}
            b'0'..=b'7' => {
                let mut octal = u16::from(byte - b'0');
                for _ in 0..2 {
                    let Some(next @ b'0'..=b'7') = self.input.get(self.pos).copied() else {
                        break;
                    };
                    self.pos += 1;
                    octal = octal * 8 + u16::from(next - b'0');
                }
                value.push((octal & 0xff) as u8);
            }
            _ => value.push(byte),
        }
        Ok(())
    }

    fn hex_string(&mut self) -> Result<Vec<u8>, PdfError> {
        let start = self.pos;
        self.pos += 1;
        let mut digits = Vec::new();
        loop {
            let byte = *self
                .input
                .get(self.pos)
                .ok_or_else(|| PdfError::syntax("unterminated hex string", start))?;
            self.pos += 1;
            if byte == b'>' {
                break;
            }
            if is_ws(byte) {
                continue;
            }
            if !byte.is_ascii_hexdigit() {
                return Err(PdfError::syntax("invalid hex string digit", self.pos - 1));
            }
            digits.push(byte);
            if digits.len() > self.limits.max_token_bytes.saturating_mul(2) {
                return Err(PdfError::limit("hex string exceeds max_token_bytes"));
            }
        }
        if digits.len() % 2 == 1 {
            digits.push(b'0');
        }
        Ok(digits
            .chunks_exact(2)
            .map(|pair| hex_pair(pair[0], pair[1]).unwrap())
            .collect())
    }

    fn array(&mut self, depth: usize) -> Result<Value, PdfError> {
        self.pos += 1;
        let mut values = Vec::new();
        loop {
            self.skip_ws();
            if self.consume(b"]") {
                return Ok(Value::Array(values));
            }
            values.push(self.value(depth)?);
        }
    }

    fn dict(&mut self, depth: usize) -> Result<Value, PdfError> {
        self.pos += 2;
        let mut values = BTreeMap::new();
        loop {
            self.skip_ws();
            if self.consume(b">>") {
                return Ok(Value::Dict(values));
            }
            if self.input.get(self.pos) != Some(&b'/') {
                return Err(PdfError::syntax("dictionary key is not a name", self.pos));
            }
            let key = self.name()?;
            let value = self.value(depth)?;
            values.insert(key, value);
        }
    }

    fn number_or_ref(&mut self) -> Result<Value, PdfError> {
        let start = self.pos;
        let first = self.word()?;
        let first_text = std::str::from_utf8(first)
            .map_err(|_| PdfError::syntax("number is not ASCII", start))?;
        if !first_text.contains('.') {
            if let Ok(number) = first_text.parse::<u32>() {
                let saved = self.pos;
                if let Ok(generation) = self.unsigned() {
                    self.skip_ws();
                    if generation <= u16::MAX as usize && self.consume_word(b"R") {
                        return Ok(Value::Ref(ObjectRef {
                            number,
                            generation: generation as u16,
                        }));
                    }
                }
                self.pos = saved;
            }
            return first_text
                .parse::<i64>()
                .map(Value::Integer)
                .map_err(|_| PdfError::syntax("invalid integer", start));
        }
        first_text
            .parse::<f64>()
            .map(Value::Real)
            .map_err(|_| PdfError::syntax("invalid real number", start))
    }
}

fn dict_integer(value: &Value, key: &[u8]) -> Option<i64> {
    let Value::Dict(values) = value else {
        return None;
    };
    match values.get(key)? {
        Value::Integer(value) => Some(*value),
        _ => None,
    }
}

#[derive(Default)]
pub(crate) struct ParseBudget {
    decoded_bytes: usize,
}

impl ParseBudget {
    fn remaining_decoded(&self, limits: &Limits) -> Result<usize, PdfError> {
        limits
            .max_total_decoded_bytes
            .checked_sub(self.decoded_bytes)
            .ok_or_else(|| PdfError::limit("cumulative decoded stream bytes exceed limit"))
    }

    fn charge_decoded(&mut self, bytes: usize, limits: &Limits) -> Result<(), PdfError> {
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(bytes)
            .ok_or_else(|| PdfError::limit("cumulative decoded stream bytes overflow"))?;
        if self.decoded_bytes > limits.max_total_decoded_bytes {
            return Err(PdfError::limit(
                "cumulative decoded stream bytes exceed limit",
            ));
        }
        Ok(())
    }
}

pub(crate) fn decode_stream(
    value: &Value,
    input: &[u8],
    limits: &Limits,
    budget: &mut ParseBudget,
) -> Result<Vec<u8>, PdfError> {
    if input.len() > limits.max_stream_bytes {
        return Err(PdfError::limit("encoded stream exceeds max_stream_bytes"));
    }
    let filters = match dict_get(value, b"Filter") {
        None => Vec::new(),
        Some(Value::Name(name)) => vec![pdf_filter(name)],
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                Value::Name(name) => Ok(pdf_filter(name)),
                _ => Err(PdfError::syntax(
                    "stream /Filter array contains a non-name",
                    0,
                )),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(PdfError::syntax(
                "stream /Filter must be a name or array",
                0,
            ));
        }
    };
    let decode_params = match dict_get(value, b"DecodeParms") {
        None | Some(Value::Null) => vec![None; filters.len()],
        Some(params @ Value::Dict(_)) => vec![Some(parse_decode_params(params)?)],
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                Value::Null => Ok(None),
                Value::Dict(_) => parse_decode_params(value).map(Some),
                _ => Err(PdfError::syntax(
                    "stream /DecodeParms array contains an invalid value",
                    0,
                )),
            })
            .collect::<Result<_, _>>()?,
        Some(_) => {
            return Err(PdfError::syntax(
                "stream /DecodeParms must be a dictionary or array",
                0,
            ));
        }
    };
    let output_limit = limits
        .max_stream_bytes
        .min(budget.remaining_decoded(limits)?);
    let output = decode_filter_chain(input, &filters, &decode_params, output_limit)?;
    budget.charge_decoded(output.len(), limits)?;
    Ok(output)
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

fn parse_decode_params(value: &Value) -> Result<DecodeParams, PdfError> {
    let mut params = DecodeParams::default();
    if let Some(value) = dict_integer(value, b"Predictor") {
        params.predictor =
            u8::try_from(value).map_err(|_| PdfError::syntax("/Predictor exceeds u8", 0))?;
    }
    if let Some(value) = dict_integer(value, b"Colors") {
        params.colors = usize::try_from(value)
            .map_err(|_| PdfError::syntax("/Colors must be non-negative", 0))?;
    }
    if let Some(value) = dict_integer(value, b"BitsPerComponent") {
        params.bits_per_component =
            u8::try_from(value).map_err(|_| PdfError::syntax("/BitsPerComponent exceeds u8", 0))?;
    }
    if let Some(value) = dict_integer(value, b"Columns") {
        params.columns = usize::try_from(value)
            .map_err(|_| PdfError::syntax("/Columns must be non-negative", 0))?;
    }
    if let Some(value) = dict_integer(value, b"EarlyChange") {
        params.early_change =
            u8::try_from(value).map_err(|_| PdfError::syntax("/EarlyChange exceeds u8", 0))?;
    }
    Ok(params)
}

fn dict_get<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    let Value::Dict(values) = value else {
        return None;
    };
    values.get(key)
}

fn dict_name<'a>(value: &'a Value, key: &[u8]) -> Option<&'a [u8]> {
    match dict_get(value, key)? {
        Value::Name(value) => Some(value),
        _ => None,
    }
}

fn integer_array(
    value: &Value,
    key: &[u8],
    max_items: usize,
    label: &str,
) -> Result<Option<Vec<i64>>, PdfError> {
    let Some(Value::Array(values)) = dict_get(value, key) else {
        return Ok(None);
    };
    if values.len() > max_items {
        return Err(PdfError::limit(format!("{label} exceeds container limit")));
    }
    Ok(values
        .iter()
        .map(|value| match value {
            Value::Integer(value) => Some(*value),
            _ => None,
        })
        .collect())
}

fn required_nonnegative_usize(value: &Value, key: &[u8], offset: usize) -> Result<usize, PdfError> {
    let value = dict_integer(value, key).ok_or_else(|| {
        PdfError::syntax(
            format!("missing or non-integer /{}", String::from_utf8_lossy(key)),
            offset,
        )
    })?;
    usize::try_from(value).map_err(|_| {
        PdfError::syntax(
            format!("/{} must be non-negative", String::from_utf8_lossy(key)),
            offset,
        )
    })
}

fn rfind(input: &[u8], needle: &[u8]) -> Option<usize> {
    input
        .windows(needle.len())
        .rposition(|value| value == needle)
}

fn is_ws(byte: u8) -> bool {
    matches!(byte, 0 | b'\t' | b'\n' | 12 | b'\r' | b' ')
}
fn is_delimiter(byte: u8) -> bool {
    is_ws(byte)
        || matches!(
            byte,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}
fn hex_pair(hi: u8, lo: u8) -> Option<u8> {
    Some(hex(hi)? * 16 + hex(lo)?)
}
fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
