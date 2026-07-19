use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    document::{PdfDocument, PdfEngine},
    encryption::write_encrypted_pdf,
    error::{PdfError, PdfErrorCode},
    forms::{AppearanceStatus, refuse_unsafe_interactive_edit},
    limits::OpenOptions,
    parser::{IndirectObject, ObjectRef, ParsedDocument, Value},
    writer::append_object_revision,
};

type ResolvedDict<'a> = (&'a BTreeMap<Vec<u8>, Value>, Option<ObjectRef>);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Annotation {
    pub index: usize,
    pub page_index: usize,
    pub page_object_number: u32,
    pub page_object_generation: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_generation: Option<u16>,
    pub subtype: String,
    pub rect: [f64; 4],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    pub flags: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnnotationContentsMutationRequest {
    pub annotation_index: usize,
    pub contents: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnnotationContentsMutationReport {
    pub operation: String,
    pub annotation_index: usize,
    pub object_number: u32,
    pub object_generation: u16,
    pub original_bytes: usize,
    pub appended_bytes: usize,
    pub appearance_status: AppearanceStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnnotationContentsMutationVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub contents_updated: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationContentsMutationOutcome {
    pub bytes: Vec<u8>,
    pub report: AnnotationContentsMutationReport,
    pub verification: AnnotationContentsMutationVerification,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationSubtype {
    Text,
    FreeText,
    Square,
    Circle,
    Link,
    Highlight,
    Underline,
    StrikeOut,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AnnotationCreateRequest {
    pub page_index: usize,
    pub subtype: AnnotationSubtype,
    pub rect: [f64; 4],
    #[serde(default)]
    pub contents: String,
    #[serde(default)]
    pub quad_points: Vec<f64>,
    #[serde(default)]
    pub uri: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnnotationRemoveRequest {
    pub annotation_index: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnnotationLifecycleReport {
    pub operation: String,
    pub subtype: String,
    pub annotation_index: usize,
    pub page_index: usize,
    pub annotations_affected: usize,
    pub appearances_affected: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AnnotationLifecycleVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub expected_annotation_count: bool,
    pub page_reachable: bool,
    pub appearance_reachable: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationLifecycleOutcome {
    pub bytes: Vec<u8>,
    pub report: AnnotationLifecycleReport,
    pub verification: AnnotationLifecycleVerification,
}

pub fn list_annotations(document: &PdfDocument) -> Result<Vec<Annotation>, PdfError> {
    let parsed = document.parsed();
    let mut output = Vec::new();
    for (page_index, page_ref) in document.page_refs()?.into_iter().enumerate() {
        let page = parsed.object(page_ref)?;
        let page_dict = dictionary(&page.value, Some(page_ref), "page")?;
        let Some(annots) = page_dict.get(b"Annots".as_slice()) else {
            continue;
        };
        let annots = resolve_array(parsed, annots, Some(page_ref), "page /Annots")?;
        if annots.len() > parsed.limits.max_container_items {
            return Err(PdfError::limit("page /Annots exceeds container limit"));
        }
        for value in annots {
            if output.len() >= parsed.limits.max_container_items {
                return Err(PdfError::limit("annotation count exceeds limit"));
            }
            let (dict, reference) = resolve_dict(parsed, value, "annotation")?;
            let subtype = required_name(
                dict.get(b"Subtype".as_slice()),
                reference,
                "annotation /Subtype",
            )?;
            let rect = rectangle(dict.get(b"Rect".as_slice()), reference)?;
            let contents = match dict.get(b"Contents".as_slice()) {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) => Some(decode_text(value)),
                Some(_) => {
                    return Err(malformed("annotation /Contents is not a string", reference));
                }
            };
            let flags = match dict.get(b"F".as_slice()) {
                None => 0,
                Some(Value::Integer(value)) => u32::try_from(*value)
                    .map_err(|_| malformed("annotation /F is outside u32", reference))?,
                Some(_) => return Err(malformed("annotation /F is not an integer", reference)),
            };
            output.push(Annotation {
                index: output.len(),
                page_index,
                page_object_number: page_ref.number,
                page_object_generation: page_ref.generation,
                object_number: reference.map(|value| value.number),
                object_generation: reference.map(|value| value.generation),
                subtype,
                rect,
                contents,
                flags,
            });
        }
    }
    Ok(output)
}

impl PdfDocument {
    pub fn set_annotation_contents(
        &self,
        request: AnnotationContentsMutationRequest,
    ) -> Result<AnnotationContentsMutationOutcome, PdfError> {
        let encoded_contents = encode_text(&request.contents);
        if encoded_contents.len() > self.engine_config().limits.max_token_bytes {
            return Err(PdfError::limit(
                "annotation contents exceeds max_token_bytes",
            ));
        }
        refuse_unsafe_interactive_edit(self)?;

        let annotations = list_annotations(self)?;
        let selected = annotations
            .get(request.annotation_index)
            .ok_or_else(|| PdfError {
                code: PdfErrorCode::SelectionNotFound,
                message: format!(
                    "annotation index {} is out of range for {} annotations",
                    request.annotation_index,
                    annotations.len()
                ),
                span: None,
                object: None,
            })?;
        if selected.subtype != "Text" {
            return Err(PdfError::unsafe_rewrite(
                "annotation mutation only supports /Subtype /Text",
            ));
        }
        let reference = match (selected.object_number, selected.object_generation) {
            (Some(number), Some(generation)) => ObjectRef { number, generation },
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "annotation mutation requires an indirect annotation dictionary",
                ));
            }
        };
        let object = self.parsed().object(reference)?;
        if object.stream.is_some() {
            return Err(PdfError::unsafe_rewrite(
                "annotation dictionary must not be a stream",
            ));
        }
        let mut replacement = dictionary(&object.value, Some(reference), "annotation")?.clone();
        if replacement.contains_key(b"AP".as_slice()) {
            return Err(PdfError::unsafe_rewrite(
                "annotation has an appearance stream; appearance generation is not implemented",
            ));
        }
        replacement.insert(b"Contents".to_vec(), Value::String(encoded_contents));

        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let bytes = append_object_revision(self, reference, &Value::Dict(replacement), None)?;
        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!(
                    "annotation mutation output did not reparse: {error}"
                ))
            })?;
        let prefix_preserved = bytes.starts_with(self.source());
        let page_count_unchanged = rewritten.page_count()? == old_pages;
        let contents_updated = list_annotations(&rewritten)?
            .get(request.annotation_index)
            .is_some_and(|annotation| {
                annotation.object_number == Some(reference.number)
                    && annotation.object_generation == Some(reference.generation)
                    && annotation.contents.as_deref() == Some(request.contents.as_str())
            });
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = AnnotationContentsMutationVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && contents_updated
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            contents_updated,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "annotation contents mutation failed post-write verification",
            ));
        }
        Ok(AnnotationContentsMutationOutcome {
            report: AnnotationContentsMutationReport {
                operation: "set_annotation_contents".into(),
                annotation_index: request.annotation_index,
                object_number: reference.number,
                object_generation: reference.generation,
                original_bytes: self.source().len(),
                appended_bytes: bytes.len() - self.source().len(),
                appearance_status: AppearanceStatus::Absent,
            },
            bytes,
            verification,
        })
    }
}

