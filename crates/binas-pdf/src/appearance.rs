use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    annotations::list_annotations,
    content,
    document::{PdfDocument, PdfEngine},
    edit::encode_literal,
    error::{PdfError, PdfErrorCode},
    forms::{AppearanceStatus, list_form_fields, refuse_unsafe_interactive_edit},
    limits::OpenOptions,
    parser::{ObjectRef, ParsedDocument, Value},
    writer::{append_object_revisions, next_object_reference},
};

type ResolvedDict<'a> = (&'a BTreeMap<Vec<u8>, Value>, Option<ObjectRef>);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextFieldAppearanceRequest {
    pub field_name: String,
    pub value: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextFieldAppearanceReport {
    pub operation: String,
    pub field_name: String,
    pub match_index: usize,
    pub field_object_number: u32,
    pub widget_object_number: u32,
    pub appearance_object_number: u32,
    pub font_name: String,
    pub font_size: f64,
    pub original_bytes: usize,
    pub appended_bytes: usize,
    pub appearance_status: AppearanceStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextFieldAppearanceVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub value_updated: bool,
    pub appearance_updated: bool,
    pub appearance_reachable: bool,
    pub no_dangling_references: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextFieldAppearanceOutcome {
    pub bytes: Vec<u8>,
    pub report: TextFieldAppearanceReport,
    pub verification: TextFieldAppearanceVerification,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FreeTextAppearanceRequest {
    pub annotation_index: usize,
    pub contents: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FreeTextAppearanceReport {
    pub operation: String,
    pub annotation_index: usize,
    pub annotation_object_number: u32,
    pub appearance_object_number: u32,
    pub font_name: String,
    pub font_size: f64,
    pub original_bytes: usize,
    pub appended_bytes: usize,
    pub appearance_status: AppearanceStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FreeTextAppearanceVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub contents_updated: bool,
    pub appearance_updated: bool,
    pub appearance_reachable: bool,
    pub no_dangling_references: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FreeTextAppearanceOutcome {
    pub bytes: Vec<u8>,
    pub report: FreeTextAppearanceReport,
    pub verification: FreeTextAppearanceVerification,
}

impl PdfDocument {
    pub fn regenerate_text_field_appearance(
        &self,
        request: TextFieldAppearanceRequest,
    ) -> Result<TextFieldAppearanceOutcome, PdfError> {
        if !request.value.is_ascii() {
            return Err(PdfError::unsafe_rewrite(
                "appearance regeneration currently requires ASCII text",
            ));
        }
        if request.value.len() > self.engine_config().limits.max_token_bytes {
            return Err(PdfError::limit("appearance text exceeds max_token_bytes"));
        }
        refuse_unsafe_interactive_edit(self)?;

        let fields = list_form_fields(self)?;
        let field = fields
            .iter()
            .filter(|field| field.name == request.field_name)
            .nth(request.match_index)
            .ok_or_else(|| selection_not_found(request.match_index))?;
        if field.field_type.as_deref() != Some("Tx") {
            return Err(PdfError::unsafe_rewrite(
                "appearance regeneration only supports text fields (/FT /Tx)",
            ));
        }
        if field.flags.unwrap_or(0) & (1 << 25) != 0 {
            return Err(PdfError::unsafe_rewrite(
                "rich-text form fields are not supported",
            ));
        }
        let field_ref = object_ref(
            field.object_number,
            field.object_generation,
            "appearance regeneration requires an indirect field dictionary",
        )?;
        if field.widget_refs.len() != 1 {
            return Err(PdfError::unsafe_rewrite(
                "appearance regeneration requires exactly one indirect widget",
            ));
        }
        let widget_ref = ObjectRef {
            number: field.widget_refs[0].object_number,
            generation: field.widget_refs[0].object_generation,
        };

        let field_object = self.parsed().object(field_ref)?;
        let mut field_dict =
            dictionary(&field_object.value, Some(field_ref), "form field")?.clone();
        if field_object.stream.is_some() || field_dict.contains_key(b"RV".as_slice()) {
            return Err(PdfError::unsafe_rewrite(
                "rich-text or stream-backed form fields are not supported",
            ));
        }
        let widget_object = self.parsed().object(widget_ref)?;
        let widget_dict = dictionary(&widget_object.value, Some(widget_ref), "widget")?;
        if widget_object.stream.is_some()
            || !matches!(widget_dict.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Widget")
        {
            return Err(PdfError::unsafe_rewrite(
                "selected widget is not an indirect Widget dictionary",
            ));
        }
        refuse_rotation(self.parsed(), widget_dict, widget_ref)?;

        let (font_name, font_size) = default_appearance(self.parsed())?;
        if !widget_dict.contains_key(b"AP".as_slice()) {
            if field_ref != widget_ref && field_dict.contains_key(b"AP".as_slice()) {
                return Err(PdfError::unsafe_rewrite(
                    "field-level appearances are not supported when creating a widget appearance",
                ));
            }
            return create_text_field_appearance(
                self,
                request,
                field_ref,
                widget_ref,
                field_dict,
                widget_dict,
                font_name,
                font_size,
            );
        }

        let ap = widget_dict
            .get(b"AP".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("widget has no /AP dictionary"))?;
        let (ap, _) = resolve_dict(self.parsed(), ap, "widget /AP")?;
        let appearance_ref = match ap.get(b"N".as_slice()) {
            Some(Value::Ref(reference)) => *reference,
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "widget /AP /N must be one indirect Form XObject",
                ));
            }
        };
        let appearance_object = self.parsed().object(appearance_ref)?;
        let appearance_dict = dictionary(
            &appearance_object.value,
            Some(appearance_ref),
            "appearance Form XObject",
        )?;
        if appearance_object.stream.is_none()
            || !matches!(appearance_dict.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"XObject")
            || !matches!(appearance_dict.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Form")
        {
            return Err(PdfError::unsafe_rewrite(
                "widget appearance is not a Form XObject stream",
            ));
        }
        if appearance_dict.contains_key(b"Filter".as_slice())
            || appearance_dict.contains_key(b"DecodeParms".as_slice())
        {
            return Err(PdfError::unsafe_rewrite(
                "filtered appearance streams are not supported",
            ));
        }
        let bbox = bbox(appearance_dict.get(b"BBox".as_slice()), appearance_ref)?;
        require_identity_matrix(appearance_dict.get(b"Matrix".as_slice()), appearance_ref)?;

        require_font_resource(self.parsed(), appearance_dict, &font_name, appearance_ref)?;
        let generated = appearance_content(&request.value, &font_name, font_size, bbox)?;
        if generated.len() > self.engine_config().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "generated appearance exceeds max_stream_bytes",
            ));
        }
        field_dict.insert(
            b"V".to_vec(),
            Value::String(request.value.as_bytes().to_vec()),
        );
        let field_value = Value::Dict(field_dict);
        let appearance_value = appearance_object.value.clone();

        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let bytes = append_object_revisions(
            self,
            &[
                (field_ref, &field_value, None),
                (appearance_ref, &appearance_value, Some(&generated)),
            ],
        )?;
        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!(
                    "appearance regeneration output did not reparse: {error}"
                ))
            })?;
        let prefix_preserved = bytes.starts_with(self.source());
        let page_count_unchanged = rewritten.page_count()? == old_pages;
        let value_updated = list_form_fields(&rewritten)?
            .iter()
            .filter(|field| field.name == request.field_name)
            .nth(request.match_index)
            .is_some_and(|field| field.value.as_deref() == Some(request.value.as_str()));
        let rewritten_appearance = rewritten.parsed().object(appearance_ref)?;
        let appearance_updated = rewritten_appearance.stream.as_deref()
            == Some(generated.as_slice())
            && content::extract_text_show(
                rewritten_appearance.stream.as_deref().unwrap_or_default(),
                0,
                &rewritten.parsed().limits,
            )?
            .iter()
            .any(|item| {
                item.text == request.value && item.font.as_deref() == Some(font_name.as_slice())
            });
        let appearance_reachable =
            widget_appearance_ref(&rewritten, widget_ref)? == Some(appearance_ref);
        let no_dangling_references =
            references_resolve(rewritten.parsed(), &rewritten_appearance.value, 0);
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = TextFieldAppearanceVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && value_updated
                && appearance_updated
                && appearance_reachable
                && no_dangling_references
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            value_updated,
            appearance_updated,
            appearance_reachable,
            no_dangling_references,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "text-field appearance regeneration failed post-write verification",
            ));
        }
        Ok(TextFieldAppearanceOutcome {
            report: TextFieldAppearanceReport {
                operation: "regenerate_text_field_appearance".into(),
                field_name: request.field_name,
                match_index: request.match_index,
                field_object_number: field_ref.number,
                widget_object_number: widget_ref.number,
                appearance_object_number: appearance_ref.number,
                font_name: String::from_utf8_lossy(&font_name).into_owned(),
                font_size,
                original_bytes: self.source().len(),
                appended_bytes: bytes.len() - self.source().len(),
                appearance_status: AppearanceStatus::Regenerated,
            },
            bytes,
            verification,
        })
    }

    pub fn regenerate_free_text_appearance(
        &self,
        request: FreeTextAppearanceRequest,
    ) -> Result<FreeTextAppearanceOutcome, PdfError> {
        if !request.contents.is_ascii() {
            return Err(PdfError::unsafe_rewrite(
                "FreeText appearance regeneration currently requires ASCII text",
            ));
        }
        if request.contents.len() > self.engine_config().limits.max_token_bytes {
            return Err(PdfError::limit("FreeText contents exceed max_token_bytes"));
        }
        refuse_unsafe_interactive_edit(self)?;
        let annotations = list_annotations(self)?;
        let selected = annotations
            .get(request.annotation_index)
            .ok_or_else(|| PdfError {
                code: PdfErrorCode::SelectionNotFound,
                message: format!(
                    "annotation index {} was not found",
                    request.annotation_index
                ),
                span: None,
                object: None,
            })?;
        if selected.subtype != "FreeText" {
            return Err(PdfError::unsafe_rewrite(
                "appearance regeneration only supports /Subtype /FreeText annotations",
            ));
        }
        let annotation_ref = object_ref(
            selected.object_number,
            selected.object_generation,
            "FreeText appearance regeneration requires an indirect annotation",
        )?;
        let annotation_object = self.parsed().object(annotation_ref)?;
        let mut annotation_dict = dictionary(
            &annotation_object.value,
            Some(annotation_ref),
            "FreeText annotation",
        )?
        .clone();
        if annotation_object.stream.is_some() || annotation_dict.contains_key(b"RC".as_slice()) {
            return Err(PdfError::unsafe_rewrite(
                "rich-text or stream-backed FreeText annotations are not supported",
            ));
        }
        match annotation_dict.get(b"Rotate".as_slice()) {
            None | Some(Value::Integer(0)) => {}
            Some(Value::Integer(_)) => {
                return Err(PdfError::unsafe_rewrite(
                    "rotated FreeText annotations are not supported",
                ));
            }
            Some(_) => {
                return Err(malformed(
                    "FreeText /Rotate is not an integer",
                    Some(annotation_ref),
                ));
            }
        }
        let da = match annotation_dict.get(b"DA".as_slice()) {
            Some(Value::String(value)) => value,
            _ => {
                return Err(malformed(
                    "FreeText /DA is missing or not a string",
                    Some(annotation_ref),
                ));
            }
        };
        let (font_name, font_size) = parse_default_appearance(da, self.parsed())?;
        let page_ref = ObjectRef {
            number: selected.page_object_number,
            generation: selected.page_object_generation,
        };
        let font_value = page_font(self.parsed(), page_ref, &font_name)?;

        let rect = selected.rect;
        let created_bbox = [0.0, 0.0, rect[2] - rect[0], rect[3] - rect[1]];
        let (appearance_ref, mut appearance_dict, status) =
            match annotation_dict.get(b"AP".as_slice()) {
                None => (
                    next_object_reference(self)?,
                    new_form_dictionary(created_bbox, &font_name, font_value.clone()),
                    AppearanceStatus::Created,
                ),
                Some(ap) => {
                    let (ap, _) = resolve_dict(self.parsed(), ap, "FreeText /AP")?;
                    if ap.len() != 1 || !ap.contains_key(b"N".as_slice()) {
                        return Err(PdfError::unsafe_rewrite(
                            "FreeText appearance updates require a simple /AP containing only /N",
                        ));
                    }
                    let reference = match ap.get(b"N".as_slice()) {
                        Some(Value::Ref(reference)) => *reference,
                        _ => {
                            return Err(PdfError::unsafe_rewrite(
                                "FreeText /AP /N must be one indirect Form XObject",
                            ));
                        }
                    };
                    let object = self.parsed().object(reference)?;
                    let dict = dictionary(&object.value, Some(reference), "FreeText appearance")?;
                    require_simple_form(object.stream.as_deref(), dict, reference)?;
                    require_identity_matrix(dict.get(b"Matrix".as_slice()), reference)?;
                    (reference, dict.clone(), AppearanceStatus::Regenerated)
                }
            };
        let appearance_bbox = bbox(appearance_dict.get(b"BBox".as_slice()), appearance_ref)?;
        appearance_dict.insert(
            b"Resources".to_vec(),
            font_resources(&font_name, font_value),
        );
        let generated =
            appearance_content(&request.contents, &font_name, font_size, appearance_bbox)?;
        if generated.len() > self.engine_config().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "generated FreeText appearance exceeds max_stream_bytes",
            ));
        }
        annotation_dict.insert(
            b"Contents".to_vec(),
            Value::String(request.contents.as_bytes().to_vec()),
        );
        if status == AppearanceStatus::Created {
            annotation_dict.insert(
                b"AP".to_vec(),
                Value::Dict(BTreeMap::from([(
                    b"N".to_vec(),
                    Value::Ref(appearance_ref),
                )])),
            );
        }
        let annotation_value = Value::Dict(annotation_dict);
        let appearance_value = Value::Dict(appearance_dict);
        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let bytes = append_object_revisions(
            self,
            &[
                (annotation_ref, &annotation_value, None),
                (appearance_ref, &appearance_value, Some(&generated)),
            ],
        )?;
        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!(
                    "FreeText appearance output did not reparse: {error}"
                ))
            })?;
        let rewritten_appearance = rewritten.parsed().object(appearance_ref)?;
        let prefix_preserved = bytes.starts_with(self.source());
        let page_count_unchanged = rewritten.page_count()? == old_pages;
        let contents_updated = list_annotations(&rewritten)?
            .get(request.annotation_index)
            .is_some_and(|annotation| {
                annotation.contents.as_deref() == Some(request.contents.as_str())
            });
        let appearance_updated = appearance_text_matches(
            &rewritten,
            appearance_ref,
            &generated,
            &request.contents,
            &font_name,
        )?;
        let appearance_reachable =
            annotation_appearance_ref(&rewritten, annotation_ref)? == Some(appearance_ref);
        let no_dangling_references =
            references_resolve(rewritten.parsed(), &rewritten_appearance.value, 0);
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = FreeTextAppearanceVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && contents_updated
                && appearance_updated
                && appearance_reachable
                && no_dangling_references
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            contents_updated,
            appearance_updated,
            appearance_reachable,
            no_dangling_references,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "FreeText appearance failed post-write verification",
            ));
        }
        Ok(FreeTextAppearanceOutcome {
            report: FreeTextAppearanceReport {
                operation: if status == AppearanceStatus::Created {
                    "create_free_text_appearance"
                } else {
                    "regenerate_free_text_appearance"
                }
                .into(),
                annotation_index: request.annotation_index,
                annotation_object_number: annotation_ref.number,
                appearance_object_number: appearance_ref.number,
                font_name: String::from_utf8_lossy(&font_name).into_owned(),
                font_size,
                original_bytes: self.source().len(),
                appended_bytes: bytes.len() - self.source().len(),
                appearance_status: status,
            },
            bytes,
            verification,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn create_text_field_appearance(
    document: &PdfDocument,
    request: TextFieldAppearanceRequest,
    field_ref: ObjectRef,
    widget_ref: ObjectRef,
    mut field_dict: BTreeMap<Vec<u8>, Value>,
    widget_dict: &BTreeMap<Vec<u8>, Value>,
    font_name: Vec<u8>,
    font_size: f64,
) -> Result<TextFieldAppearanceOutcome, PdfError> {
    let appearance_ref = next_object_reference(document)?;
    let rect = bbox(widget_dict.get(b"Rect".as_slice()), widget_ref)?;
    let bbox = [0.0, 0.0, rect[2] - rect[0], rect[3] - rect[1]];
    let font_value = acro_form_font(document.parsed(), &font_name)?;
    let appearance_dict = new_form_dictionary(bbox, &font_name, font_value);
    let appearance_value = Value::Dict(appearance_dict);
    let generated = appearance_content(&request.value, &font_name, font_size, bbox)?;
    if generated.len() > document.engine_config().limits.max_stream_bytes {
        return Err(PdfError::limit(
            "generated appearance exceeds max_stream_bytes",
        ));
    }
    let ap = Value::Dict(BTreeMap::from([(
        b"N".to_vec(),
        Value::Ref(appearance_ref),
    )]));
    field_dict.insert(
        b"V".to_vec(),
        Value::String(request.value.as_bytes().to_vec()),
    );
    let field_value;
    let widget_value;
    let bytes = if field_ref == widget_ref {
        field_dict.insert(b"AP".to_vec(), ap);
        field_value = Value::Dict(field_dict);
        append_object_revisions(
            document,
            &[
                (field_ref, &field_value, None),
                (appearance_ref, &appearance_value, Some(&generated)),
            ],
        )?
    } else {
        let mut replacement = widget_dict.clone();
        replacement.insert(b"AP".to_vec(), ap);
        field_value = Value::Dict(field_dict);
        widget_value = Value::Dict(replacement);
        append_object_revisions(
            document,
            &[
                (field_ref, &field_value, None),
                (widget_ref, &widget_value, None),
                (appearance_ref, &appearance_value, Some(&generated)),
            ],
        )?
    };

    let old_pages = document.page_count()?;
    let old_revisions = document.parsed().xref_revisions;
    let rewritten = PdfEngine::new(document.engine_config().clone())
        .open(&bytes, OpenOptions::default())
        .map_err(|error| {
            PdfError::verification(format!(
                "created appearance output did not reparse: {error}"
            ))
        })?;
    let rewritten_appearance = rewritten.parsed().object(appearance_ref)?;
    let prefix_preserved = bytes.starts_with(document.source());
    let page_count_unchanged = rewritten.page_count()? == old_pages;
    let value_updated = list_form_fields(&rewritten)?
        .iter()
        .filter(|field| field.name == request.field_name)
        .nth(request.match_index)
        .is_some_and(|field| field.value.as_deref() == Some(request.value.as_str()));
    let appearance_updated = appearance_text_matches(
        &rewritten,
        appearance_ref,
        &generated,
        &request.value,
        &font_name,
    )?;
    let appearance_reachable =
        widget_appearance_ref(&rewritten, widget_ref)? == Some(appearance_ref);
    let no_dangling_references =
        references_resolve(rewritten.parsed(), &rewritten_appearance.value, 0);
    let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
    let verification = TextFieldAppearanceVerification {
        passed: prefix_preserved
            && page_count_unchanged
            && value_updated
            && appearance_updated
            && appearance_reachable
            && no_dangling_references
            && revision_incremented,
        prefix_preserved,
        page_count_unchanged,
        value_updated,
        appearance_updated,
        appearance_reachable,
        no_dangling_references,
        revision_incremented,
    };
    if !verification.passed {
        return Err(PdfError::verification(
            "created text-field appearance failed post-write verification",
        ));
    }
    Ok(TextFieldAppearanceOutcome {
        report: TextFieldAppearanceReport {
            operation: "create_text_field_appearance".into(),
            field_name: request.field_name,
            match_index: request.match_index,
            field_object_number: field_ref.number,
            widget_object_number: widget_ref.number,
            appearance_object_number: appearance_ref.number,
            font_name: String::from_utf8_lossy(&font_name).into_owned(),
            font_size,
            original_bytes: document.source().len(),
            appended_bytes: bytes.len() - document.source().len(),
            appearance_status: AppearanceStatus::Created,
        },
        bytes,
        verification,
    })
}

fn acro_form_font(parsed: &ParsedDocument, font_name: &[u8]) -> Result<Value, PdfError> {
    let trailer = dictionary(&parsed.trailer, None, "trailer")?;
    let (catalog, _) = resolve_dict(
        parsed,
        trailer
            .get(b"Root".as_slice())
            .ok_or_else(|| malformed("trailer has no /Root", None))?,
        "catalog",
    )?;
    let (acro_form, _) = resolve_dict(
        parsed,
        catalog
            .get(b"AcroForm".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("catalog has no /AcroForm"))?,
        "catalog /AcroForm",
    )?;
    let (resources, _) = resolve_dict(
        parsed,
        acro_form
            .get(b"DR".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("AcroForm has no /DR resources"))?,
        "AcroForm /DR",
    )?;
    let (fonts, _) = resolve_dict(
        parsed,
        resources
            .get(b"Font".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("AcroForm /DR has no /Font"))?,
        "AcroForm /DR /Font",
    )?;
    let value = fonts.get(font_name).ok_or_else(|| {
        PdfError::unsafe_rewrite("AcroForm /DA font is absent from /DR resources")
    })?;
    let (font, reference) = resolve_dict(parsed, value, "AcroForm font resource")?;
    if !matches!(font.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Font") {
        return Err(malformed(
            "AcroForm /DR font resource is not a Font dictionary",
            reference,
        ));
    }
    Ok(value.clone())
}

fn new_form_dictionary(
    bbox: [f64; 4],
    font_name: &[u8],
    font_value: Value,
) -> BTreeMap<Vec<u8>, Value> {
    let font = Value::Dict(BTreeMap::from([(font_name.to_vec(), font_value)]));
    let resources = Value::Dict(BTreeMap::from([(b"Font".to_vec(), font)]));
    BTreeMap::from([
        (b"BBox".to_vec(), numbers(&bbox)),
        (b"FormType".to_vec(), Value::Integer(1)),
        (b"Matrix".to_vec(), numbers(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0])),
        (b"Resources".to_vec(), resources),
        (b"Subtype".to_vec(), Value::Name(b"Form".to_vec())),
        (b"Type".to_vec(), Value::Name(b"XObject".to_vec())),
    ])
}

fn numbers(values: &[f64]) -> Value {
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

fn font_resources(font_name: &[u8], font_value: Value) -> Value {
    Value::Dict(BTreeMap::from([(
        b"Font".to_vec(),
        Value::Dict(BTreeMap::from([(font_name.to_vec(), font_value)])),
    )]))
}

fn default_appearance(parsed: &ParsedDocument) -> Result<(Vec<u8>, f64), PdfError> {
    let trailer = dictionary(&parsed.trailer, None, "trailer")?;
    let (catalog, _) = resolve_dict(
        parsed,
        trailer
            .get(b"Root".as_slice())
            .ok_or_else(|| malformed("trailer has no /Root", None))?,
        "catalog",
    )?;
    let (acro_form, acro_ref) = resolve_dict(
        parsed,
        catalog
            .get(b"AcroForm".as_slice())
            .ok_or_else(|| malformed("catalog has no /AcroForm", None))?,
        "catalog /AcroForm",
    )?;
    let da = match acro_form.get(b"DA".as_slice()) {
        Some(Value::String(value)) => value.as_slice(),
        _ => {
            return Err(malformed(
                "AcroForm /DA is missing or not a string",
                acro_ref,
            ));
        }
    };
    parse_default_appearance(da, parsed)
}

fn parse_default_appearance(
    da: &[u8],
    parsed: &ParsedDocument,
) -> Result<(Vec<u8>, f64), PdfError> {
    if da.len() > parsed.limits.max_token_bytes {
        return Err(PdfError::limit(
            "default appearance exceeds max_token_bytes",
        ));
    }
    let tokens = da
        .split(u8::is_ascii_whitespace)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.len() > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "default appearance token count exceeds limit",
        ));
    }
    let mut selected = None;
    for window in tokens.windows(3) {
        if window[2] != b"Tf" || !window[0].starts_with(b"/") {
            continue;
        }
        if selected.is_some()
            || window[0].len() == 1
            || !window[0][1..]
                .iter()
                .all(|byte| (33..=126).contains(byte) && !b"()<>[]{}/%#".contains(byte))
        {
            return Err(PdfError::unsafe_rewrite(
                "default appearance must contain one simple font selection",
            ));
        }
        let size = std::str::from_utf8(window[1])
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|size| size.is_finite() && *size > 0.0)
            .ok_or_else(|| {
                PdfError::unsafe_rewrite(
                    "default appearance font size must be finite and greater than zero",
                )
            })?;
        selected = Some((window[0][1..].to_vec(), size));
    }
    selected.ok_or_else(|| {
        PdfError::unsafe_rewrite("default appearance has no supported non-autosize font selection")
    })
}

