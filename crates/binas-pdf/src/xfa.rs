use std::collections::{BTreeMap, BTreeSet};

use quick_xml::{
    Reader,
    escape::{escape, unescape},
    events::Event,
};
use serde::{Deserialize, Serialize};

use crate::{
    PdfDocument, PdfError,
    filters::encode_pdf_stream,
    parser::{ObjectRef, ParseBudget, Value, decode_stream},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaPacket {
    pub index: usize,
    pub label: String,
    pub object_number: u32,
    pub object_generation: u16,
    pub root_element: Option<String>,
    pub unsafe_xml: bool,
    pub byte_length: usize,
    pub preview: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaDynamicReport {
    pub present: bool,
    pub dynamic: bool,
    pub static_packets: bool,
    pub markers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaReplaceRequest {
    pub old_text: String,
    pub new_text: String,
    #[serde(default)]
    pub packet_index: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaReplaceReport {
    pub operation: String,
    pub packet_index: usize,
    pub object_number: u32,
    pub replacements: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaReplaceVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub old_text_removed: bool,
    pub new_text_present: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XfaReplaceOutcome {
    pub bytes: Vec<u8>,
    pub report: XfaReplaceReport,
    pub verification: XfaReplaceVerification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaDatasetField {
    pub path: String,
    pub value: String,
    pub packet_index: usize,
    pub object_number: u32,
    pub object_generation: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaTemplateDatasetMapping {
    pub field_name: String,
    pub dataset_path: String,
    pub value: String,
    pub template_packet_index: usize,
    pub dataset_packet_index: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaDatasetSetRequest {
    pub path: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaDatasetMutationReport {
    pub operation: String,
    pub path: String,
    pub packet_index: usize,
    pub object_number: u32,
    pub object_generation: u16,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct XfaDatasetMutationVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub path_state_verified: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XfaDatasetMutationOutcome {
    pub bytes: Vec<u8>,
    pub report: XfaDatasetMutationReport,
    pub verification: XfaDatasetMutationVerification,
}

struct PacketData {
    label: String,
    reference: ObjectRef,
    bytes: Vec<u8>,
}

struct DatasetPacket {
    index: usize,
    packet: PacketData,
    layout: DatasetLayout,
}

struct TemplatePacket {
    index: usize,
    packet: PacketData,
    layout: TemplateLayout,
}

#[derive(Default)]
struct DatasetLayout {
    fields: Vec<DatasetFieldSpan>,
    containers: BTreeSet<String>,
}

struct DatasetFieldSpan {
    path: String,
    value: String,
    start: usize,
    content_start: usize,
    content_end: usize,
    end: usize,
    raw_name: String,
    self_closing: bool,
}

#[derive(Default)]
struct TemplateLayout {
    fields: Vec<TemplateField>,
}

struct TemplateField {
    name: String,
    candidates: Vec<String>,
}

struct DatasetFrame {
    local_name: String,
    raw_name: String,
    path: Option<String>,
    is_root: bool,
    inside_data: bool,
    start: usize,
    content_start: usize,
    has_child: bool,
    non_whitespace_text: bool,
    unsupported_content: bool,
}

struct TemplateFrame {
    local_name: String,
    named_subform_path: Option<String>,
    named_subform_path_safe: bool,
}

#[derive(Clone, Copy)]
enum DatasetMutation<'a> {
    Set(&'a str),
    Remove,
}

pub fn list_xfa_packets(document: &PdfDocument) -> Result<Vec<XfaPacket>, PdfError> {
    packet_data(document)?
        .into_iter()
        .enumerate()
        .map(|(index, packet)| {
            let (root_element, unsafe_xml, _) = inspect_xml(&packet.bytes)?;
            let preview = String::from_utf8_lossy(&packet.bytes)
                .chars()
                .take(80)
                .collect();
            Ok(XfaPacket {
                index,
                label: packet.label,
                object_number: packet.reference.number,
                object_generation: packet.reference.generation,
                root_element,
                unsafe_xml,
                byte_length: packet.bytes.len(),
                preview,
            })
        })
        .collect()
}

pub fn inspect_xfa_dynamic(document: &PdfDocument) -> Result<XfaDynamicReport, PdfError> {
    let packets = packet_data(document)?;
    let mut report = XfaDynamicReport {
        present: !packets.is_empty(),
        static_packets: !packets.is_empty(),
        ..XfaDynamicReport::default()
    };
    for packet in packets {
        let (_, unsafe_xml, markers) = inspect_xml(&packet.bytes)?;
        if unsafe_xml {
            report
                .markers
                .push(format!("{}: unsafe XML declaration", packet.label));
        }
        report.markers.extend(
            markers
                .into_iter()
                .map(|marker| format!("{}: {marker}", packet.label)),
        );
    }
    report.dynamic = !report.markers.is_empty();
    report.static_packets &= !report.dynamic;
    Ok(report)
}

pub fn list_xfa_dataset_fields(document: &PdfDocument) -> Result<Vec<XfaDatasetField>, PdfError> {
    let dataset = static_dataset_packet(document)?;
    Ok(dataset
        .layout
        .fields
        .iter()
        .map(|field| XfaDatasetField {
            path: field.path.clone(),
            value: field.value.clone(),
            packet_index: dataset.index,
            object_number: dataset.packet.reference.number,
            object_generation: dataset.packet.reference.generation,
        })
        .collect())
}

pub fn list_xfa_template_dataset_mappings(
    document: &PdfDocument,
) -> Result<Vec<XfaTemplateDatasetMapping>, PdfError> {
    require_static_xfa_template_datasets(document)?;
    let template = static_template_packet(document)?;
    let dataset = static_dataset_packet(document)?;
    let mut datasets_by_path = BTreeMap::<&str, Vec<&DatasetFieldSpan>>::new();
    for field in &dataset.layout.fields {
        datasets_by_path.entry(&field.path).or_default().push(field);
    }

    let mut mappings = Vec::new();
    for field in &template.layout.fields {
        let Some((dataset_path, dataset_field)) = field.candidates.iter().find_map(|candidate| {
            let fields = datasets_by_path.get(candidate.as_str())?;
            (fields.len() == 1).then_some((candidate, fields[0]))
        }) else {
            continue;
        };
        mappings.push(XfaTemplateDatasetMapping {
            field_name: field.name.clone(),
            dataset_path: dataset_path.clone(),
            value: dataset_field.value.clone(),
            template_packet_index: template.index,
            dataset_packet_index: dataset.index,
            label: template.packet.label.clone(),
        });
    }
    Ok(mappings)
}

impl PdfDocument {
    pub fn get_xfa_dataset_field(&self, path: &str) -> Result<XfaDatasetField, PdfError> {
        validate_dataset_path(path, &self.parsed().limits)?;
        let dataset = static_dataset_packet(self)?;
        let field = select_dataset_field(&dataset.layout, path)?;
        Ok(XfaDatasetField {
            path: field.path.clone(),
            value: field.value.clone(),
            packet_index: dataset.index,
            object_number: dataset.packet.reference.number,
            object_generation: dataset.packet.reference.generation,
        })
    }

    pub fn set_xfa_dataset_field(
        &self,
        request: XfaDatasetSetRequest,
    ) -> Result<XfaDatasetMutationOutcome, PdfError> {
        validate_dataset_path(&request.path, &self.parsed().limits)?;
        if request.value.len() > self.parsed().limits.max_token_bytes {
            return Err(PdfError::limit("XFA dataset value exceeds max_token_bytes"));
        }
        self.mutate_xfa_dataset_field(&request.path, DatasetMutation::Set(&request.value))
    }

    pub fn remove_xfa_dataset_field(
        &self,
        path: &str,
    ) -> Result<XfaDatasetMutationOutcome, PdfError> {
        validate_dataset_path(path, &self.parsed().limits)?;
        self.mutate_xfa_dataset_field(path, DatasetMutation::Remove)
    }

    fn mutate_xfa_dataset_field(
        &self,
        path: &str,
        mutation: DatasetMutation<'_>,
    ) -> Result<XfaDatasetMutationOutcome, PdfError> {
        require_static_xfa_template_datasets(self)?;
        let dataset = static_dataset_packet(self)?;
        let field = select_dataset_field(&dataset.layout, path)?;
        let replacement = match mutation {
            DatasetMutation::Set(value) => replace_dataset_field_value(
                &dataset.packet.bytes,
                field,
                value,
                &self.parsed().limits,
            )?,
            DatasetMutation::Remove => remove_dataset_field(&dataset.packet.bytes, field)?,
        };
        let old_pages = self.page_count()?;
        let (bytes, reopened) = rewrite_xfa_packet(self, &dataset.packet, &replacement)?;
        let output = static_dataset_packet(&reopened).map_err(|error| {
            PdfError::verification(format!("XFA dataset mutation output is unsafe: {error}"))
        })?;
        let path_state_verified = match mutation {
            DatasetMutation::Set(value) => {
                let mut matching = output
                    .layout
                    .fields
                    .iter()
                    .filter(|candidate| candidate.path == path);
                matches!(matching.next(), Some(candidate) if candidate.value == value)
                    && matching.next().is_none()
            }
            DatasetMutation::Remove => {
                !output.layout.containers.contains(path)
                    && !output
                        .layout
                        .fields
                        .iter()
                        .any(|candidate| candidate.path == path)
            }
        };
        let page_count_unchanged = reopened.page_count()? == old_pages;
        let verification = XfaDatasetMutationVerification {
            passed: page_count_unchanged && path_state_verified,
            reparsed: true,
            page_count_unchanged,
            path_state_verified,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "XFA dataset mutation failed post-write verification",
            ));
        }
        let operation = match mutation {
            DatasetMutation::Set(_) => "set_xfa_dataset_field",
            DatasetMutation::Remove => "remove_xfa_dataset_field",
        };
        Ok(XfaDatasetMutationOutcome {
            report: XfaDatasetMutationReport {
                operation: operation.into(),
                path: path.into(),
                packet_index: dataset.index,
                object_number: dataset.packet.reference.number,
                object_generation: dataset.packet.reference.generation,
                input_bytes: self.source_len(),
                output_bytes: bytes.len(),
            },
            bytes,
            verification,
        })
    }

    pub fn replace_xfa_text(
        &self,
        request: XfaReplaceRequest,
    ) -> Result<XfaReplaceOutcome, PdfError> {
        if request.old_text.is_empty() {
            return Err(PdfError::syntax("XFA replacement text is empty", 0));
        }
        let packets = packet_data(self)?;
        let packet = packets
            .get(request.packet_index)
            .ok_or_else(|| PdfError::selection("XFA packet index is out of range"))?;
        let old = request.old_text.as_bytes();
        let replacements = packet
            .bytes
            .windows(old.len())
            .filter(|value| *value == old)
            .count();
        if replacements == 0 {
            return Err(PdfError::selection(
                "XFA packet does not contain the selected text",
            ));
        }
        let replaced = replace_all(&packet.bytes, old, request.new_text.as_bytes());
        let (_, unsafe_xml, _) = inspect_xml(&replaced)?;
        if unsafe_xml {
            return Err(PdfError::unsafe_rewrite(
                "XFA replacement refuses XML with a document type declaration",
            ));
        }

        let old_pages = self.page_count()?;
        let mut parsed = self.parsed().clone();
        let object = parsed
            .objects
            .get_mut(&packet.reference)
            .ok_or_else(|| PdfError::syntax("XFA packet object disappeared during mutation", 0))?;
        let encoded = encode_pdf_stream(&object.value, &replaced, &parsed.limits)?;
        let Value::Dict(dictionary) = &mut object.value else {
            return Err(PdfError::unsupported(
                "XFA packet stream dictionary is malformed",
            ));
        };
        if matches!(dictionary.get(b"Length".as_slice()), Some(Value::Ref(_))) {
            return Err(PdfError::unsupported(
                "XFA replacement does not support indirect stream lengths",
            ));
        }
        dictionary.insert(
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(encoded.len())
                    .map_err(|_| PdfError::limit("XFA stream length exceeds i64"))?,
            ),
        );
        object.stream = Some(encoded);
        let canonical = self.with_parsed(parsed).canonicalize()?;
        let reopened = crate::PdfEngine::new(self.engine_config().clone())
            .open(&canonical.bytes, crate::OpenOptions::default())?;
        let packets = packet_data(&reopened)?;
        let output_packet = packets
            .get(request.packet_index)
            .ok_or_else(|| PdfError::verification("XFA packet disappeared after replacement"))?;
        let old_text_removed = !output_packet
            .bytes
            .windows(old.len())
            .any(|value| value == old);
        let new = request.new_text.as_bytes();
        let new_text_present = new.is_empty()
            || output_packet
                .bytes
                .windows(new.len())
                .any(|value| value == new);
        let page_count_unchanged = reopened.page_count()? == old_pages;
        let verification = XfaReplaceVerification {
            passed: page_count_unchanged && old_text_removed && new_text_present,
            reparsed: true,
            page_count_unchanged,
            old_text_removed,
            new_text_present,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "XFA replacement verification failed",
            ));
        }
        Ok(XfaReplaceOutcome {
            report: XfaReplaceReport {
                operation: "replace_xfa_text".into(),
                packet_index: request.packet_index,
                object_number: packet.reference.number,
                replacements,
                input_bytes: self.source_len(),
                output_bytes: canonical.bytes.len(),
            },
            bytes: canonical.bytes,
            verification,
        })
    }
}

fn static_dataset_packet(document: &PdfDocument) -> Result<DatasetPacket, PdfError> {
    let dynamic = inspect_xfa_dynamic(document)?;
    if !dynamic.present {
        return Err(PdfError::selection("PDF has no XFA datasets packet"));
    }
    if !dynamic.static_packets {
        return Err(PdfError::unsafe_rewrite(
            "XFA dataset paths require static XFA without dynamic or unsafe XML markers",
        ));
    }
    let mut candidates = packet_data(document)?
        .into_iter()
        .enumerate()
        .filter_map(|(index, packet)| {
            let root = inspect_xml(&packet.bytes).ok()?.0;
            (packet.label == "datasets"
                || (packet.label == "xfa" && root.as_deref() == Some("datasets")))
            .then_some((index, packet))
        })
        .collect::<Vec<_>>();
    match candidates.len() {
        0 => Err(PdfError::selection("PDF has no static XFA datasets packet")),
        1 => {
            let (index, packet) = candidates.pop().expect("checked candidate length");
            let layout = parse_dataset_layout(&packet.bytes, &document.parsed().limits)?;
            Ok(DatasetPacket {
                index,
                packet,
                layout,
            })
        }
        _ => Err(PdfError::unsafe_rewrite(
            "XFA dataset paths require exactly one datasets packet",
        )),
    }
}

fn static_template_packet(document: &PdfDocument) -> Result<TemplatePacket, PdfError> {
    let mut candidates = packet_data(document)?
        .into_iter()
        .enumerate()
        .filter_map(|(index, packet)| {
            let root = inspect_xml(&packet.bytes).ok()?.0;
            (packet.label == "template"
                || (packet.label == "xfa" && root.as_deref() == Some("template")))
            .then_some((index, packet))
        })
        .collect::<Vec<_>>();
    match candidates.len() {
        0 => Err(PdfError::selection("PDF has no static XFA template packet")),
        1 => {
            let (index, packet) = candidates.pop().expect("checked candidate length");
            let layout = parse_template_layout(&packet.bytes, &document.parsed().limits)?;
            Ok(TemplatePacket {
                index,
                packet,
                layout,
            })
        }
        _ => Err(PdfError::unsafe_rewrite(
            "XFA template-to-dataset mappings require exactly one template packet",
        )),
    }
}

fn require_static_xfa_template_datasets(document: &PdfDocument) -> Result<(), PdfError> {
    for packet in packet_data(document)? {
        let (root, unsafe_xml, markers) = inspect_xml(&packet.bytes)?;
        if unsafe_xml || !markers.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "XFA template/dataset access requires static XFA without dynamic or unsafe XML markers",
            ));
        }
        if !matches!(
            xfa_semantic_packet_kind(&packet.label, root.as_deref()),
            "template" | "datasets"
        ) {
            return Err(PdfError::unsafe_rewrite(
                "XFA template/dataset access requires packet families limited to static template/datasets",
            ));
        }
    }
    Ok(())
}

fn xfa_semantic_packet_kind<'a>(label: &'a str, root: Option<&'a str>) -> &'a str {
    match label.trim() {
        "template" | "datasets" => label.trim(),
        _ => match root {
            Some("template") => "template",
            Some("datasets") => "datasets",
            _ => "",
        },
    }
}

fn validate_dataset_path(path: &str, limits: &crate::Limits) -> Result<(), PdfError> {
    if path.is_empty() || path.len() > limits.max_token_bytes {
        return Err(PdfError::selection(
            "XFA dataset path is empty or exceeds max_token_bytes",
        ));
    }
    if !path.split('.').all(valid_dataset_path_segment) {
        return Err(PdfError::unsafe_rewrite(
            "XFA dataset paths only support ASCII letter, digit, underscore, and hyphen segments",
        ));
    }
    Ok(())
}

fn valid_dataset_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn select_dataset_field<'a>(
    layout: &'a DatasetLayout,
    path: &str,
) -> Result<&'a DatasetFieldSpan, PdfError> {
    if layout.containers.contains(path) {
        return Err(PdfError::unsafe_rewrite(
            "XFA dataset path selects a container rather than a leaf value",
        ));
    }
    let mut matching = layout.fields.iter().filter(|field| field.path == path);
    let field = matching
        .next()
        .ok_or_else(|| PdfError::selection("XFA dataset path was not found"))?;
    if matching.next().is_some() {
        return Err(PdfError::unsafe_rewrite("XFA dataset path is ambiguous"));
    }
    Ok(field)
}

