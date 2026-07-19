use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError, content,
    parser::{IndirectObject, ObjectRef, ParsedDocument, Value},
    writer::refuse_security_boundaries,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OverlayStampRequest {
    pub page_indices: Vec<usize>,
    pub form_content: Vec<u8>,
    pub bbox: [f64; 4],
    pub transform: [f64; 6],
    pub opacity: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextOverlayRequest {
    pub page_index: usize,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub font_size: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OverlayStampReport {
    pub operation: String,
    pub pages_stamped: usize,
    pub form_object_number: u32,
    pub resource_names: Vec<String>,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextOverlayReport {
    pub operation: String,
    pub page_index: usize,
    pub form_object_number: u32,
    pub font_object_number: u32,
    pub resource_name: String,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OverlayStampVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub form_stream_matches: bool,
    pub placements_match: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextOverlayVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub form_stream_matches: bool,
    pub placements_match: bool,
    pub font_resource_matches: bool,
    pub text_selectable: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OverlayStampOutcome {
    pub bytes: Vec<u8>,
    pub report: OverlayStampReport,
    pub verification: OverlayStampVerification,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextOverlayOutcome {
    pub bytes: Vec<u8>,
    pub report: TextOverlayReport,
    pub verification: TextOverlayVerification,
}

struct FormPlacement {
    bytes: Vec<u8>,
    resource_names: Vec<String>,
    verification: OverlayStampVerification,
    reopened: PdfDocument,
}

struct FormDefinition {
    dictionary: BTreeMap<Vec<u8>, Value>,
    stream: Vec<u8>,
}

struct FormPlacementRequest<'a> {
    pages: &'a [ObjectRef],
    selected: &'a [ObjectRef],
    form_ref: ObjectRef,
    form_stream: &'a [u8],
    transform: [f64; 6],
    next: &'a mut u32,
}

impl PdfDocument {
    pub fn place_overlay_stamp(
        &self,
        request: OverlayStampRequest,
    ) -> Result<OverlayStampOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        validate_request(self, &request)?;
        let pages = self.page_refs()?;
        let selected = selected_pages(&pages, &request.page_indices)?;
        let mut parsed = self.parsed().clone();
        let mut next = next_object_number(&parsed)?;
        ensure_capacity(&parsed, next, selected.len() + 1)?;
        let form_ref = allocate(&mut next)?;
        let form = form(&request);
        parsed.objects.insert(
            form_ref,
            IndirectObject {
                value: Value::Dict(form.dictionary),
                stream: Some(form.stream.clone()),
                stream_offset: 0,
                offset: 0,
            },
        );
        let placement = place_form(
            self,
            parsed,
            FormPlacementRequest {
                pages: &pages,
                selected: &selected,
                form_ref,
                form_stream: &form.stream,
                transform: request.transform,
                next: &mut next,
            },
        )?;
        let output_bytes = placement.bytes.len();
        Ok(OverlayStampOutcome {
            report: OverlayStampReport {
                operation: "place_overlay_stamp".into(),
                pages_stamped: request.page_indices.len(),
                form_object_number: form_ref.number,
                resource_names: placement.resource_names,
                input_bytes: self.source_len(),
                output_bytes,
            },
            bytes: placement.bytes,
            verification: placement.verification,
        })
    }

    pub fn place_text_overlay(
        &self,
        request: TextOverlayRequest,
    ) -> Result<TextOverlayOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        validate_text_request(self, &request)?;
        let pages = self.page_refs()?;
        let selected = selected_pages(&pages, &[request.page_index])?;
        let mut parsed = self.parsed().clone();
        let mut next = next_object_number(&parsed)?;
        ensure_capacity(&parsed, next, selected.len() + 2)?;
        let font_ref = allocate(&mut next)?;
        let form_ref = allocate(&mut next)?;
        let form = text_form(&request, font_ref)?;
        if form.stream.len() > self.parsed().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "text overlay Form content exceeds stream limit",
            ));
        }
        parsed.objects.insert(font_ref, standard_helvetica_font());
        parsed.objects.insert(
            form_ref,
            IndirectObject {
                value: Value::Dict(form.dictionary),
                stream: Some(form.stream.clone()),
                stream_offset: 0,
                offset: 0,
            },
        );
        let placement = place_form(
            self,
            parsed,
            FormPlacementRequest {
                pages: &pages,
                selected: &selected,
                form_ref,
                form_stream: &form.stream,
                transform: [1.0, 0.0, 0.0, 1.0, request.x, request.y],
                next: &mut next,
            },
        )?;
        let font_resource_matches =
            text_font_matches(placement.reopened.parsed(), form_ref, font_ref)?;
        let text_selectable =
            text_is_selectable(placement.reopened.parsed(), form_ref, &request.text)?;
        let base = &placement.verification;
        let verification = TextOverlayVerification {
            passed: base.passed && font_resource_matches && text_selectable,
            reparsed: base.reparsed,
            page_count_unchanged: base.page_count_unchanged,
            form_stream_matches: base.form_stream_matches,
            placements_match: base.placements_match,
            font_resource_matches,
            text_selectable,
            no_dangling_references: base.no_dangling_references,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "text overlay failed post-write verification",
            ));
        }
        let resource_name = placement.resource_names.into_iter().next().ok_or_else(|| {
            PdfError::verification("text overlay did not produce a page resource name")
        })?;
        let output_bytes = placement.bytes.len();
        Ok(TextOverlayOutcome {
            report: TextOverlayReport {
                operation: "place_text_overlay".into(),
                page_index: request.page_index,
                form_object_number: form_ref.number,
                font_object_number: font_ref.number,
                resource_name,
                input_bytes: self.source_len(),
                output_bytes,
            },
            bytes: placement.bytes,
            verification,
        })
    }
}