fn page_font(
    parsed: &ParsedDocument,
    page_ref: ObjectRef,
    font_name: &[u8],
) -> Result<Value, PdfError> {
    let mut current = page_ref;
    let mut seen = BTreeSet::new();
    let resources = loop {
        if seen.len() > parsed.limits.max_parser_depth {
            return Err(PdfError::limit("page resource inheritance exceeds limit"));
        }
        if !seen.insert(current) {
            return Err(malformed(
                "cycle in page resource inheritance",
                Some(current),
            ));
        }
        let page = parsed.object(current)?;
        let dict = dictionary(&page.value, Some(current), "page resource owner")?;
        if let Some(resources) = dict.get(b"Resources".as_slice()) {
            break resources;
        }
        current = match dict.get(b"Parent".as_slice()) {
            Some(Value::Ref(reference)) => *reference,
            _ => return Err(PdfError::unsafe_rewrite("page has no inherited /Resources")),
        };
    };
    let (resources, _) = resolve_dict(parsed, resources, "page /Resources")?;
    let (fonts, _) = resolve_dict(
        parsed,
        resources
            .get(b"Font".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("page resources have no /Font"))?,
        "page /Resources /Font",
    )?;
    let value = fonts.get(font_name).ok_or_else(|| {
        PdfError::unsafe_rewrite("FreeText /DA font is absent from page resources")
    })?;
    let (font, reference) = resolve_dict(parsed, value, "page font resource")?;
    if !matches!(font.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Font") {
        return Err(malformed(
            "page font resource is not a Font dictionary",
            reference,
        ));
    }
    Ok(value.clone())
}