fn replace_dataset_field_value(
    input: &[u8],
    field: &DatasetFieldSpan,
    value: &str,
    limits: &crate::Limits,
) -> Result<Vec<u8>, PdfError> {
    let escaped = escape(value);
    if field.self_closing {
        let tag = checked_dataset_span(input, field.start, field.end)?;
        let close = tag.iter().rposition(|byte| *byte == b'>').ok_or_else(|| {
            PdfError::unsafe_rewrite("XFA dataset empty element has no closing angle")
        })?;
        let slash = tag[..close]
            .iter()
            .rposition(|byte| !byte.is_ascii_whitespace())
            .ok_or_else(|| PdfError::unsafe_rewrite("XFA dataset empty element is malformed"))?;
        if tag[slash] != b'/' {
            return Err(PdfError::unsafe_rewrite(
                "XFA dataset empty element is not safely self-closing",
            ));
        }
        let replacement_len = slash
            .checked_add(escaped.len())
            .and_then(|length| length.checked_add(field.raw_name.len()))
            .and_then(|length| length.checked_add(4))
            .ok_or_else(|| PdfError::limit("XFA dataset replacement length overflows usize"))?;
        let output_len =
            checked_dataset_output_len(input.len(), tag.len(), replacement_len, limits)?;
        let mut output = Vec::with_capacity(output_len);
        output.extend_from_slice(checked_dataset_span(input, 0, field.start)?);
        output.extend_from_slice(&tag[..slash]);
        output.push(b'>');
        output.extend_from_slice(escaped.as_bytes());
        output.extend_from_slice(b"</");
        output.extend_from_slice(field.raw_name.as_bytes());
        output.push(b'>');
        output.extend_from_slice(checked_dataset_span(input, field.end, input.len())?);
        return Ok(output);
    }
    let output_len = checked_dataset_output_len(
        input.len(),
        field.content_end - field.content_start,
        escaped.len(),
        limits,
    )?;
    let mut output = Vec::with_capacity(output_len);
    output.extend_from_slice(checked_dataset_span(input, 0, field.content_start)?);
    output.extend_from_slice(escaped.as_bytes());
    output.extend_from_slice(checked_dataset_span(input, field.content_end, input.len())?);
    Ok(output)
}

