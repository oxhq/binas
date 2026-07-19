use std::collections::{BTreeMap, BTreeSet};

use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError,
    parser::{ObjectRef, ParsedDocument, Value},
    writer::{append_object_revisions, next_object_reference, refuse_security_boundaries},
    xfa::inspect_xfa_dynamic,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OcrTextBox {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OcrTextLayerRequest {
    pub page_index: usize,
    pub source_width: f64,
    pub source_height: f64,
    pub boxes: Vec<OcrTextBox>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OcrParseLimits {
    pub max_input_bytes: usize,
    pub max_boxes: usize,
    pub max_text_bytes: usize,
}

impl Default for OcrParseLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_boxes: 100_000,
            max_text_bytes: 8 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OcrPlacedText {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OcrTextLayerPlan {
    pub page_index: usize,
    pub page_object_number: u32,
    pub page_object_generation: u16,
    pub source_sha256: String,
    pub boxes: Vec<OcrPlacedText>,
    content_stream: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OcrTextLayerVerification {
    pub passed: bool,
    pub page_count_unchanged: bool,
    pub text_selectable: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OcrTextLayerReport {
    pub operation: String,
    pub page_index: usize,
    pub boxes_placed: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OcrTextLayerOutcome {
    pub bytes: Vec<u8>,
    pub report: OcrTextLayerReport,
    pub verification: OcrTextLayerVerification,
}

impl PdfDocument {
    pub fn plan_ocr_text_layer(
        &self,
        request: OcrTextLayerRequest,
    ) -> Result<OcrTextLayerPlan, PdfError> {
        refuse_ocr_boundaries(self)?;
        validate_request(&request, &self.engine_config().limits)?;
        let page = self
            .page_refs()?
            .get(request.page_index)
            .copied()
            .ok_or_else(|| PdfError::selection("OCR page index is out of range"))?;
        let (media, rotation) = inherited_page_geometry(self.parsed(), page)?;
        if rotation != 0 {
            return Err(PdfError::unsupported(
                "OCR text layers currently require an unrotated page",
            ));
        }
        let page_width = media[2] - media[0];
        let page_height = media[3] - media[1];
        let scale_x = page_width / request.source_width;
        let scale_y = page_height / request.source_height;
        let boxes = request
            .boxes
            .into_iter()
            .map(|value| OcrPlacedText {
                text: value.text,
                x: media[0] + value.x * scale_x,
                y: media[3] - (value.y + value.height) * scale_y,
                width: value.width * scale_x,
                height: value.height * scale_y,
            })
            .collect::<Vec<_>>();
        let content_stream = content_stream(&boxes)?;
        Ok(OcrTextLayerPlan {
            page_index: request.page_index,
            page_object_number: page.number,
            page_object_generation: page.generation,
            source_sha256: hex(&Sha256::digest(self.source())),
            boxes,
            content_stream,
        })
    }

    pub fn apply_ocr_text_layer(
        &self,
        plan: &OcrTextLayerPlan,
    ) -> Result<OcrTextLayerOutcome, PdfError> {
        refuse_ocr_boundaries(self)?;
        if plan.source_sha256 != hex(&Sha256::digest(self.source())) {
            return Err(PdfError::verification(
                "OCR plan was prepared for a different PDF",
            ));
        }
        let page = ObjectRef {
            number: plan.page_object_number,
            generation: plan.page_object_generation,
        };
        if self.page_refs()?.get(plan.page_index) != Some(&page) {
            return Err(PdfError::verification("OCR plan page no longer matches"));
        }
        if content_stream(&plan.boxes)? != plan.content_stream {
            return Err(PdfError::verification("OCR plan content was modified"));
        }
        let font = next_object_reference(self)?;
        let content = next_reference_after(self, font)?;
        let font_name = format!("BinasOcr{}", font.number).into_bytes();
        let page_object = self.parsed().object(page)?;
        let Value::Dict(mut page_dictionary) = page_object.value.clone() else {
            return Err(PdfError::unsafe_rewrite("OCR page is not a dictionary"));
        };
        let mut resources = inherited_page_resources(self.parsed(), page)?;
        let mut fonts = match resources.remove(b"Font".as_slice()) {
            None => BTreeMap::new(),
            Some(value) => resolve_dictionary(self.parsed(), &value, "page /Resources /Font")?,
        };
        fonts.insert(font_name.clone(), Value::Ref(font));
        resources.insert(b"Font".to_vec(), Value::Dict(fonts));
        page_dictionary.insert(b"Resources".to_vec(), Value::Dict(resources));
        append_page_content(&mut page_dictionary, content)?;
        let font_dictionary = Value::Dict(BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"Font".to_vec())),
            (b"Subtype".to_vec(), Value::Name(b"Type1".to_vec())),
            (b"BaseFont".to_vec(), Value::Name(b"Helvetica".to_vec())),
            (
                b"Encoding".to_vec(),
                Value::Name(b"WinAnsiEncoding".to_vec()),
            ),
        ]));
        let stream = plan.content_stream.replace_font_name(&font_name)?;
        let bytes = append_object_revisions(
            self,
            &[
                (page, &Value::Dict(page_dictionary), None),
                (font, &font_dictionary, None),
                (
                    content,
                    &Value::Dict(BTreeMap::new()),
                    Some(stream.as_slice()),
                ),
            ],
        )?;
        let rewritten =
            PdfEngine::new(self.engine_config().clone()).open(&bytes, OpenOptions::default())?;
        let page_count_unchanged = rewritten.page_count()? == self.page_count()?;
        let text_selectable = plan.boxes.iter().all(|value| {
            rewritten
                .query_text_all(&value.text)
                .is_ok_and(|matches| !matches.is_empty())
        });
        let no_dangling_references = references_resolve(rewritten.parsed())?;
        let verification = OcrTextLayerVerification {
            passed: page_count_unchanged && text_selectable && no_dangling_references,
            page_count_unchanged,
            text_selectable,
            no_dangling_references,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "OCR text layer failed post-write verification",
            ));
        }
        Ok(OcrTextLayerOutcome {
            report: OcrTextLayerReport {
                operation: "apply_ocr_text_layer".into(),
                page_index: plan.page_index,
                boxes_placed: plan.boxes.len(),
                input_bytes: self.source_len(),
                output_bytes: bytes.len(),
            },
            bytes,
            verification,
        })
    }
}