fn require_simple_form(
    stream: Option<&[u8]>,
    dict: &BTreeMap<Vec<u8>, Value>,
    reference: ObjectRef,
) -> Result<(), PdfError> {
    if stream.is_none()
        || !matches!(dict.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"XObject")
        || !matches!(dict.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Form")
    {
        return Err(PdfError::unsafe_rewrite(
            "appearance is not a Form XObject stream",
        ));
    }
    if dict.contains_key(b"Filter".as_slice()) || dict.contains_key(b"DecodeParms".as_slice()) {
        return Err(PdfError::unsafe_rewrite(
            "filtered appearance streams are not supported",
        ));
    }
    bbox(dict.get(b"BBox".as_slice()), reference)?;
    Ok(())
}

fn appearance_text_matches(
    document: &PdfDocument,
    reference: ObjectRef,
    generated: &[u8],
    text: &str,
    font_name: &[u8],
) -> Result<bool, PdfError> {
    let object = document.parsed().object(reference)?;
    Ok(object.stream.as_deref() == Some(generated)
        && content::extract_text_show(
            object.stream.as_deref().unwrap_or_default(),
            0,
            &document.parsed().limits,
        )?
        .iter()
        .any(|item| item.text == text && item.font.as_deref() == Some(font_name)))
}

fn widget_appearance_ref(
    document: &PdfDocument,
    reference: ObjectRef,
) -> Result<Option<ObjectRef>, PdfError> {
    annotation_appearance_ref(document, reference)
}

fn annotation_appearance_ref(
    document: &PdfDocument,
    reference: ObjectRef,
) -> Result<Option<ObjectRef>, PdfError> {
    let object = document.parsed().object(reference)?;
    let dict = dictionary(&object.value, Some(reference), "appearance owner")?;
    let Some(ap) = dict.get(b"AP".as_slice()) else {
        return Ok(None);
    };
    let (ap, _) = resolve_dict(document.parsed(), ap, "appearance owner /AP")?;
    match ap.get(b"N".as_slice()) {
        Some(Value::Ref(reference)) => Ok(Some(*reference)),
        _ => Ok(None),
    }
}

fn references_resolve(parsed: &ParsedDocument, value: &Value, depth: usize) -> bool {
    if depth > parsed.limits.max_parser_depth {
        return false;
    }
    match value {
        Value::Ref(reference) => parsed.objects.contains_key(reference),
        Value::Array(values) => values
            .iter()
            .all(|value| references_resolve(parsed, value, depth + 1)),
        Value::Dict(values) => values
            .values()
            .all(|value| references_resolve(parsed, value, depth + 1)),
        _ => true,
    }
}

fn require_font_resource(
    parsed: &ParsedDocument,
    appearance: &BTreeMap<Vec<u8>, Value>,
    font_name: &[u8],
    reference: ObjectRef,
) -> Result<(), PdfError> {
    let (resources, _) = resolve_dict(
        parsed,
        appearance
            .get(b"Resources".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("appearance has no /Resources"))?,
        "appearance /Resources",
    )?;
    let (fonts, _) = resolve_dict(
        parsed,
        resources
            .get(b"Font".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("appearance resources have no /Font"))?,
        "appearance /Resources /Font",
    )?;
    let (font, font_ref) = resolve_dict(
        parsed,
        fonts.get(font_name).ok_or_else(|| {
            PdfError::unsafe_rewrite("AcroForm /DA font is absent from appearance resources")
        })?,
        "appearance font resource",
    )?;
    if !matches!(font.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Font") {
        return Err(malformed(
            "appearance font resource is not a Font dictionary",
            font_ref.or(Some(reference)),
        ));
    }
    Ok(())
}

fn appearance_content(
    text: &str,
    font_name: &[u8],
    font_size: f64,
    bbox: [f64; 4],
) -> Result<Vec<u8>, PdfError> {
    let height = bbox[3] - bbox[1];
    if height <= 0.0 || bbox[2] <= bbox[0] {
        return Err(PdfError::unsafe_rewrite(
            "appearance /BBox must have positive width and height",
        ));
    }
    let x = bbox[0] + 2.0;
    let y = bbox[1] + (height - font_size) / 2.0;
    if !x.is_finite() || !y.is_finite() {
        return Err(PdfError::unsafe_rewrite(
            "appearance text position is not finite",
        ));
    }
    let literal = encode_literal(text.as_bytes());
    let mut output = Vec::new();
    output.extend_from_slice(b"q\nBT\n/");
    output.extend_from_slice(font_name);
    output.extend_from_slice(
        format!(
            " {} Tf\n1 0 0 1 {} {} Tm\n",
            number(font_size),
            number(x),
            number(y)
        )
        .as_bytes(),
    );
    output.extend_from_slice(&literal);
    output.extend_from_slice(b" Tj\nET\nQ\n");
    Ok(output)
}

fn number(value: f64) -> String {
    if value == 0.0 {
        "0".into()
    } else {
        value.to_string()
    }
}

fn bbox(value: Option<&Value>, reference: ObjectRef) -> Result<[f64; 4], PdfError> {
    let Some(Value::Array(values)) = value else {
        return Err(malformed(
            "appearance /BBox is missing or not an array",
            Some(reference),
        ));
    };
    if values.len() != 4 {
        return Err(malformed(
            "appearance /BBox must contain four numbers",
            Some(reference),
        ));
    }
    let mut result = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        result[index] = finite_number(value).ok_or_else(|| {
            malformed(
                "appearance /BBox contains a non-finite number",
                Some(reference),
            )
        })?;
    }
    Ok(result)
}