fn checked_dataset_output_len(
    input_len: usize,
    replaced_len: usize,
    replacement_len: usize,
    limits: &crate::Limits,
) -> Result<usize, PdfError> {
    let output_len = input_len
        .checked_sub(replaced_len)
        .and_then(|length| length.checked_add(replacement_len))
        .ok_or_else(|| PdfError::limit("XFA dataset replacement length overflows usize"))?;
    if output_len > limits.max_stream_bytes {
        return Err(PdfError::limit(
            "XFA dataset replacement exceeds max_stream_bytes",
        ));
    }
    Ok(output_len)
}

fn remove_dataset_field(input: &[u8], field: &DatasetFieldSpan) -> Result<Vec<u8>, PdfError> {
    let mut output = Vec::with_capacity(input.len() - (field.end - field.start));
    output.extend_from_slice(checked_dataset_span(input, 0, field.start)?);
    output.extend_from_slice(checked_dataset_span(input, field.end, input.len())?);
    Ok(output)
}

fn checked_dataset_span(input: &[u8], start: usize, end: usize) -> Result<&[u8], PdfError> {
    input
        .get(start..end)
        .ok_or_else(|| PdfError::unsafe_rewrite("XFA dataset field span is outside its packet"))
}

fn rewrite_xfa_packet(
    document: &PdfDocument,
    packet: &PacketData,
    replacement: &[u8],
) -> Result<(Vec<u8>, PdfDocument), PdfError> {
    let mut parsed = document.parsed().clone();
    let object = parsed
        .objects
        .get_mut(&packet.reference)
        .ok_or_else(|| PdfError::syntax("XFA packet object disappeared during mutation", 0))?;
    let encoded = encode_pdf_stream(&object.value, replacement, &parsed.limits)?;
    let Value::Dict(dictionary) = &mut object.value else {
        return Err(PdfError::unsupported(
            "XFA packet stream dictionary is malformed",
        ));
    };
    if matches!(dictionary.get(b"Length".as_slice()), Some(Value::Ref(_))) {
        return Err(PdfError::unsupported(
            "XFA replacement does not support indirect stream lengths",
        ));
    }
    dictionary.insert(
        b"Length".to_vec(),
        Value::Integer(
            i64::try_from(encoded.len())
                .map_err(|_| PdfError::limit("XFA stream length exceeds i64"))?,
        ),
    );
    object.stream = Some(encoded);
    let canonical = document.with_parsed(parsed).canonicalize()?;
    let reopened = crate::PdfEngine::new(document.engine_config().clone())
        .open(&canonical.bytes, crate::OpenOptions::default())
        .map_err(|error| PdfError::verification(format!("XFA output did not reparse: {error}")))?;
    Ok((canonical.bytes, reopened))
}