fn place_form(
    document: &PdfDocument,
    mut parsed: ParsedDocument,
    request: FormPlacementRequest<'_>,
) -> Result<FormPlacement, PdfError> {
    let mut placements = Vec::with_capacity(request.selected.len());
    let mut resource_names = Vec::with_capacity(request.selected.len());
    for &page_ref in request.selected {
        let content_ref = allocate(request.next)?;
        let mut resources = inherited_page_resources(&parsed, page_ref)?;
        let mut xobjects = match resources.remove(b"XObject".as_slice()) {
            None => BTreeMap::new(),
            Some(value) => resolve_dictionary(&parsed, &value, "page /Resources /XObject")?,
        };
        let name = available_resource_name(&xobjects);
        xobjects.insert(name.clone(), Value::Ref(request.form_ref));
        resources.insert(b"XObject".to_vec(), Value::Dict(xobjects));
        let command = placement_command(&name, request.transform);
        let page = parsed
            .objects
            .get_mut(&page_ref)
            .ok_or_else(|| PdfError::syntax("selected overlay page is missing", 0))?;
        let dictionary = dictionary_mut(&mut page.value, "overlay page")?;
        dictionary.insert(b"Resources".to_vec(), Value::Dict(resources));
        append_contents(dictionary, content_ref)?;
        parsed
            .objects
            .insert(content_ref, stream_object(command.clone()));
        resource_names.push(
            String::from_utf8(name).map_err(|_| {
                PdfError::verification("generated overlay resource name is not UTF-8")
            })?,
        );
        placements.push((page_ref, content_ref, command));
    }
    let canonical = document.with_parsed(parsed).canonicalize()?;
    let reopened = PdfEngine::new(document.engine_config().clone())
        .open(&canonical.bytes, OpenOptions::default())
        .map_err(|error| {
            PdfError::verification(format!("overlay output did not reparse: {error}"))
        })?;
    let page_count_unchanged = reopened.page_count()? == request.pages.len();
    let form_stream_matches = reopened
        .parsed()
        .object(request.form_ref)?
        .stream
        .as_deref()
        == Some(request.form_stream);
    let placements_match = placements.iter().all(|(page, content, command)| {
        placement_matches(
            reopened.parsed(),
            *page,
            *content,
            command,
            request.form_ref,
        )
    });
    let no_dangling_references = verify_references(reopened.parsed())?;
    let verification = OverlayStampVerification {
        passed: page_count_unchanged
            && form_stream_matches
            && placements_match
            && no_dangling_references,
        reparsed: true,
        page_count_unchanged,
        form_stream_matches,
        placements_match,
        no_dangling_references,
    };
    if !verification.passed {
        return Err(PdfError::verification(
            "overlay stamp failed post-write verification",
        ));
    }
    Ok(FormPlacement {
        bytes: canonical.bytes,
        resource_names,
        verification,
        reopened,
    })
}