impl PdfDocument {
    pub fn create_annotation(
        &self,
        request: AnnotationCreateRequest,
    ) -> Result<AnnotationLifecycleOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        validate_annotation_create(&request, &self.parsed().limits)?;
        let pages = self.page_refs()?;
        let page = *pages.get(request.page_index).ok_or_else(|| {
            PdfError::selection(format!(
                "annotation page index {} is outside {} pages",
                request.page_index,
                pages.len()
            ))
        })?;
        let old_count = list_annotations(self)?.len();
        let mut parsed = self.parsed().clone();
        let refs = allocate_annotation_refs(
            &parsed,
            if request.subtype == AnnotationSubtype::FreeText {
                3
            } else {
                2
            },
        )?;
        let annotation_ref = refs[0];
        let appearance_ref = refs[1];
        let font_ref = refs.get(2).copied();
        let (appearance, font) =
            annotation_appearance(&request, font_ref, self.parsed().limits.max_stream_bytes)?;
        parsed.objects.insert(appearance_ref, appearance);
        if let Some((reference, object)) = font {
            parsed.objects.insert(reference, object);
        }
        let mut annotation = BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"Annot".to_vec())),
            (
                b"Subtype".to_vec(),
                Value::Name(annotation_subtype_name(request.subtype).to_vec()),
            ),
            (b"Rect".to_vec(), number_array(&request.rect)),
            (b"P".to_vec(), Value::Ref(page)),
            (b"F".to_vec(), Value::Integer(4)),
            (
                b"AP".to_vec(),
                Value::Dict(BTreeMap::from([(
                    b"N".to_vec(),
                    Value::Ref(appearance_ref),
                )])),
            ),
        ]);
        if !request.contents.is_empty() {
            annotation.insert(
                b"Contents".to_vec(),
                Value::String(encode_text(&request.contents)),
            );
        }
        match request.subtype {
            AnnotationSubtype::Link => {
                annotation.insert(
                    b"Border".to_vec(),
                    Value::Array(vec![
                        Value::Integer(0),
                        Value::Integer(0),
                        Value::Integer(1),
                    ]),
                );
                if !request.uri.is_empty() {
                    annotation.insert(
                        b"A".to_vec(),
                        Value::Dict(BTreeMap::from([
                            (b"S".to_vec(), Value::Name(b"URI".to_vec())),
                            (
                                b"URI".to_vec(),
                                Value::String(request.uri.as_bytes().to_vec()),
                            ),
                        ])),
                    );
                }
            }
            AnnotationSubtype::Highlight
            | AnnotationSubtype::Underline
            | AnnotationSubtype::StrikeOut => {
                annotation.insert(b"QuadPoints".to_vec(), number_array(&request.quad_points));
                annotation.insert(
                    b"C".to_vec(),
                    Value::Array(vec![
                        Value::Integer(1),
                        Value::Integer(1),
                        Value::Integer(0),
                    ]),
                );
            }
            AnnotationSubtype::Square | AnnotationSubtype::Circle => {
                annotation.insert(
                    b"C".to_vec(),
                    Value::Array(vec![
                        Value::Integer(0),
                        Value::Integer(0),
                        Value::Integer(1),
                    ]),
                );
                annotation.insert(
                    b"BS".to_vec(),
                    Value::Dict(BTreeMap::from([
                        (b"W".to_vec(), Value::Integer(1)),
                        (b"S".to_vec(), Value::Name(b"S".to_vec())),
                    ])),
                );
            }
            AnnotationSubtype::FreeText => {
                annotation.insert(b"DA".to_vec(), Value::String(b"/Helv 12 Tf 0 g".to_vec()));
            }
            AnnotationSubtype::Text => {
                annotation.insert(b"Name".to_vec(), Value::Name(b"Note".to_vec()));
            }
        }
        parsed
            .objects
            .insert(annotation_ref, plain_object(Value::Dict(annotation)));
        append_annotation_to_page(&mut parsed, page, annotation_ref)?;
        let bytes = write_annotation_lifecycle(self, parsed)?;
        let rewritten = reopen_annotation(self, &bytes, "created annotation")?;
        let annotations = list_annotations(&rewritten)?;
        let selected = annotations.iter().rfind(|annotation| {
            annotation.object_number == Some(annotation_ref.number)
                && annotation.object_generation == Some(annotation_ref.generation)
        });
        annotation_lifecycle_outcome(
            self,
            &rewritten,
            bytes,
            AnnotationLifecycleReport {
                operation: "create_annotation".into(),
                subtype: String::from_utf8_lossy(annotation_subtype_name(request.subtype))
                    .into_owned(),
                annotation_index: selected.map_or(annotations.len(), |value| value.index),
                page_index: request.page_index,
                annotations_affected: 1,
                appearances_affected: 1,
                input_bytes: self.source_len(),
                output_bytes: 0,
            },
            annotations.len() == old_count + 1,
            selected.is_some_and(|value| value.page_index == request.page_index)
                && page_has_annotation(rewritten.parsed(), page, annotation_ref)?,
            selected.is_some()
                && rewritten.parsed().objects.contains_key(&appearance_ref)
                && annotation_has_appearance(rewritten.parsed(), annotation_ref, appearance_ref)?,
        )
    }

    pub fn remove_annotation(
        &self,
        request: AnnotationRemoveRequest,
    ) -> Result<AnnotationLifecycleOutcome, PdfError> {
        refuse_unsafe_interactive_edit(self)?;
        let annotations = list_annotations(self)?;
        let selected = annotations.get(request.annotation_index).ok_or_else(|| {
            PdfError::selection(format!(
                "annotation index {} is outside {} annotations",
                request.annotation_index,
                annotations.len()
            ))
        })?;
        let annotation_ref = match (selected.object_number, selected.object_generation) {
            (Some(number), Some(generation)) => ObjectRef { number, generation },
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "annotation removal requires an indirect annotation",
                ));
            }
        };
        let page = ObjectRef {
            number: selected.page_object_number,
            generation: selected.page_object_generation,
        };
        let appearances = annotation_appearance_refs(self.parsed(), annotation_ref)?;
        let appearance_dependencies = appearances
            .iter()
            .flat_map(|reference| {
                let mut output = BTreeSet::new();
                if let Ok(object) = self.parsed().object(*reference) {
                    collect_references(&object.value, &mut output);
                }
                output
            })
            .collect::<BTreeSet<_>>();
        let old_count = annotations.len();
        let mut parsed = self.parsed().clone();
        remove_annotation_from_page(&mut parsed, page, annotation_ref)?;
        parsed.objects.remove(&annotation_ref);
        for appearance in &appearances {
            if !parsed
                .objects
                .values()
                .any(|object| contains_reference(&object.value, *appearance))
            {
                parsed.objects.remove(appearance);
            }
        }
        for dependency in appearance_dependencies {
            if !parsed
                .objects
                .values()
                .any(|object| contains_reference(&object.value, dependency))
            {
                parsed.objects.remove(&dependency);
            }
        }
        let bytes = write_annotation_lifecycle(self, parsed)?;
        let rewritten = reopen_annotation(self, &bytes, "removed annotation")?;
        let remaining = list_annotations(&rewritten)?;
        annotation_lifecycle_outcome(
            self,
            &rewritten,
            bytes,
            AnnotationLifecycleReport {
                operation: "remove_annotation".into(),
                subtype: selected.subtype.clone(),
                annotation_index: request.annotation_index,
                page_index: selected.page_index,
                annotations_affected: 1,
                appearances_affected: appearances.len(),
                input_bytes: self.source_len(),
                output_bytes: 0,
            },
            remaining.len() + 1 == old_count,
            !page_has_annotation(rewritten.parsed(), page, annotation_ref)?,
            appearances
                .iter()
                .all(|reference| !rewritten.parsed().objects.contains_key(reference)),
        )
    }
}