fn parse_template_layout(input: &[u8], limits: &crate::Limits) -> Result<TemplateLayout, PdfError> {
    let _ = std::str::from_utf8(input)
        .map_err(|_| PdfError::unsafe_rewrite("XFA template mappings require UTF-8 XML"))?;
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(false);
    let mut layout = TemplateLayout::default();
    let mut stack = Vec::<TemplateFrame>::new();
    let mut root_seen = false;
    let mut root_closed = false;

    loop {
        let event_start = usize::try_from(reader.buffer_position())
            .map_err(|_| PdfError::limit("XFA XML offset exceeds usize"))?;
        let event = reader
            .read_event()
            .map_err(|_| PdfError::syntax("XFA template packet is malformed XML", event_start))?;
        let event_end = usize::try_from(reader.buffer_position())
            .map_err(|_| PdfError::limit("XFA XML offset exceeds usize"))?;
        match event {
            Event::Decl(declaration) => {
                if root_seen || !stack.is_empty() {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA template XML declaration must precede the root element",
                    ));
                }
                let encoding = declaration.encoding().transpose().map_err(|_| {
                    PdfError::syntax("XFA XML declaration is malformed", event_start)
                })?;
                if encoding.is_some_and(|value| !value.as_ref().eq_ignore_ascii_case(b"utf-8")) {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA template mappings only support UTF-8 XML declarations",
                    ));
                }
            }
            Event::Start(element) => {
                if stack.len() >= limits.max_parser_depth {
                    return Err(PdfError::limit(
                        "XFA template XML exceeds parser depth limit",
                    ));
                }
                let (local_name, raw_name) = xml_start_names(&element, event_start)?;
                if raw_name.contains(':') {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA template mappings only support unqualified XML elements",
                    ));
                }
                if stack.is_empty() {
                    if root_seen || local_name != "template" {
                        return Err(PdfError::unsafe_rewrite(
                            "XFA template mappings require a single template root element",
                        ));
                    }
                    root_seen = true;
                    stack.push(TemplateFrame {
                        local_name,
                        named_subform_path: None,
                        named_subform_path_safe: true,
                    });
                    continue;
                }
                let parent = stack.last().expect("checked template stack");
                if local_name == "field"
                    && let Some(field) = template_field(parent, &element, limits)?
                {
                    if layout.fields.len() >= limits.max_container_items {
                        return Err(PdfError::limit(
                            "XFA template field count exceeds container limit",
                        ));
                    }
                    layout.fields.push(field);
                }
                stack.push(template_child_frame(parent, local_name, &element, limits)?);
            }
            Event::Empty(element) => {
                let (local_name, raw_name) = xml_start_names(&element, event_start)?;
                if raw_name.contains(':') {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA template mappings only support unqualified XML elements",
                    ));
                }
                let Some(parent) = stack.last() else {
                    if root_seen || local_name != "template" {
                        return Err(PdfError::unsafe_rewrite(
                            "XFA template mappings require a single template root element",
                        ));
                    }
                    root_seen = true;
                    root_closed = true;
                    continue;
                };
                if local_name == "field"
                    && let Some(field) = template_field(parent, &element, limits)?
                {
                    if layout.fields.len() >= limits.max_container_items {
                        return Err(PdfError::limit(
                            "XFA template field count exceeds container limit",
                        ));
                    }
                    layout.fields.push(field);
                }
            }
            Event::Text(_) | Event::CData(_) | Event::GeneralRef(_) if stack.is_empty() => {
                let text =
                    std::str::from_utf8(checked_dataset_span(input, event_start, event_end)?)
                        .map_err(|_| {
                            PdfError::unsafe_rewrite("XFA template mappings require UTF-8 XML")
                        })?;
                if !text.trim().is_empty() {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA template XML has text outside its root element",
                    ));
                }
            }
            Event::DocType(_) => {
                return Err(PdfError::unsafe_rewrite(
                    "XFA template mappings do not support document type declarations",
                ));
            }
            Event::End(element) => {
                let local_name = String::from_utf8(element.local_name().as_ref().to_vec())
                    .map_err(|_| {
                        PdfError::syntax("XFA XML element name is invalid", event_start)
                    })?;
                let frame = stack.pop().ok_or_else(|| {
                    PdfError::syntax(
                        "XFA template XML has an unexpected closing element",
                        event_start,
                    )
                })?;
                if frame.local_name != local_name {
                    return Err(PdfError::syntax(
                        "XFA template XML closing element does not match its opening element",
                        event_start,
                    ));
                }
                if stack.is_empty() {
                    root_closed = true;
                }
            }
            Event::Eof => {
                if !stack.is_empty() {
                    return Err(PdfError::syntax(
                        "XFA template XML is truncated",
                        event_start,
                    ));
                }
                if !root_seen || !root_closed {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA template mappings require a complete template XML document",
                    ));
                }
                break;
            }
            _ => {}
        }
    }
    Ok(layout)
}