fn validate_request(document: &PdfDocument, request: &OverlayStampRequest) -> Result<(), PdfError> {
    if request.page_indices.is_empty() {
        return Err(PdfError::selection("overlay requires at least one page"));
    }
    if request.form_content.is_empty() {
        return Err(PdfError::unsupported(
            "overlay Form content must not be empty",
        ));
    }
    if request.form_content.len() > document.parsed().limits.max_stream_bytes {
        return Err(PdfError::limit("overlay Form content exceeds stream limit"));
    }
    validate_rectangle(request.bbox, "overlay bbox")?;
    if request.transform.iter().any(|value| !value.is_finite()) {
        return Err(PdfError::unsupported("overlay transform must be finite"));
    }
    let determinant =
        request.transform[0] * request.transform[3] - request.transform[1] * request.transform[2];
    if !determinant.is_finite() || determinant == 0.0 {
        return Err(PdfError::unsupported(
            "overlay transform must be invertible",
        ));
    }
    if let Some(opacity) = request.opacity
        && (!opacity.is_finite() || !(0.0..=1.0).contains(&opacity))
    {
        return Err(PdfError::unsupported(
            "overlay opacity must be between zero and one",
        ));
    }
    if request.form_content.contains(&b'/') {
        return Err(PdfError::unsupported(
            "overlay Form content must be resource-free; name operands are not supported",
        ));
    }
    if !content::extract_text_show(&request.form_content, 0, &document.parsed().limits)?.is_empty()
    {
        return Err(PdfError::unsupported(
            "overlay Form text requires font resources and is not supported",
        ));
    }
    Ok(())
}

fn validate_text_request(
    document: &PdfDocument,
    request: &TextOverlayRequest,
) -> Result<(), PdfError> {
    if request.text.is_empty() {
        return Err(PdfError::unsupported("text overlay text must not be empty"));
    }
    if request.text.len() > document.parsed().limits.max_stream_bytes {
        return Err(PdfError::limit("text overlay text exceeds stream limit"));
    }
    if request
        .text
        .bytes()
        .any(|byte| !matches!(byte, 0x20..=0x7e))
    {
        return Err(PdfError::unsupported(
            "text overlay supports printable ASCII with the built-in Helvetica font",
        ));
    }
    if !request.x.is_finite() || !request.y.is_finite() {
        return Err(PdfError::unsupported(
            "text overlay coordinates must be finite",
        ));
    }
    if !request.font_size.is_finite() || request.font_size <= 0.0 {
        return Err(PdfError::unsupported(
            "text overlay font size must be finite and positive",
        ));
    }
    let width = request.font_size * request.text.len() as f64;
    validate_rectangle(
        [0.0, -request.font_size, width, request.font_size],
        "text overlay bbox",
    )
}

fn form(request: &OverlayStampRequest) -> FormDefinition {
    let mut resources = BTreeMap::new();
    let mut stream = request.form_content.clone();
    if let Some(opacity) = request.opacity {
        let state = Value::Dict(BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"ExtGState".to_vec())),
            (b"ca".to_vec(), Value::Real(opacity)),
            (b"CA".to_vec(), Value::Real(opacity)),
        ]));
        resources.insert(
            b"ExtGState".to_vec(),
            Value::Dict(BTreeMap::from([(b"GS0".to_vec(), state)])),
        );
        let mut with_opacity = b"q /GS0 gs\n".to_vec();
        with_opacity.extend_from_slice(&stream);
        with_opacity.extend_from_slice(b"\nQ");
        stream = with_opacity;
    }
    FormDefinition {
        dictionary: BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Value::Name(b"Form".to_vec())),
            (b"FormType".to_vec(), Value::Integer(1)),
            (b"BBox".to_vec(), rectangle_value(request.bbox)),
            (b"Resources".to_vec(), Value::Dict(resources)),
        ]),
        stream,
    }
}