fn require_identity_matrix(value: Option<&Value>, reference: ObjectRef) -> Result<(), PdfError> {
    let Some(value) = value else {
        return Ok(());
    };
    let Value::Array(values) = value else {
        return Err(malformed(
            "appearance /Matrix is not an array",
            Some(reference),
        ));
    };
    let expected = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    if values.len() != expected.len()
        || !values
            .iter()
            .zip(expected)
            .all(|(value, expected)| finite_number(value) == Some(expected))
    {
        return Err(PdfError::unsafe_rewrite(
            "rotated or transformed appearance matrices are not supported",
        ));
    }
    Ok(())
}

fn refuse_rotation(
    parsed: &ParsedDocument,
    widget: &BTreeMap<Vec<u8>, Value>,
    reference: ObjectRef,
) -> Result<(), PdfError> {
    let Some(mk) = widget.get(b"MK".as_slice()) else {
        return Ok(());
    };
    let (mk, mk_ref) = resolve_dict(parsed, mk, "widget /MK")?;
    match mk.get(b"R".as_slice()) {
        None | Some(Value::Integer(0)) => Ok(()),
        Some(Value::Integer(_)) => Err(PdfError::unsafe_rewrite(
            "rotated widget appearances are not supported",
        )),
        Some(_) => Err(malformed(
            "widget /MK /R is not an integer",
            mk_ref.or(Some(reference)),
        )),
    }
}