fn template_child_frame(
    parent: &TemplateFrame,
    local_name: String,
    element: &quick_xml::events::BytesStart<'_>,
    limits: &crate::Limits,
) -> Result<TemplateFrame, PdfError> {
    let mut named_subform_path = parent.named_subform_path.clone();
    let mut named_subform_path_safe = parent.named_subform_path_safe;
    if local_name == "subform"
        && let Some(name) = template_element_name(element)?
    {
        if !valid_dataset_path_segment(&name) {
            named_subform_path = None;
            named_subform_path_safe = false;
        } else if named_subform_path_safe {
            named_subform_path = Some(dataset_child_path(
                named_subform_path.as_deref(),
                &name,
                limits,
            )?);
        }
    }
    Ok(TemplateFrame {
        local_name,
        named_subform_path,
        named_subform_path_safe,
    })
}

fn template_field(
    parent: &TemplateFrame,
    element: &quick_xml::events::BytesStart<'_>,
    limits: &crate::Limits,
) -> Result<Option<TemplateField>, PdfError> {
    let Some(name) = template_element_name(element)? else {
        return Ok(None);
    };
    if name.len() > limits.max_token_bytes {
        return Err(PdfError::limit(
            "XFA template field name exceeds max_token_bytes",
        ));
    }
    if name.is_empty() || !name.split('.').all(valid_dataset_path_segment) {
        return Ok(None);
    }
    let candidates = if name.contains('.') {
        vec![name.clone()]
    } else if parent.named_subform_path_safe {
        vec![match parent.named_subform_path.as_deref() {
            Some(path) => dataset_child_path(Some(path), &name, limits)?,
            None => name.clone(),
        }]
    } else {
        Vec::new()
    };
    Ok((!candidates.is_empty()).then_some(TemplateField { name, candidates }))
}