fn validate_annotation_create(
    request: &AnnotationCreateRequest,
    limits: &crate::Limits,
) -> Result<(), PdfError> {
    if request.rect.iter().any(|value| !value.is_finite())
        || request.rect[0] >= request.rect[2]
        || request.rect[1] >= request.rect[3]
    {
        return Err(PdfError::unsafe_rewrite(
            "annotation rectangle must be finite with x1 < x2 and y1 < y2",
        ));
    }
    if request.contents.len() > limits.max_token_bytes || request.uri.len() > limits.max_token_bytes
    {
        return Err(PdfError::limit(
            "annotation contents or URI exceeds token limit",
        ));
    }
    if request.subtype == AnnotationSubtype::FreeText && !request.contents.is_ascii() {
        return Err(PdfError::unsafe_rewrite(
            "FreeText appearance creation currently requires ASCII contents",
        ));
    }
    if request.subtype == AnnotationSubtype::Link {
        if !request.uri.is_ascii() {
            return Err(PdfError::unsafe_rewrite("Link URI must be ASCII"));
        }
    } else if !request.uri.is_empty() {
        return Err(PdfError::unsafe_rewrite(
            "only Link annotations accept a URI",
        ));
    }
    let markup = matches!(
        request.subtype,
        AnnotationSubtype::Highlight | AnnotationSubtype::Underline | AnnotationSubtype::StrikeOut
    );
    if markup {
        if request.quad_points.is_empty()
            || !request.quad_points.len().is_multiple_of(8)
            || request.quad_points.len() > limits.max_container_items
            || request.quad_points.iter().any(|value| !value.is_finite())
        {
            return Err(PdfError::unsafe_rewrite(
                "text markup annotations require finite QuadPoints in groups of eight",
            ));
        }
        for pair in request.quad_points.chunks_exact(2) {
            if pair[0] < request.rect[0]
                || pair[0] > request.rect[2]
                || pair[1] < request.rect[1]
                || pair[1] > request.rect[3]
            {
                return Err(PdfError::unsafe_rewrite(
                    "annotation QuadPoints must lie inside Rect",
                ));
            }
        }
    } else if !request.quad_points.is_empty() {
        return Err(PdfError::unsafe_rewrite(
            "only text markup annotations accept QuadPoints",
        ));
    }
    Ok(())
}