fn finite_number(value: &Value) -> Option<f64> {
    match value {
        Value::Integer(value) => Some(*value as f64),
        Value::Real(value) if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn object_ref(
    number: Option<u32>,
    generation: Option<u16>,
    message: &str,
) -> Result<ObjectRef, PdfError> {
    match (number, generation) {
        (Some(number), Some(generation)) => Ok(ObjectRef { number, generation }),
        _ => Err(PdfError::unsafe_rewrite(message)),
    }
}

fn resolve_dict<'a>(
    parsed: &'a ParsedDocument,
    value: &'a Value,
    label: &str,
) -> Result<ResolvedDict<'a>, PdfError> {
    let mut value = value;
    let mut seen = BTreeSet::new();
    let mut reference = None;
    while let Value::Ref(next) = value {
        if seen.len() >= parsed.limits.max_parser_depth {
            return Err(PdfError::limit(format!(
                "{label} reference depth exceeds limit"
            )));
        }
        if !seen.insert(*next) {
            return Err(malformed(format!("cycle resolving {label}"), Some(*next)));
        }
        value = &parsed.object(*next)?.value;
        reference = Some(*next);
    }
    Ok((dictionary(value, reference, label)?, reference))
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

fn selection_not_found(match_index: usize) -> PdfError {
    PdfError {
        code: PdfErrorCode::SelectionNotFound,
        message: format!("form field match index {match_index} was not found"),
        span: None,
        object: None,
    }
}

fn malformed(message: impl Into<String>, reference: Option<ObjectRef>) -> PdfError {
    PdfError {
        code: PdfErrorCode::InvalidSyntax,
        message: message.into(),
        span: None,
        object: reference.map(|value| (value.number, value.generation)),
    }
}