fn template_element_name(
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<Option<String>, PdfError> {
    let mut name = None;
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|_| PdfError::syntax("XFA XML attribute is malformed", 0))?;
        if attribute.key.as_ref() == b"name" {
            if name.is_some() {
                return Err(PdfError::unsafe_rewrite(
                    "XFA template element has duplicate name attributes",
                ));
            }
            let value = std::str::from_utf8(attribute.value.as_ref())
                .map_err(|_| PdfError::syntax("XFA XML attribute is malformed", 0))?;
            name = Some(
                unescape(value)
                    .map_err(|_| PdfError::syntax("XFA XML attribute is malformed", 0))?
                    .into_owned(),
            );
        }
    }
    Ok(name)
}

fn parse_dataset_layout(input: &[u8], limits: &crate::Limits) -> Result<DatasetLayout, PdfError> {
    let _ = std::str::from_utf8(input)
        .map_err(|_| PdfError::unsafe_rewrite("XFA dataset paths require UTF-8 XML"))?;
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(false);
    let mut layout = DatasetLayout::default();
    let mut stack = Vec::<DatasetFrame>::new();
    let mut root_seen = false;
    let mut root_closed = false;
    let mut data_count = 0usize;

    loop {
        let event_start = usize::try_from(reader.buffer_position())
            .map_err(|_| PdfError::limit("XFA XML offset exceeds usize"))?;
        let event = reader
            .read_event()
            .map_err(|_| PdfError::syntax("XFA dataset packet is malformed XML", event_start))?;
        let event_end = usize::try_from(reader.buffer_position())
            .map_err(|_| PdfError::limit("XFA XML offset exceeds usize"))?;
        match event {
            Event::Decl(declaration) => {
                if root_seen || !stack.is_empty() {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset XML declaration must precede the root element",
                    ));
                }
                let encoding = declaration.encoding().transpose().map_err(|_| {
                    PdfError::syntax("XFA XML declaration is malformed", event_start)
                })?;
                if encoding.is_some_and(|value| !value.as_ref().eq_ignore_ascii_case(b"utf-8")) {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset paths only support UTF-8 XML declarations",
                    ));
                }
            }
            Event::Start(element) => {
                if stack.len() >= limits.max_parser_depth {
                    return Err(PdfError::limit(
                        "XFA dataset XML exceeds parser depth limit",
                    ));
                }
                let (local_name, raw_name) = xml_start_names(&element, event_start)?;
                if stack.is_empty() {
                    if root_seen || local_name != "datasets" {
                        return Err(PdfError::unsafe_rewrite(
                            "XFA dataset paths require a single datasets root element",
                        ));
                    }
                    root_seen = true;
                    stack.push(DatasetFrame {
                        local_name,
                        raw_name,
                        path: None,
                        is_root: true,
                        inside_data: false,
                        start: event_start,
                        content_start: event_end,
                        has_child: false,
                        non_whitespace_text: false,
                        unsupported_content: false,
                    });
                    continue;
                }
                let (parent_is_root, parent_inside_data, parent_path) = {
                    let parent = stack.last_mut().expect("checked stack");
                    parent.has_child = true;
                    (parent.is_root, parent.inside_data, parent.path.clone())
                };
                if parent_is_root {
                    if local_name != "data" || data_count != 0 {
                        return Err(PdfError::unsafe_rewrite(
                            "XFA datasets must contain exactly one direct data element",
                        ));
                    }
                    data_count += 1;
                    stack.push(DatasetFrame {
                        local_name,
                        raw_name,
                        path: None,
                        is_root: false,
                        inside_data: true,
                        start: event_start,
                        content_start: event_end,
                        has_child: false,
                        non_whitespace_text: false,
                        unsupported_content: false,
                    });
                    continue;
                }
                if !parent_inside_data
                    || !valid_dataset_path_segment(&local_name)
                    || raw_name.contains(':')
                {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset paths only support simple unqualified data elements",
                    ));
                }
                let path = dataset_child_path(parent_path.as_deref(), &local_name, limits)?;
                stack.push(DatasetFrame {
                    local_name,
                    raw_name,
                    path: Some(path),
                    is_root: false,
                    inside_data: true,
                    start: event_start,
                    content_start: event_end,
                    has_child: false,
                    non_whitespace_text: false,
                    unsupported_content: false,
                });
            }
            Event::Empty(element) => {
                if stack.len() >= limits.max_parser_depth {
                    return Err(PdfError::limit(
                        "XFA dataset XML exceeds parser depth limit",
                    ));
                }
                let (local_name, raw_name) = xml_start_names(&element, event_start)?;
                let Some(parent) = stack.last_mut() else {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset paths require a non-empty datasets root element",
                    ));
                };
                parent.has_child = true;
                if parent.is_root {
                    if local_name != "data" || data_count != 0 {
                        return Err(PdfError::unsafe_rewrite(
                            "XFA datasets must contain exactly one direct data element",
                        ));
                    }
                    data_count += 1;
                    continue;
                }
                if !parent.inside_data
                    || !valid_dataset_path_segment(&local_name)
                    || raw_name.contains(':')
                {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset paths only support simple unqualified data elements",
                    ));
                }
                if layout.fields.len() >= limits.max_container_items {
                    return Err(PdfError::limit(
                        "XFA dataset field count exceeds container limit",
                    ));
                }
                let path = dataset_child_path(parent.path.as_deref(), &local_name, limits)?;
                layout.fields.push(DatasetFieldSpan {
                    path,
                    value: String::new(),
                    start: event_start,
                    content_start: event_end,
                    content_end: event_end,
                    end: event_end,
                    raw_name,
                    self_closing: true,
                });
            }
            Event::Text(_) => {
                let text =
                    std::str::from_utf8(checked_dataset_span(input, event_start, event_end)?)
                        .map_err(|_| {
                            PdfError::unsafe_rewrite("XFA dataset paths require UTF-8 XML")
                        })?;
                if let Some(frame) = stack.last_mut() {
                    if frame.path.is_some() {
                        frame.non_whitespace_text |= !text.trim().is_empty();
                    } else if !text.trim().is_empty() {
                        return Err(PdfError::unsafe_rewrite(
                            "XFA datasets root and data containers may not contain text",
                        ));
                    }
                } else if !text.trim().is_empty() {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset XML has text outside its root element",
                    ));
                }
            }
            Event::GeneralRef(_) => {
                let Some(frame) = stack.last_mut() else {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset XML has an entity outside its root element",
                    ));
                };
                if frame.path.is_none() {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA datasets root and data containers may not contain entities",
                    ));
                }
                frame.non_whitespace_text = true;
            }
            Event::CData(_) => {
                return Err(PdfError::unsafe_rewrite(
                    "XFA dataset paths do not support CDATA sections",
                ));
            }
            Event::Comment(_) | Event::PI(_) => {
                if let Some(frame) = stack.last_mut().filter(|frame| frame.path.is_some()) {
                    frame.unsupported_content = true;
                }
            }
            Event::DocType(_) => {
                return Err(PdfError::unsafe_rewrite(
                    "XFA dataset paths do not support document type declarations",
                ));
            }
            Event::End(element) => {
                let local_name = String::from_utf8(element.local_name().as_ref().to_vec())
                    .map_err(|_| {
                        PdfError::syntax("XFA XML element name is invalid", event_start)
                    })?;
                let frame = stack.pop().ok_or_else(|| {
                    PdfError::syntax(
                        "XFA dataset XML has an unexpected closing element",
                        event_start,
                    )
                })?;
                if frame.local_name != local_name {
                    return Err(PdfError::syntax(
                        "XFA dataset XML closing element does not match its opening element",
                        event_start,
                    ));
                }
                if frame.is_root {
                    if data_count != 1 {
                        return Err(PdfError::unsafe_rewrite(
                            "XFA datasets must contain exactly one data element",
                        ));
                    }
                    root_closed = true;
                    continue;
                }
                if let Some(path) = frame.path {
                    if frame.has_child {
                        if frame.non_whitespace_text || frame.unsupported_content {
                            return Err(PdfError::unsafe_rewrite(
                                "XFA dataset paths do not support mixed-content containers",
                            ));
                        }
                        layout.containers.insert(path);
                    } else {
                        if frame.unsupported_content {
                            return Err(PdfError::unsafe_rewrite(
                                "XFA dataset paths do not support comments or processing instructions in leaf values",
                            ));
                        }
                        if layout.fields.len() >= limits.max_container_items {
                            return Err(PdfError::limit(
                                "XFA dataset field count exceeds container limit",
                            ));
                        }
                        layout.fields.push(DatasetFieldSpan {
                            value: decode_dataset_value(input, frame.content_start, event_start)?,
                            path,
                            start: frame.start,
                            content_start: frame.content_start,
                            content_end: event_start,
                            end: event_end,
                            raw_name: frame.raw_name,
                            self_closing: false,
                        });
                    }
                }
            }
            Event::Eof => {
                if !stack.is_empty() {
                    return Err(PdfError::syntax(
                        "XFA dataset XML is truncated",
                        event_start,
                    ));
                }
                if !root_seen || !root_closed {
                    return Err(PdfError::unsafe_rewrite(
                        "XFA dataset paths require a complete datasets XML document",
                    ));
                }
                break;
            }
        }
    }
    Ok(layout)
}