trait ReplaceFontName {
    fn replace_font_name(&self, name: &[u8]) -> Result<Vec<u8>, PdfError>;
}

impl ReplaceFontName for Vec<u8> {
    fn replace_font_name(&self, name: &[u8]) -> Result<Vec<u8>, PdfError> {
        let marker = b"BinasOcrFont";
        let mut output = Vec::with_capacity(self.len() + name.len());
        let mut rest = self.as_slice();
        while let Some(position) = rest.windows(marker.len()).position(|value| value == marker) {
            output.extend_from_slice(&rest[..position]);
            output.extend_from_slice(name);
            rest = &rest[position + marker.len()..];
        }
        output.extend_from_slice(rest);
        Ok(output)
    }
}

pub fn parse_ocr_json(
    input: &[u8],
    limits: OcrParseLimits,
) -> Result<OcrTextLayerRequest, PdfError> {
    require_input_limit(input, limits)?;
    let request: OcrTextLayerRequest = serde_json::from_slice(input)
        .map_err(|error| PdfError::syntax(format!("invalid OCR JSON: {error}"), 0))?;
    validate_parse_limits(std::slice::from_ref(&request), limits)?;
    Ok(request)
}

pub fn parse_alto_xml(
    input: &[u8],
    limits: OcrParseLimits,
) -> Result<Vec<OcrTextLayerRequest>, PdfError> {
    require_input_limit(input, limits)?;
    let mut reader = Reader::from_reader(input);
    reader.config_mut().trim_text(true);
    let mut pages = Vec::new();
    let mut current: Option<OcrTextLayerRequest> = None;
    loop {
        match reader.read_event() {
            Ok(Event::DocType(_)) => {
                return Err(PdfError::syntax("ALTO document types are not allowed", 0));
            }
            Ok(Event::Start(event)) | Ok(Event::Empty(event))
                if event.local_name().as_ref() == b"Page" =>
            {
                if let Some(page) = current.take() {
                    pages.push(page);
                }
                current = Some(OcrTextLayerRequest {
                    page_index: pages.len(),
                    source_width: required_attribute(&event, b"WIDTH")?,
                    source_height: required_attribute(&event, b"HEIGHT")?,
                    boxes: Vec::new(),
                });
            }
            Ok(Event::Start(event)) | Ok(Event::Empty(event))
                if event.local_name().as_ref() == b"String" =>
            {
                let page = current
                    .as_mut()
                    .ok_or_else(|| PdfError::syntax("ALTO String appears outside Page", 0))?;
                if page.boxes.len() >= limits.max_boxes {
                    return Err(PdfError::limit("ALTO OCR box count exceeds limit"));
                }
                page.boxes.push(OcrTextBox {
                    text: required_text_attribute(&event, b"CONTENT")?,
                    x: required_attribute(&event, b"HPOS")?,
                    y: required_attribute(&event, b"VPOS")?,
                    width: required_attribute(&event, b"WIDTH")?,
                    height: required_attribute(&event, b"HEIGHT")?,
                });
            }
            Ok(Event::End(event)) if event.local_name().as_ref() == b"Page" => {
                if let Some(page) = current.take() {
                    pages.push(page);
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(PdfError::syntax(format!("invalid ALTO XML: {error}"), 0)),
            _ => {}
        }
    }
    if let Some(page) = current {
        pages.push(page);
    }
    if pages.is_empty() {
        return Err(PdfError::syntax("ALTO XML contains no Page", 0));
    }
    validate_parse_limits(&pages, limits)?;
    Ok(pages)
}

fn content_stream(boxes: &[OcrPlacedText]) -> Result<Vec<u8>, PdfError> {
    let mut output = Vec::new();
    for value in boxes {
        let size = value.height.max(0.1);
        let estimated_width = size * value.text.len().max(1) as f64 * 0.5;
        let horizontal_scale = (value.width / estimated_width * 100.0).clamp(1.0, 1000.0);
        output.extend_from_slice(
            format!(
                "BT /BinasOcrFont {size} Tf 3 Tr {horizontal_scale} Tz 1 0 0 1 {} {} Tm ({}) Tj ET\n",
                value.x,
                value.y,
                escape_literal(&value.text)
            )
            .as_bytes(),
        );
    }
    Ok(output)
}

fn validate_request(request: &OcrTextLayerRequest, limits: &crate::Limits) -> Result<(), PdfError> {
    if !request.source_width.is_finite()
        || !request.source_height.is_finite()
        || request.source_width <= 0.0
        || request.source_height <= 0.0
    {
        return Err(PdfError::unsafe_rewrite(
            "OCR source dimensions must be positive and finite",
        ));
    }
    if request.boxes.is_empty() || request.boxes.len() > limits.max_container_items {
        return Err(PdfError::limit("OCR box count is empty or exceeds limit"));
    }
    for value in &request.boxes {
        if value.text.is_empty()
            || !value.text.is_ascii()
            || value.text.len() > limits.max_token_bytes
            || [value.x, value.y, value.width, value.height]
                .iter()
                .any(|number| !number.is_finite())
            || value.x < 0.0
            || value.y < 0.0
            || value.width <= 0.0
            || value.height <= 0.0
            || value.x + value.width > request.source_width
            || value.y + value.height > request.source_height
        {
            return Err(PdfError::unsafe_rewrite(
                "OCR boxes require bounded positive coordinates and non-empty ASCII text",
            ));
        }
    }
    Ok(())
}

fn validate_parse_limits(
    requests: &[OcrTextLayerRequest],
    limits: OcrParseLimits,
) -> Result<(), PdfError> {
    let boxes = requests.iter().try_fold(0usize, |count, request| {
        count.checked_add(request.boxes.len())
    });
    let text = requests
        .iter()
        .flat_map(|request| &request.boxes)
        .try_fold(0usize, |count, value| count.checked_add(value.text.len()));
    if boxes.is_none_or(|count| count > limits.max_boxes)
        || text.is_none_or(|count| count > limits.max_text_bytes)
    {
        return Err(PdfError::limit("OCR parsed content exceeds limits"));
    }
    if requests.iter().any(|request| {
        !request.source_width.is_finite()
            || !request.source_height.is_finite()
            || request.source_width <= 0.0
            || request.source_height <= 0.0
            || request.boxes.iter().any(|value| {
                [value.x, value.y, value.width, value.height]
                    .iter()
                    .any(|number| !number.is_finite())
                    || value.x < 0.0
                    || value.y < 0.0
                    || value.width <= 0.0
                    || value.height <= 0.0
                    || value.x + value.width > request.source_width
                    || value.y + value.height > request.source_height
            })
    }) {
        return Err(PdfError::syntax("OCR parsed geometry is invalid", 0));
    }
    Ok(())
}

fn require_input_limit(input: &[u8], limits: OcrParseLimits) -> Result<(), PdfError> {
    if input.len() > limits.max_input_bytes {
        Err(PdfError::limit("OCR input exceeds max_input_bytes"))
    } else {
        Ok(())
    }
}

fn required_attribute(
    event: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
) -> Result<f64, PdfError> {
    required_text_attribute(event, key)?
        .parse()
        .map_err(|_| PdfError::syntax("ALTO numeric attribute is invalid", 0))
}

fn required_text_attribute(
    event: &quick_xml::events::BytesStart<'_>,
    key: &[u8],
) -> Result<String, PdfError> {
    for attribute in event.attributes().with_checks(false) {
        let attribute =
            attribute.map_err(|_| PdfError::syntax("ALTO attribute is malformed", 0))?;
        if attribute.key.local_name().as_ref() == key {
            return attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|value| value.into_owned())
                .map_err(|_| PdfError::syntax("ALTO attribute is malformed", 0));
        }
    }
    Err(PdfError::syntax("ALTO required attribute is missing", 0))
}

fn refuse_ocr_boundaries(document: &PdfDocument) -> Result<(), PdfError> {
    refuse_security_boundaries(document.parsed())?;
    if inspect_xfa_dynamic(document)?.dynamic {
        return Err(PdfError::unsafe_rewrite(
            "OCR text layers refuse dynamic XFA PDFs",
        ));
    }
    Ok(())
}

fn next_reference_after(document: &PdfDocument, first: ObjectRef) -> Result<ObjectRef, PdfError> {
    let number = first
        .number
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("OCR object number overflows"))?;
    if document.parsed().objects.len() + 2 > document.parsed().limits.max_objects
        || usize::try_from(number)
            .ok()
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > document.parsed().limits.max_xref_entries)
    {
        return Err(PdfError::limit("OCR object allocation exceeds limits"));
    }
    Ok(ObjectRef {
        number,
        generation: 0,
    })
}