fn text_form(
    request: &TextOverlayRequest,
    font_ref: ObjectRef,
) -> Result<FormDefinition, PdfError> {
    let width = request.font_size * request.text.len() as f64;
    let bbox = [0.0, -request.font_size, width, request.font_size];
    validate_rectangle(bbox, "text overlay bbox")?;
    let resources = Value::Dict(BTreeMap::from([(
        b"Font".to_vec(),
        Value::Dict(BTreeMap::from([(
            b"BinasTextFont".to_vec(),
            Value::Ref(font_ref),
        )])),
    )]));
    let stream = format!(
        "q BT /BinasTextFont {} Tf 0 0 Td ({}) Tj ET Q\n",
        number(request.font_size),
        escape_literal(&request.text)
    )
    .into_bytes();
    Ok(FormDefinition {
        dictionary: BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Value::Name(b"Form".to_vec())),
            (b"FormType".to_vec(), Value::Integer(1)),
            (b"BBox".to_vec(), rectangle_value(bbox)),
            (b"Resources".to_vec(), resources),
        ]),
        stream,
    })
}

fn standard_helvetica_font() -> IndirectObject {
    IndirectObject {
        value: Value::Dict(BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"Font".to_vec())),
            (b"Subtype".to_vec(), Value::Name(b"Type1".to_vec())),
            (b"BaseFont".to_vec(), Value::Name(b"Helvetica".to_vec())),
            (
                b"Encoding".to_vec(),
                Value::Name(b"WinAnsiEncoding".to_vec()),
            ),
        ])),
        stream: None,
        stream_offset: 0,
        offset: 0,
    }
}

fn escape_literal(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'(' | b')' | b'\\' => vec![b'\\', byte],
            _ => vec![byte],
        })
        .map(char::from)
        .collect()
}