fn xml_start_names(
    element: &quick_xml::events::BytesStart<'_>,
    offset: usize,
) -> Result<(String, String), PdfError> {
    let local_name = String::from_utf8(element.local_name().as_ref().to_vec())
        .map_err(|_| PdfError::syntax("XFA XML element name is invalid", offset))?;
    let raw_name = String::from_utf8(element.name().as_ref().to_vec())
        .map_err(|_| PdfError::syntax("XFA XML element name is invalid", offset))?;
    Ok((local_name, raw_name))
}

fn dataset_child_path(
    parent: Option<&str>,
    name: &str,
    limits: &crate::Limits,
) -> Result<String, PdfError> {
    let path = parent.map_or_else(|| name.to_owned(), |parent| format!("{parent}.{name}"));
    if path.len() > limits.max_token_bytes {
        return Err(PdfError::limit("XFA dataset path exceeds max_token_bytes"));
    }
    Ok(path)
}

fn decode_dataset_value(input: &[u8], start: usize, end: usize) -> Result<String, PdfError> {
    let raw = std::str::from_utf8(checked_dataset_span(input, start, end)?)
        .map_err(|_| PdfError::unsafe_rewrite("XFA dataset paths require UTF-8 XML"))?;
    unescape(raw)
        .map(|value| value.into_owned())
        .map_err(|_| PdfError::unsafe_rewrite("XFA dataset leaf contains an unsupported entity"))
}