fn append_page_content(
    page: &mut BTreeMap<Vec<u8>, Value>,
    content: ObjectRef,
) -> Result<(), PdfError> {
    match page.remove(b"Contents".as_slice()) {
        None => page.insert(b"Contents".to_vec(), Value::Ref(content)),
        Some(Value::Ref(reference)) => page.insert(
            b"Contents".to_vec(),
            Value::Array(vec![Value::Ref(reference), Value::Ref(content)]),
        ),
        Some(Value::Array(mut values))
            if values.iter().all(|value| matches!(value, Value::Ref(_))) =>
        {
            values.push(Value::Ref(content));
            page.insert(b"Contents".to_vec(), Value::Array(values))
        }
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "page /Contents must contain indirect streams",
            ));
        }
    };
    Ok(())
}

fn inherited_page_resources(
    parsed: &ParsedDocument,
    page: ObjectRef,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    let mut current = page;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(current) || seen.len() > parsed.limits.max_parser_depth {
            return Err(PdfError::limit("page resource inheritance exceeds limit"));
        }
        let Value::Dict(dictionary) = &parsed.object(current)?.value else {
            return Err(PdfError::unsafe_rewrite(
                "page resource owner is not a dictionary",
            ));
        };
        if let Some(value) = dictionary.get(b"Resources".as_slice()) {
            return resolve_dictionary(parsed, value, "page /Resources");
        }
        current = match dictionary.get(b"Parent".as_slice()) {
            Some(Value::Ref(reference)) => *reference,
            _ => return Ok(BTreeMap::new()),
        };
    }
}