fn annotation_subtype_name(subtype: AnnotationSubtype) -> &'static [u8] {
    match subtype {
        AnnotationSubtype::Text => b"Text",
        AnnotationSubtype::FreeText => b"FreeText",
        AnnotationSubtype::Square => b"Square",
        AnnotationSubtype::Circle => b"Circle",
        AnnotationSubtype::Link => b"Link",
        AnnotationSubtype::Highlight => b"Highlight",
        AnnotationSubtype::Underline => b"Underline",
        AnnotationSubtype::StrikeOut => b"StrikeOut",
    }
}

fn allocate_annotation_refs(
    parsed: &ParsedDocument,
    count: usize,
) -> Result<Vec<ObjectRef>, PdfError> {
    if parsed
        .objects
        .len()
        .checked_add(count)
        .is_none_or(|value| value > parsed.limits.max_objects)
    {
        return Err(PdfError::limit(
            "annotation object allocation exceeds limits",
        ));
    }
    let mut number = parsed
        .objects
        .keys()
        .map(|value| value.number)
        .max()
        .unwrap_or(0);
    let mut output = Vec::with_capacity(count);
    for _ in 0..count {
        number = number
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("annotation object number overflows"))?;
        if usize::try_from(number)
            .ok()
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > parsed.limits.max_xref_entries)
        {
            return Err(PdfError::limit("annotation allocation exceeds xref limit"));
        }
        output.push(ObjectRef {
            number,
            generation: 0,
        });
    }
    Ok(output)
}