fn packet_data(document: &PdfDocument) -> Result<Vec<PacketData>, PdfError> {
    let parsed = document.parsed();
    let Value::Dict(trailer) = &parsed.trailer else {
        return Err(PdfError::syntax("trailer must be a dictionary", 0));
    };
    let Some(root) = trailer.get(b"Root".as_slice()) else {
        return Ok(Vec::new());
    };
    let catalog = resolve_dict(parsed, root, "catalog")?;
    let Some(acroform) = catalog.get(b"AcroForm".as_slice()) else {
        return Ok(Vec::new());
    };
    let acroform = resolve_dict(parsed, acroform, "AcroForm")?;
    let Some(xfa) = acroform.get(b"XFA".as_slice()) else {
        return Ok(Vec::new());
    };
    let mut budget = ParseBudget::default();
    match xfa {
        Value::Ref(reference) => Ok(vec![read_packet(document, *reference, "xfa", &mut budget)?]),
        Value::Array(values) if values.len() % 2 == 0 => values
            .chunks_exact(2)
            .map(|pair| {
                let label = match &pair[0] {
                    Value::String(value) => String::from_utf8_lossy(value).into_owned(),
                    _ => return Err(PdfError::syntax("XFA packet label must be a string", 0)),
                };
                let Value::Ref(reference) = pair[1] else {
                    return Err(PdfError::unsupported(
                        "XFA packet must be an indirect stream",
                    ));
                };
                read_packet(document, reference, &label, &mut budget)
            })
            .collect(),
        Value::Array(_) => Err(PdfError::syntax(
            "XFA packet array must contain label/ref pairs",
            0,
        )),
        _ => Err(PdfError::unsupported(
            "XFA must be an indirect stream or packet array",
        )),
    }
}

fn read_packet(
    document: &PdfDocument,
    reference: ObjectRef,
    label: &str,
    budget: &mut ParseBudget,
) -> Result<PacketData, PdfError> {
    let object = document.parsed().object(reference)?;
    let stream = object
        .stream
        .as_deref()
        .ok_or_else(|| PdfError::unsupported("XFA packet must be a stream"))?;
    let bytes = decode_stream(&object.value, stream, &document.parsed().limits, budget)?;
    Ok(PacketData {
        label: label.into(),
        reference,
        bytes,
    })
}

fn resolve_dict<'a>(
    parsed: &'a crate::parser::ParsedDocument,
    value: &'a Value,
    label: &str,
) -> Result<&'a std::collections::BTreeMap<Vec<u8>, Value>, PdfError> {
    let value = match value {
        Value::Ref(reference) => &parsed.object(*reference)?.value,
        value => value,
    };
    match value {
        Value::Dict(dictionary) => Ok(dictionary),
        _ => Err(PdfError::syntax(format!("{label} must be a dictionary"), 0)),
    }
}

fn inspect_xml(bytes: &[u8]) -> Result<(Option<String>, bool, Vec<String>), PdfError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut root = None;
    let mut unsafe_xml = false;
    let mut stack = Vec::new();
    let mut markers = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let name = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
                root.get_or_insert_with(|| name.clone());
                if stack.iter().any(|ancestor| ancestor == "template") {
                    markers.extend(template_dynamic_markers(&element)?);
                }
                stack.push(name);
            }
            Ok(Event::Empty(element)) => {
                let name = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
                root.get_or_insert(name);
                if stack.iter().any(|ancestor| ancestor == "template") {
                    markers.extend(template_dynamic_markers(&element)?);
                }
            }
            Ok(Event::Text(text)) if stack.last().is_some_and(|name| name == "dynamicRender") => {
                let value = text
                    .decode()
                    .map_err(|_| PdfError::syntax("XFA XML text is invalid", 0))?;
                if !matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "" | "0" | "false" | "forbidden" | "static" | "none"
                ) {
                    markers.push(format!("dynamicRender={:?}", value.trim()));
                }
            }
            Ok(Event::End(element)) => {
                let name = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
                if stack.pop().as_deref() != Some(name.as_str()) {
                    return Err(PdfError::syntax("XFA XML elements are unbalanced", 0));
                }
            }
            Ok(Event::DocType(_)) => unsafe_xml = true,
            Ok(Event::Eof) if stack.is_empty() => break,
            Ok(Event::Eof) => return Err(PdfError::syntax("XFA XML is truncated", 0)),
            Ok(_) => {}
            Err(_) => return Err(PdfError::syntax("XFA packet is malformed XML", 0)),
        }
    }
    Ok((root, unsafe_xml, markers))
}

fn template_dynamic_markers(
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<Vec<String>, PdfError> {
    let name = element.local_name();
    let mut markers = Vec::new();
    if name.as_ref() == b"subform"
        && xfa_xml_attribute(element, b"layout")?
            .is_some_and(|value| value.eq_ignore_ascii_case("flowed"))
    {
        markers.push("template layout=\"flowed\"".into());
    }
    if name.as_ref() == b"occur" && xfa_occur_allows_dynamic_content(element)? {
        markers.push("template occur allows repeatable content".into());
    }
    if matches!(
        name.as_ref(),
        b"break" | b"breakBefore" | b"breakAfter" | b"overflow"
    ) {
        markers.push("template pagination/layout node requires XFA renderer semantics".into());
    }
    if let Some(presence) = xfa_xml_attribute(element, b"presence")?
        && !presence.eq_ignore_ascii_case("visible")
    {
        markers.push(format!("template presence={presence:?}"));
    }
    Ok(markers)
}

fn xfa_occur_allows_dynamic_content(
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<bool, PdfError> {
    for attribute in [b"max".as_slice(), b"initial", b"min"] {
        if let Some(value) = xfa_xml_attribute(element, attribute)?
            && value != "1"
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn xfa_xml_attribute(
    element: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
) -> Result<Option<String>, PdfError> {
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|_| PdfError::syntax("XFA XML attribute is malformed", 0))?;
        if attribute.key.local_name().as_ref() == key {
            return attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|value| Some(value.into_owned()))
                .map_err(|_| PdfError::syntax("XFA XML attribute is malformed", 0));
        }
    }
    Ok(None)
}

fn replace_all(input: &[u8], old: &[u8], new: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative) = input[cursor..]
        .windows(old.len())
        .position(|value| value == old)
    {
        let start = cursor + relative;
        output.extend_from_slice(&input[cursor..start]);
        output.extend_from_slice(new);
        cursor = start + old.len();
    }
    output.extend_from_slice(&input[cursor..]);
    output
}