fn resolve_dictionary(
    parsed: &ParsedDocument,
    value: &Value,
    label: &str,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(dictionary) => Ok(dictionary.clone()),
        Value::Ref(reference) => match &parsed.object(*reference)?.value {
            Value::Dict(dictionary) => Ok(dictionary.clone()),
            _ => Err(PdfError::unsafe_rewrite(format!(
                "{label} is not a dictionary"
            ))),
        },
        _ => Err(PdfError::unsafe_rewrite(format!(
            "{label} is not a dictionary"
        ))),
    }
}

fn inherited_page_geometry(
    parsed: &ParsedDocument,
    page: ObjectRef,
) -> Result<([f64; 4], i64), PdfError> {
    let mut current = page;
    let mut seen = BTreeSet::new();
    let mut media = None;
    let mut rotation = None;
    loop {
        if !seen.insert(current) || seen.len() > parsed.limits.max_parser_depth {
            return Err(PdfError::limit("page geometry inheritance exceeds limit"));
        }
        let Value::Dict(dictionary) = &parsed.object(current)?.value else {
            return Err(PdfError::unsafe_rewrite(
                "page geometry owner is not a dictionary",
            ));
        };
        media = media.or_else(|| dictionary.get(b"MediaBox".as_slice()).and_then(rectangle));
        rotation = rotation.or_else(|| match dictionary.get(b"Rotate".as_slice()) {
            Some(Value::Integer(value)) => Some(value.rem_euclid(360)),
            _ => None,
        });
        if media.is_some() && rotation.is_some() {
            break;
        }
        current = match dictionary.get(b"Parent".as_slice()) {
            Some(Value::Ref(reference)) => *reference,
            _ => break,
        };
    }
    let media = media.ok_or_else(|| PdfError::unsafe_rewrite("page has no inherited MediaBox"))?;
    if media[0] >= media[2] || media[1] >= media[3] {
        return Err(PdfError::unsafe_rewrite("page MediaBox is invalid"));
    }
    Ok((media, rotation.unwrap_or(0)))
}