fn annotation_appearance(
    request: &AnnotationCreateRequest,
    font: Option<ObjectRef>,
    max_stream_bytes: usize,
) -> Result<(IndirectObject, Option<(ObjectRef, IndirectObject)>), PdfError> {
    let width = request.rect[2] - request.rect[0];
    let height = request.rect[3] - request.rect[1];
    let mut resources = BTreeMap::new();
    let (stream, font_object) = match request.subtype {
        AnnotationSubtype::Text => (
            format!("q 1 1 0 rg 0 0 {width} {height} re f 0 0 0 RG 0 0 {width} {height} re S Q\n")
                .into_bytes(),
            None,
        ),
        AnnotationSubtype::FreeText => {
            let font =
                font.ok_or_else(|| PdfError::verification("FreeText font allocation mismatch"))?;
            resources.insert(
                b"Font".to_vec(),
                Value::Dict(BTreeMap::from([(b"Helv".to_vec(), Value::Ref(font))])),
            );
            let font_object = plain_object(Value::Dict(BTreeMap::from([
                (b"Type".to_vec(), Value::Name(b"Font".to_vec())),
                (b"Subtype".to_vec(), Value::Name(b"Type1".to_vec())),
                (b"BaseFont".to_vec(), Value::Name(b"Helvetica".to_vec())),
                (
                    b"Encoding".to_vec(),
                    Value::Name(b"WinAnsiEncoding".to_vec()),
                ),
            ])));
            (
                format!(
                    "q BT /Helv 12 Tf 2 2 Td ({}) Tj ET Q\n",
                    escape_literal(&request.contents)
                )
                .into_bytes(),
                Some((font, font_object)),
            )
        }
        AnnotationSubtype::Square => (
            format!("q 0 0 1 RG 0 0 {width} {height} re S Q\n").into_bytes(),
            None,
        ),
        AnnotationSubtype::Circle => (ellipse_stream(width, height), None),
        AnnotationSubtype::Link => (
            format!("q 0 0 1 RG 0 0 {width} {height} re S Q\n").into_bytes(),
            None,
        ),
        AnnotationSubtype::Highlight
        | AnnotationSubtype::Underline
        | AnnotationSubtype::StrikeOut => (
            markup_stream(request.subtype, request.rect, &request.quad_points),
            None,
        ),
    };
    if stream.len() > max_stream_bytes {
        return Err(PdfError::limit(
            "annotation appearance exceeds stream limit",
        ));
    }
    Ok((
        IndirectObject {
            value: Value::Dict(BTreeMap::from([
                (b"Type".to_vec(), Value::Name(b"XObject".to_vec())),
                (b"Subtype".to_vec(), Value::Name(b"Form".to_vec())),
                (b"FormType".to_vec(), Value::Integer(1)),
                (b"BBox".to_vec(), number_array(&[0.0, 0.0, width, height])),
                (b"Resources".to_vec(), Value::Dict(resources)),
            ])),
            stream: Some(stream),
            stream_offset: 0,
            offset: 0,
        },
        font_object,
    ))
}