fn text_font_matches(
    parsed: &ParsedDocument,
    form_ref: ObjectRef,
    font_ref: ObjectRef,
) -> Result<bool, PdfError> {
    let form = parsed.object(form_ref)?;
    let form_dictionary = dictionary(&form.value, "verified text overlay Form")?;
    let Some(resources) = form_dictionary.get(b"Resources".as_slice()) else {
        return Ok(false);
    };
    let resources = resolve_dictionary(parsed, resources, "verified text overlay resources")?;
    let Some(fonts) = resources.get(b"Font".as_slice()) else {
        return Ok(false);
    };
    let fonts = resolve_dictionary(parsed, fonts, "verified text overlay fonts")?;
    if fonts.get(b"BinasTextFont".as_slice()) != Some(&Value::Ref(font_ref)) {
        return Ok(false);
    }
    let font = dictionary(
        &parsed.object(font_ref)?.value,
        "verified text overlay font",
    )?;
    Ok(
        matches!(font.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Font")
            && matches!(font.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Type1")
            && matches!(font.get(b"BaseFont".as_slice()), Some(Value::Name(name)) if name == b"Helvetica")
            && matches!(font.get(b"Encoding".as_slice()), Some(Value::Name(name)) if name == b"WinAnsiEncoding"),
    )
}

fn text_is_selectable(
    parsed: &ParsedDocument,
    form_ref: ObjectRef,
    expected_text: &str,
) -> Result<bool, PdfError> {
    let form = parsed.object(form_ref)?;
    let stream = form
        .stream
        .as_deref()
        .ok_or_else(|| PdfError::verification("text overlay Form stream is missing"))?;
    Ok(content::extract_text_show(stream, 0, &parsed.limits)?
        .iter()
        .any(|item| {
            item.text == expected_text && item.font.as_deref() == Some(b"BinasTextFont".as_slice())
        }))
}

fn selected_pages(pages: &[ObjectRef], indices: &[usize]) -> Result<Vec<ObjectRef>, PdfError> {
    let mut seen = BTreeSet::new();
    let mut selected = Vec::with_capacity(indices.len());
    for &index in indices {
        let page = pages.get(index).copied().ok_or_else(|| {
            PdfError::selection(format!(
                "overlay page index {index} exceeds page count {}",
                pages.len()
            ))
        })?;
        if !seen.insert(index) {
            return Err(PdfError::unsupported(
                "overlay page selection cannot contain duplicates",
            ));
        }
        selected.push(page);
    }
    Ok(selected)
}

fn inherited_page_resources(
    parsed: &ParsedDocument,
    page: ObjectRef,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    let mut current = page;
    let mut seen = BTreeSet::new();
    for _ in 0..=parsed.limits.max_parser_depth {
        if !seen.insert(current) {
            return Err(PdfError::syntax("cycle in page resource inheritance", 0));
        }
        let dictionary = dictionary(&parsed.object(current)?.value, "page resource owner")?;
        if let Some(value) = dictionary.get(b"Resources".as_slice()) {
            return resolve_dictionary(parsed, value, "page /Resources");
        }
        match dictionary.get(b"Parent".as_slice()) {
            Some(Value::Ref(parent)) => current = *parent,
            None => return Ok(BTreeMap::new()),
            Some(_) => return Err(PdfError::syntax("page /Parent is not a reference", 0)),
        }
    }
    Err(PdfError::limit("page resource inheritance exceeds limit"))
}

fn resolve_dictionary(
    parsed: &ParsedDocument,
    value: &Value,
    label: &str,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    let mut value = value;
    let mut seen = BTreeSet::new();
    for _ in 0..=parsed.limits.max_parser_depth {
        match value {
            Value::Dict(dictionary) => return Ok(dictionary.clone()),
            Value::Ref(reference) => {
                if !seen.insert(*reference) {
                    return Err(PdfError::syntax(format!("cycle in {label}"), 0));
                }
                value = &parsed.object(*reference)?.value;
            }
            _ => {
                return Err(PdfError::unsupported(format!(
                    "{label} is not a dictionary"
                )));
            }
        }
    }
    Err(PdfError::limit(format!(
        "{label} indirection exceeds limit"
    )))
}

fn available_resource_name(xobjects: &BTreeMap<Vec<u8>, Value>) -> Vec<u8> {
    for suffix in 0_u32.. {
        let name = if suffix == 0 {
            b"BinasOverlay".to_vec()
        } else {
            format!("BinasOverlay{suffix}").into_bytes()
        };
        if !xobjects.contains_key(&name) {
            return name;
        }
    }
    unreachable!()
}

fn append_contents(
    dictionary: &mut BTreeMap<Vec<u8>, Value>,
    content: ObjectRef,
) -> Result<(), PdfError> {
    match dictionary.remove(b"Contents".as_slice()) {
        None => {
            dictionary.insert(b"Contents".to_vec(), Value::Ref(content));
        }
        Some(Value::Ref(existing)) => {
            dictionary.insert(
                b"Contents".to_vec(),
                Value::Array(vec![Value::Ref(existing), Value::Ref(content)]),
            );
        }
        Some(Value::Array(mut values))
            if values.iter().all(|value| matches!(value, Value::Ref(_))) =>
        {
            values.push(Value::Ref(content));
            dictionary.insert(b"Contents".to_vec(), Value::Array(values));
        }
        Some(_) => {
            return Err(PdfError::unsupported(
                "overlay requires indirect page content streams",
            ));
        }
    }
    Ok(())
}

fn placement_command(name: &[u8], matrix: [f64; 6]) -> Vec<u8> {
    format!(
        "q {} {} {} {} {} {} cm /{} Do Q\n",
        number(matrix[0]),
        number(matrix[1]),
        number(matrix[2]),
        number(matrix[3]),
        number(matrix[4]),
        number(matrix[5]),
        String::from_utf8_lossy(name)
    )
    .into_bytes()
}

fn placement_matches(
    parsed: &ParsedDocument,
    page: ObjectRef,
    content: ObjectRef,
    command: &[u8],
    form: ObjectRef,
) -> bool {
    let Ok(page) = parsed.object(page) else {
        return false;
    };
    let Ok(dictionary) = dictionary(&page.value, "verified page") else {
        return false;
    };
    let content_is_last = match dictionary.get(b"Contents".as_slice()) {
        Some(Value::Ref(reference)) => *reference == content,
        Some(Value::Array(values)) => values.last() == Some(&Value::Ref(content)),
        _ => false,
    };
    let stream_matches = parsed
        .object(content)
        .ok()
        .and_then(|object| object.stream.as_deref())
        == Some(command);
    let resource_matches = dictionary
        .get(b"Resources".as_slice())
        .and_then(|resources| resolve_dictionary(parsed, resources, "verified resources").ok())
        .and_then(|resources| resources.get(b"XObject".as_slice()).cloned())
        .and_then(|xobjects| resolve_dictionary(parsed, &xobjects, "verified XObjects").ok())
        .is_some_and(|xobjects| xobjects.values().any(|value| *value == Value::Ref(form)));
    content_is_last && stream_matches && resource_matches
}

fn ensure_capacity(parsed: &ParsedDocument, next: u32, additions: usize) -> Result<(), PdfError> {
    if parsed
        .objects
        .len()
        .checked_add(additions)
        .is_none_or(|count| count > parsed.limits.max_objects)
        || usize::try_from(next)
            .ok()
            .and_then(|next| next.checked_add(additions))
            .is_none_or(|size| size > parsed.limits.max_xref_entries)
    {
        return Err(PdfError::limit(
            "overlay objects exceed object or xref limits",
        ));
    }
    Ok(())
}

fn next_object_number(parsed: &ParsedDocument) -> Result<u32, PdfError> {
    parsed
        .objects
        .keys()
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("overlay object number overflows"))
}

fn allocate(next: &mut u32) -> Result<ObjectRef, PdfError> {
    let reference = ObjectRef {
        number: *next,
        generation: 0,
    };
    *next = next
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("overlay object number overflows"))?;
    Ok(reference)
}