fn rectangle(value: &Value) -> Option<[f64; 4]> {
    let Value::Array(values) = value else {
        return None;
    };
    values
        .iter()
        .map(|value| match value {
            Value::Integer(value) => Some(*value as f64),
            Value::Real(value) => Some(*value),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()
}

fn references_resolve(parsed: &ParsedDocument) -> Result<bool, PdfError> {
    fn visit(value: &Value, parsed: &ParsedDocument, depth: usize) -> Result<bool, PdfError> {
        if depth > parsed.limits.max_parser_depth {
            return Err(PdfError::limit(
                "OCR reference validation exceeds depth limit",
            ));
        }
        match value {
            Value::Ref(reference) => Ok(parsed.objects.contains_key(reference)),
            Value::Array(values) => {
                for value in values {
                    if !visit(value, parsed, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Value::Dict(dictionary) => {
                for value in dictionary.values() {
                    if !visit(value, parsed, depth + 1)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(true),
        }
    }
    if !visit(&parsed.trailer, parsed, 0)? {
        return Ok(false);
    }
    for object in parsed.objects.values() {
        if !visit(&object.value, parsed, 0)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn escape_literal(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'(' | b')' | b'\\' => vec![b'\\', byte],
            byte => vec![byte],
        })
        .map(char::from)
        .collect()
}

fn hex(input: &[u8]) -> String {
    input.iter().map(|byte| format!("{byte:02x}")).collect()
}