fn ellipse_stream(width: f64, height: f64) -> Vec<u8> {
    let rx = width / 2.0;
    let ry = height / 2.0;
    let cx = width / 2.0;
    let cy = height / 2.0;
    let ox = rx * 0.552_284_749_8;
    let oy = ry * 0.552_284_749_8;
    format!("q 0 0 1 RG {} {} m {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c {} {} {} {} {} {} c S Q\n",
        cx + rx, cy, cx + rx, cy + oy, cx + ox, cy + ry, cx, cy + ry,
        cx - ox, cy + ry, cx - rx, cy + oy, cx - rx, cy,
        cx - rx, cy - oy, cx - ox, cy - ry, cx, cy - ry,
        cx + ox, cy - ry, cx + rx, cy - oy, cx + rx, cy).into_bytes()
}

fn markup_stream(subtype: AnnotationSubtype, rect: [f64; 4], points: &[f64]) -> Vec<u8> {
    let mut stream = b"q 1 1 0 rg 1 0.8 0 RG\n".to_vec();
    for quad in points.chunks_exact(8) {
        let xs = [quad[0], quad[2], quad[4], quad[6]];
        let ys = [quad[1], quad[3], quad[5], quad[7]];
        let left = xs.into_iter().fold(f64::INFINITY, f64::min) - rect[0];
        let right = xs.into_iter().fold(f64::NEG_INFINITY, f64::max) - rect[0];
        let bottom = ys.into_iter().fold(f64::INFINITY, f64::min) - rect[1];
        let top = ys.into_iter().fold(f64::NEG_INFINITY, f64::max) - rect[1];
        let command = match subtype {
            AnnotationSubtype::Highlight => {
                format!("{left} {bottom} {} {} re f\n", right - left, top - bottom)
            }
            AnnotationSubtype::Underline => format!("{left} {bottom} m {right} {bottom} l S\n"),
            AnnotationSubtype::StrikeOut => {
                let middle = (bottom + top) / 2.0;
                format!("{left} {middle} m {right} {middle} l S\n")
            }
            _ => unreachable!(),
        };
        stream.extend_from_slice(command.as_bytes());
    }
    stream.extend_from_slice(b"Q\n");
    stream
}

fn escape_literal(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'(' | b')' | b'\\' => vec![b'\\', byte],
            0x20..=0x7e => vec![byte],
            _ => vec![b' '],
        })
        .map(char::from)
        .collect()
}

fn number_array(values: &[f64]) -> Value {
    Value::Array(
        values
            .iter()
            .map(|value| {
                if value.fract() == 0.0 && *value >= i64::MIN as f64 && *value <= i64::MAX as f64 {
                    Value::Integer(*value as i64)
                } else {
                    Value::Real(*value)
                }
            })
            .collect(),
    )
}

fn plain_object(value: Value) -> IndirectObject {
    IndirectObject {
        value,
        stream: None,
        stream_offset: 0,
        offset: 0,
    }
}

fn append_annotation_to_page(
    parsed: &mut ParsedDocument,
    page: ObjectRef,
    annotation: ObjectRef,
) -> Result<(), PdfError> {
    let object = parsed.object(page)?.clone();
    let mut dict = dictionary(&object.value, Some(page), "page")?.clone();
    match dict
        .entry(b"Annots".to_vec())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(values) => values.push(Value::Ref(annotation)),
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "page /Annots must be a direct array",
            ));
        }
    }
    parsed.objects.insert(page, plain_object(Value::Dict(dict)));
    Ok(())
}

fn remove_annotation_from_page(
    parsed: &mut ParsedDocument,
    page: ObjectRef,
    annotation: ObjectRef,
) -> Result<(), PdfError> {
    let object = parsed.object(page)?.clone();
    let mut dict = dictionary(&object.value, Some(page), "page")?.clone();
    let Some(Value::Array(values)) = dict.get_mut(b"Annots".as_slice()) else {
        return Err(PdfError::unsafe_rewrite(
            "annotation page /Annots must be a direct array",
        ));
    };
    let before = values.len();
    values.retain(|value| !matches!(value, Value::Ref(reference) if *reference == annotation));
    if values.len() == before {
        return Err(PdfError::unsafe_rewrite(
            "annotation reference is absent from page /Annots",
        ));
    }
    if values.is_empty() {
        dict.remove(b"Annots".as_slice());
    }
    parsed.objects.insert(page, plain_object(Value::Dict(dict)));
    Ok(())
}