fn validate_rectangle(rectangle: [f64; 4], label: &str) -> Result<(), PdfError> {
    if rectangle.iter().any(|value| !value.is_finite())
        || rectangle[2] <= rectangle[0]
        || rectangle[3] <= rectangle[1]
    {
        return Err(PdfError::unsupported(format!(
            "{label} must be finite with positive width and height"
        )));
    }
    Ok(())
}

fn rectangle_value(rectangle: [f64; 4]) -> Value {
    Value::Array(rectangle.into_iter().map(Value::Real).collect())
}

fn number(value: f64) -> String {
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}

fn stream_object(stream: Vec<u8>) -> IndirectObject {
    IndirectObject {
        value: Value::Dict(BTreeMap::new()),
        stream: Some(stream),
        stream_offset: 0,
        offset: 0,
    }
}

fn dictionary<'a>(value: &'a Value, label: &str) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(dictionary) => Ok(dictionary),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn dictionary_mut<'a>(
    value: &'a mut Value,
    label: &str,
) -> Result<&'a mut BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(dictionary) => Ok(dictionary),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn verify_references(parsed: &ParsedDocument) -> Result<bool, PdfError> {
    fn walk(value: &Value, parsed: &ParsedDocument, depth: usize) -> Result<bool, PdfError> {
        if depth > parsed.limits.max_parser_depth {
            return Err(PdfError::limit(
                "overlay reference validation exceeds depth limit",
            ));
        }
        match value {
            Value::Ref(reference) => Ok(parsed.objects.contains_key(reference)),
            Value::Array(values) => values.iter().try_fold(true, |valid, value| {
                Ok(valid && walk(value, parsed, depth + 1)?)
            }),
            Value::Dict(dictionary) => dictionary.values().try_fold(true, |valid, value| {
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