fn page_has_annotation(
    parsed: &ParsedDocument,
    page: ObjectRef,
    annotation: ObjectRef,
) -> Result<bool, PdfError> {
    let dict = dictionary(&parsed.object(page)?.value, Some(page), "page")?;
    Ok(
        matches!(dict.get(b"Annots".as_slice()), Some(Value::Array(values)) if values.iter().any(|value| matches!(value, Value::Ref(reference) if *reference == annotation))),
    )
}

fn annotation_appearance_refs(
    parsed: &ParsedDocument,
    annotation: ObjectRef,
) -> Result<Vec<ObjectRef>, PdfError> {
    let dict = dictionary(
        &parsed.object(annotation)?.value,
        Some(annotation),
        "annotation",
    )?;
    let Some(ap) = dict.get(b"AP".as_slice()) else {
        return Ok(Vec::new());
    };
    let (ap, _) = resolve_dict(parsed, ap, "annotation /AP")?;
    match ap.get(b"N".as_slice()) {
        Some(Value::Ref(reference)) => Ok(vec![*reference]),
        None => Ok(Vec::new()),
        _ => Err(PdfError::unsafe_rewrite(
            "annotation /AP /N must be indirect",
        )),
    }
}

fn annotation_has_appearance(
    parsed: &ParsedDocument,
    annotation: ObjectRef,
    appearance: ObjectRef,
) -> Result<bool, PdfError> {
    Ok(annotation_appearance_refs(parsed, annotation)?.contains(&appearance))
}

fn contains_reference(value: &Value, expected: ObjectRef) -> bool {
    match value {
        Value::Ref(reference) => *reference == expected,
        Value::Array(values) => values
            .iter()
            .any(|value| contains_reference(value, expected)),
        Value::Dict(values) => values
            .values()
            .any(|value| contains_reference(value, expected)),
        _ => false,
    }
}

fn collect_references(value: &Value, output: &mut BTreeSet<ObjectRef>) {
    match value {
        Value::Ref(reference) => {
            output.insert(*reference);
        }
        Value::Array(values) => {
            for value in values {
                collect_references(value, output);
            }
        }
        Value::Dict(values) => {
            for value in values.values() {
                collect_references(value, output);
            }
        }
        _ => {}
    }
}

fn all_references_resolve(parsed: &ParsedDocument) -> bool {
    fn resolve(value: &Value, parsed: &ParsedDocument, depth: usize) -> bool {
        if depth > parsed.limits.max_parser_depth {
            return false;
        }
        match value {
            Value::Ref(reference) => parsed.objects.contains_key(reference),
            Value::Array(values) => values.iter().all(|value| resolve(value, parsed, depth + 1)),
            Value::Dict(values) => values
                .values()
                .all(|value| resolve(value, parsed, depth + 1)),
            _ => true,
        }
    }
    resolve(&parsed.trailer, parsed, 0)
        && parsed
            .objects
            .values()
            .all(|object| resolve(&object.value, parsed, 0))
}

fn write_annotation_lifecycle(
    document: &PdfDocument,
    parsed: ParsedDocument,
) -> Result<Vec<u8>, PdfError> {
    write_encrypted_pdf(document, &parsed)
}

fn reopen_annotation(
    document: &PdfDocument,
    bytes: &[u8],
    label: &str,
) -> Result<PdfDocument, PdfError> {
    PdfEngine::new(document.engine_config().clone())
        .open(bytes, OpenOptions::default())
        .map_err(|error| PdfError::verification(format!("{label} output did not reparse: {error}")))
}

fn annotation_lifecycle_outcome(
    original: &PdfDocument,
    rewritten: &PdfDocument,
    bytes: Vec<u8>,
    mut report: AnnotationLifecycleReport,
    expected_annotation_count: bool,
    page_reachable: bool,
    appearance_reachable: bool,
) -> Result<AnnotationLifecycleOutcome, PdfError> {
    let page_count_unchanged = original.page_count()? == rewritten.page_count()?;
    let no_dangling_references = all_references_resolve(rewritten.parsed());
    let verification = AnnotationLifecycleVerification {
        passed: page_count_unchanged
            && expected_annotation_count
            && page_reachable
            && appearance_reachable
            && no_dangling_references,
        reparsed: true,
        page_count_unchanged,
        expected_annotation_count,
        page_reachable,
        appearance_reachable,
        no_dangling_references,
    };
    if !verification.passed {
        return Err(PdfError::verification(
            "annotation lifecycle mutation failed post-write verification",
        ));
    }
    report.output_bytes = bytes.len();
    Ok(AnnotationLifecycleOutcome {
        bytes,
        report,
        verification,
    })
}

fn resolve_dict<'a>(
    parsed: &'a ParsedDocument,
    value: &'a Value,
    label: &str,
) -> Result<ResolvedDict<'a>, PdfError> {
    let (value, reference) = dereference(parsed, value, label)?;
    Ok((dictionary(value, reference, label)?, reference))
}

fn resolve_array<'a>(
    parsed: &'a ParsedDocument,
    value: &'a Value,
    owner: Option<ObjectRef>,
    label: &str,
) -> Result<&'a [Value], PdfError> {
    let (value, reference) = dereference(parsed, value, label)?;
    match value {
        Value::Array(value) => Ok(value),
        _ => Err(malformed(
            format!("{label} is not an array"),
            reference.or(owner),
        )),
    }
}

fn dereference<'a>(
    parsed: &'a ParsedDocument,
    mut value: &'a Value,
    label: &str,
) -> Result<(&'a Value, Option<ObjectRef>), PdfError> {
    let mut seen = BTreeSet::new();
    let mut last = None;
    while let Value::Ref(reference) = value {
        if seen.len() >= parsed.limits.max_parser_depth {
            return Err(PdfError::limit(format!(
                "{label} reference depth exceeds limit"
            )));
        }
        if !seen.insert(*reference) {
            return Err(malformed(
                format!("cycle resolving {label}"),
                Some(*reference),
            ));
        }
        let object = parsed.object(*reference).map_err(|mut error| {
            error.object = Some((reference.number, reference.generation));
            error
        })?;
        value = &object.value;
        last = Some(*reference);
    }
    Ok((value, last))
}

fn dictionary<'a>(
    value: &'a Value,
    reference: Option<ObjectRef>,
    label: &str,
) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(value) => Ok(value),
        _ => Err(malformed(format!("{label} is not a dictionary"), reference)),
    }
}

fn required_name(
    value: Option<&Value>,
    reference: Option<ObjectRef>,
    label: &str,
) -> Result<String, PdfError> {
    match value {
        Some(Value::Name(value)) => Ok(String::from_utf8_lossy(value).into_owned()),
        _ => Err(malformed(
            format!("{label} is missing or not a name"),
            reference,
        )),
    }
}

fn rectangle(value: Option<&Value>, reference: Option<ObjectRef>) -> Result<[f64; 4], PdfError> {
    let Some(Value::Array(values)) = value else {
        return Err(malformed(
            "annotation /Rect is missing or not an array",
            reference,
        ));
    };
    if values.len() != 4 {
        return Err(malformed(
            "annotation /Rect must contain four numbers",
            reference,
        ));
    }
    let mut rect = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        rect[index] = match value {
            Value::Integer(value) => *value as f64,
            Value::Real(value) if value.is_finite() => *value,
            _ => {
                return Err(malformed(
                    "annotation /Rect contains a non-number",
                    reference,
                ));
            }
        };
    }
    Ok(rect)
}

fn decode_text(value: &[u8]) -> String {
    if let Some(value) = value.strip_prefix(&[0xfe, 0xff]) {
        return String::from_utf16_lossy(
            &value
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        );
    }
    String::from_utf8_lossy(value).into_owned()
}

fn encode_text(value: &str) -> Vec<u8> {
    if value.is_ascii() {
        return value.as_bytes().to_vec();
    }
    let mut encoded = vec![0xfe, 0xff];
    for unit in value.encode_utf16() {
        encoded.extend_from_slice(&unit.to_be_bytes());
    }
    encoded
}

fn malformed(message: impl Into<String>, reference: Option<ObjectRef>) -> PdfError {
    PdfError {
        code: PdfErrorCode::InvalidSyntax,
        message: message.into(),
        span: None,
        object: reference.map(|value| (value.number, value.generation)),
    }
}
