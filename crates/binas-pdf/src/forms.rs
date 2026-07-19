use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    document::{PdfDocument, PdfEngine},
    encryption::write_encrypted_pdf,
    error::{PdfError, PdfErrorCode},
    limits::OpenOptions,
    parser::{IndirectObject, ObjectRef, ParsedDocument, Value},
    writer::{append_object_revision, append_object_revisions},
};

type ResolvedDict<'a> = (&'a BTreeMap<Vec<u8>, Value>, Option<ObjectRef>);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormWidgetRef {
    pub object_number: u32,
    pub object_generation: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormField {
    pub index: usize,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_generation: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widget_refs: Vec<FormWidgetRef>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceStatus {
    ViewerRegenerationRequired,
    Regenerated,
    Created,
    Absent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormValueMutationRequest {
    pub field_name: String,
    pub value: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormValueMutationReport {
    pub operation: String,
    pub field_name: String,
    pub match_index: usize,
    pub object_number: u32,
    pub object_generation: u16,
    pub original_bytes: usize,
    pub appended_bytes: usize,
    pub appearance_status: AppearanceStatus,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormValueMutationVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub value_updated: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormValueMutationOutcome {
    pub bytes: Vec<u8>,
    pub report: FormValueMutationReport,
    pub verification: FormValueMutationVerification,
}

/// Selects a proven checkbox-like AcroForm button by its fully qualified name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckboxFieldMutationRequest {
    pub field_name: String,
    pub checked: bool,
    #[serde(default)]
    pub match_index: usize,
}

/// Selects a proven radio-button state by its exact exported appearance name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ButtonChoiceMutationRequest {
    pub field_name: String,
    pub state: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ButtonFieldMutationReport {
    pub operation: String,
    pub field_name: String,
    pub match_index: usize,
    pub object_number: u32,
    pub object_generation: u16,
    pub selected_state: String,
    pub widgets_affected: usize,
    pub original_bytes: usize,
    pub appended_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ButtonFieldMutationVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub value_updated: bool,
    pub widget_appearance_states_updated: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonFieldMutationOutcome {
    pub bytes: Vec<u8>,
    pub report: ButtonFieldMutationReport,
    pub verification: ButtonFieldMutationVerification,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldKind {
    Text,
    Checkbox,
    Radio,
    Choice,
    Signature,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FormFieldCreateRequest {
    pub name: String,
    pub page_index: usize,
    pub rect: [f64; 4],
    pub kind: FormFieldKind,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormFieldRemoveRequest {
    pub field_name: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormLifecycleReport {
    pub operation: String,
    pub field_name: String,
    pub field_type: String,
    pub page_index: usize,
    pub fields_affected: usize,
    pub widgets_affected: usize,
    pub appearances_placed: usize,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormLifecycleVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub expected_field_count: bool,
    pub widgets_reachable: bool,
    pub appearances_reachable: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormLifecycleOutcome {
    pub bytes: Vec<u8>,
    pub report: FormLifecycleReport,
    pub verification: FormLifecycleVerification,
}

#[derive(Clone, Default)]
struct FieldContext {
    name: String,
    field_type: Option<String>,
    flags: Option<u32>,
}

#[derive(Default)]
struct PagePaint {
    xobjects: BTreeMap<Vec<u8>, Value>,
    commands: Vec<u8>,
}

const BUTTON_FLAG_RADIO: u32 = 1 << 15;
const BUTTON_FLAG_PUSHBUTTON: u32 = 1 << 16;

enum ButtonMutationKind {
    Checkbox { checked: bool },
    Choice { state: String },
}

impl ButtonMutationKind {
    fn operation(&self) -> &'static str {
        match self {
            Self::Checkbox { .. } => "set_checkbox_field",
            Self::Choice { .. } => "set_button_field_choice",
        }
    }

    fn select_state(&self, flags: u32, plan: &ButtonMutationPlan) -> Result<Vec<u8>, PdfError> {
        match self {
            Self::Checkbox { checked } => {
                if flags & (BUTTON_FLAG_RADIO | BUTTON_FLAG_PUSHBUTTON) != 0 {
                    return Err(PdfError::unsafe_rewrite(
                        "checkbox mutation does not support radio buttons or pushbuttons",
                    ));
                }
                let [widget] = plan.widgets.as_slice() else {
                    return Err(PdfError::unsafe_rewrite(
                        "checkbox mutation requires exactly one proven widget",
                    ));
                };
                Ok(if *checked {
                    widget.on_state.clone()
                } else {
                    b"Off".to_vec()
                })
            }
            Self::Choice { state } => {
                if state.is_empty() {
                    return Err(PdfError::unsafe_rewrite(
                        "button choice mutation requires a non-empty state",
                    ));
                }
                if flags & BUTTON_FLAG_PUSHBUTTON != 0 || flags & BUTTON_FLAG_RADIO == 0 {
                    return Err(PdfError::unsafe_rewrite(
                        "button choice mutation only supports radio buttons",
                    ));
                }
                if state == "Off" {
                    return Ok(b"Off".to_vec());
                }
                let state = state.as_bytes();
                let matches = plan
                    .widgets
                    .iter()
                    .filter(|widget| widget.on_state == state)
                    .count();
                if matches != 1 {
                    return Err(PdfError::unsafe_rewrite(
                        "radio choice must select one unique proven appearance state",
                    ));
                }
                Ok(state.to_vec())
            }
        }
    }
}

struct ButtonMutationPlan {
    field_is_widget: bool,
    widgets: Vec<ButtonWidget>,
}

struct ButtonWidget {
    reference: ObjectRef,
    dictionary: BTreeMap<Vec<u8>, Value>,
    on_state: Vec<u8>,
}

pub fn list_form_fields(document: &PdfDocument) -> Result<Vec<FormField>, PdfError> {
    let parsed = document.parsed();
    let trailer = dictionary(&parsed.trailer, None, "trailer")?;
    let root = trailer
        .get(b"Root".as_slice())
        .ok_or_else(|| malformed("trailer has no /Root", None))?;
    let (catalog, _) = resolve_dict(parsed, root, "catalog")?;
    let Some(acro_form) = catalog.get(b"AcroForm".as_slice()) else {
        return Ok(Vec::new());
    };
    if matches!(acro_form, Value::Null) {
        return Ok(Vec::new());
    }
    let (acro_form, acro_ref) = resolve_dict(parsed, acro_form, "catalog /AcroForm")?;
    let Some(fields) = acro_form.get(b"Fields".as_slice()) else {
        return Ok(Vec::new());
    };
    let fields = resolve_array(parsed, fields, acro_ref, "AcroForm /Fields")?;
    if fields.len() > parsed.limits.max_container_items {
        return Err(PdfError::limit("AcroForm /Fields exceeds container limit"));
    }

    let mut output = Vec::new();
    let mut path = BTreeSet::new();
    let mut walked = 0usize;
    for field in fields {
        walk_field(
            parsed,
            field,
            &FieldContext::default(),
            0,
            &mut path,
            &mut walked,
            &mut output,
        )?;
    }
    for (index, field) in output.iter_mut().enumerate() {
        field.index = index;
    }
    Ok(output)
}

impl PdfDocument {
    pub fn set_form_field_value(
        &self,
        request: FormValueMutationRequest,
    ) -> Result<FormValueMutationOutcome, PdfError> {
        let encoded_value = encode_text(&request.value);
        if encoded_value.len() > self.engine_config().limits.max_token_bytes {
            return Err(PdfError::limit("form value exceeds max_token_bytes"));
        }
        refuse_unsafe_interactive_edit(self)?;

        let fields = list_form_fields(self)?;
        let selected = fields
            .iter()
            .filter(|field| field.name == request.field_name)
            .nth(request.match_index)
            .ok_or_else(|| selection_not_found("form field", request.match_index))?;
        if selected.field_type.as_deref() != Some("Tx") {
            return Err(PdfError::unsafe_rewrite(
                "form mutation only supports text fields (/FT /Tx)",
            ));
        }
        let reference = match (selected.object_number, selected.object_generation) {
            (Some(number), Some(generation)) => ObjectRef { number, generation },
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "form mutation requires an indirect field dictionary",
                ));
            }
        };
        let object = self.parsed().object(reference)?;
        if object.stream.is_some() {
            return Err(PdfError::unsafe_rewrite(
                "form field dictionary must not be a stream",
            ));
        }
        let mut replacement = dictionary(&object.value, Some(reference), "form field")?.clone();
        if field_or_widget_has_appearance(self.parsed(), &replacement, Some(reference))? {
            return Err(PdfError::unsafe_rewrite(
                "form field or widget has an appearance stream; appearance generation is not implemented",
            ));
        }
        require_need_appearances(self.parsed())?;
        replacement.insert(b"V".to_vec(), Value::String(encoded_value));

        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let bytes = append_object_revision(self, reference, &Value::Dict(replacement), None)?;
        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("form mutation output did not reparse: {error}"))
            })?;
        let prefix_preserved = bytes.starts_with(self.source());
        let page_count_unchanged = rewritten.page_count()? == old_pages;
        let value_updated = list_form_fields(&rewritten)?
            .iter()
            .filter(|field| field.name == request.field_name)
            .nth(request.match_index)
            .is_some_and(|field| field.value.as_deref() == Some(request.value.as_str()));
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = FormValueMutationVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && value_updated
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            value_updated,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "form value mutation failed post-write verification",
            ));
        }
        Ok(FormValueMutationOutcome {
            report: FormValueMutationReport {
                operation: "set_form_field_value".into(),
                field_name: request.field_name,
                match_index: request.match_index,
                object_number: reference.number,
                object_generation: reference.generation,
                original_bytes: self.source().len(),
                appended_bytes: bytes.len() - self.source().len(),
                appearance_status: AppearanceStatus::ViewerRegenerationRequired,
            },
            bytes,
            verification,
        })
    }

    /// Updates a checkbox only when its existing normal appearance proves one unambiguous on-state.
    pub fn set_checkbox_field(
        &self,
        request: CheckboxFieldMutationRequest,
    ) -> Result<ButtonFieldMutationOutcome, PdfError> {
        self.mutate_button_field(
            request.field_name,
            request.match_index,
            ButtonMutationKind::Checkbox {
                checked: request.checked,
            },
        )
    }

    /// Updates a radio button only when `state` exactly names one proven normal appearance state.
    pub fn set_button_field_choice(
        &self,
        request: ButtonChoiceMutationRequest,
    ) -> Result<ButtonFieldMutationOutcome, PdfError> {
        self.mutate_button_field(
            request.field_name,
            request.match_index,
            ButtonMutationKind::Choice {
                state: request.state,
            },
        )
    }

    fn mutate_button_field(
        &self,
        field_name: String,
        match_index: usize,
        mutation: ButtonMutationKind,
    ) -> Result<ButtonFieldMutationOutcome, PdfError> {
        if field_name.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "button field mutation requires a field name",
            ));
        }
        refuse_unsafe_interactive_edit(self)?;

        let fields = list_form_fields(self)?;
        let selected = fields
            .iter()
            .filter(|field| field.name == field_name)
            .nth(match_index)
            .ok_or_else(|| selection_not_found("form field", match_index))?;
        if selected.field_type.as_deref() != Some("Btn") {
            return Err(PdfError::unsafe_rewrite(
                "button mutation only supports button fields (/FT /Btn)",
            ));
        }
        let field_ref = field_reference(selected)?;
        let field_object = self.parsed().object(field_ref)?;
        if field_object.stream.is_some() {
            return Err(PdfError::unsafe_rewrite(
                "button field dictionary must not be a stream",
            ));
        }
        let field = dictionary(&field_object.value, Some(field_ref), "button field")?.clone();
        let plan = button_mutation_plan(self.parsed(), field_ref, &field)?;
        let state = mutation.select_state(selected.flags.unwrap_or_default(), &plan)?;

        let mut replacements = BTreeMap::new();
        let mut field_replacement = field;
        field_replacement.insert(b"V".to_vec(), Value::Name(state.clone()));
        if plan.field_is_widget {
            field_replacement.insert(b"AS".to_vec(), Value::Name(state.clone()));
        }
        replacements.insert(field_ref, Value::Dict(field_replacement));

        let expected_widget_states = plan
            .widgets
            .iter()
            .map(|widget| {
                let widget_state = if plan.field_is_widget || widget.on_state == state {
                    state.clone()
                } else {
                    b"Off".to_vec()
                };
                (widget.reference, widget_state)
            })
            .collect::<Vec<_>>();
        if !plan.field_is_widget {
            for (widget, (_, widget_state)) in plan.widgets.iter().zip(&expected_widget_states) {
                let mut replacement = widget.dictionary.clone();
                replacement.insert(b"AS".to_vec(), Value::Name(widget_state.clone()));
                if replacements
                    .insert(widget.reference, Value::Dict(replacement))
                    .is_some()
                {
                    return Err(PdfError::unsafe_rewrite(
                        "button field shares an object with one of its widgets",
                    ));
                }
            }
        }

        let replacement_refs = replacements
            .iter()
            .map(|(reference, value)| (*reference, value, None))
            .collect::<Vec<_>>();
        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let bytes = append_object_revisions(self, &replacement_refs)?;
        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!(
                    "button field mutation output did not reparse: {error}"
                ))
            })?;
        let prefix_preserved = bytes.starts_with(self.source());
        let page_count_unchanged = rewritten.page_count()? == old_pages;
        let value_updated = button_field_value_is(&rewritten, field_ref, &state)?;
        let widget_appearance_states_updated = expected_widget_states
            .iter()
            .map(|(reference, expected)| button_widget_state_is(&rewritten, *reference, expected))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .all(|value| value);
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = ButtonFieldMutationVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && value_updated
                && widget_appearance_states_updated
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            value_updated,
            widget_appearance_states_updated,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "button field mutation failed post-write verification",
            ));
        }
        Ok(ButtonFieldMutationOutcome {
            report: ButtonFieldMutationReport {
                operation: mutation.operation().into(),
                field_name,
                match_index,
                object_number: field_ref.number,
                object_generation: field_ref.generation,
                selected_state: String::from_utf8_lossy(&state).into_owned(),
                widgets_affected: expected_widget_states.len(),
                original_bytes: self.source().len(),
                appended_bytes: bytes.len() - self.source().len(),
            },
            bytes,
            verification,
        })
    }
}

impl PdfDocument {
    pub fn create_form_field(
        &self,
        request: FormFieldCreateRequest,
    ) -> Result<FormLifecycleOutcome, PdfError> {
        refuse_form_lifecycle_security(self)?;
        validate_create_request(&request, &self.parsed().limits)?;
        let pages = self.page_refs()?;
        let page_ref = *pages.get(request.page_index).ok_or_else(|| {
            PdfError::unsafe_rewrite(format!(
                "form page index {} is outside {} pages",
                request.page_index,
                pages.len()
            ))
        })?;
        let old_count = list_form_fields(self)?.len();
        let root_ref = root_ref(self.parsed())?;
        let mut parsed = self.parsed().clone();
        let references = allocate_references(&parsed, create_object_count(request.kind))?;
        let field_ref = references[0];
        let widget_ref = references[1];
        let appearance_refs = &references[2..];
        let (appearance, objects) = create_appearances(&request, appearance_refs)?;
        for (reference, object) in objects {
            parsed.objects.insert(reference, object);
        }

        let mut field = BTreeMap::from([
            (
                b"FT".to_vec(),
                Value::Name(field_type(request.kind).to_vec()),
            ),
            (b"T".to_vec(), Value::String(encode_text(&request.name))),
            (b"Kids".to_vec(), Value::Array(vec![Value::Ref(widget_ref)])),
        ]);
        match request.kind {
            FormFieldKind::Text => {
                if !request.value.is_empty() {
                    field.insert(b"V".to_vec(), Value::String(encode_text(&request.value)));
                }
            }
            FormFieldKind::Checkbox | FormFieldKind::Radio => {
                field.insert(b"V".to_vec(), Value::Name(b"Off".to_vec()));
                if request.kind == FormFieldKind::Radio {
                    field.insert(b"Ff".to_vec(), Value::Integer(1 << 15));
                }
            }
            FormFieldKind::Choice => {
                field.insert(
                    b"Opt".to_vec(),
                    Value::Array(
                        request
                            .options
                            .iter()
                            .map(|value| Value::String(encode_text(value)))
                            .collect(),
                    ),
                );
                if !request.value.is_empty() {
                    field.insert(b"V".to_vec(), Value::String(encode_text(&request.value)));
                }
            }
            FormFieldKind::Signature => {}
        }
        let widget = BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"Annot".to_vec())),
            (b"Subtype".to_vec(), Value::Name(b"Widget".to_vec())),
            (b"Parent".to_vec(), Value::Ref(field_ref)),
            (b"P".to_vec(), Value::Ref(page_ref)),
            (b"Rect".to_vec(), number_array(&request.rect)),
            (b"F".to_vec(), Value::Integer(4)),
            (b"AP".to_vec(), appearance),
            (b"AS".to_vec(), Value::Name(b"Off".to_vec())),
        ]);
        parsed
            .objects
            .insert(field_ref, plain_object(Value::Dict(field)));
        parsed
            .objects
            .insert(widget_ref, plain_object(Value::Dict(widget)));
        add_field_to_acroform(&mut parsed, root_ref, field_ref)?;
        add_widget_to_page(&mut parsed, page_ref, widget_ref)?;

        let bytes = write_lifecycle(self, parsed)?;
        let rewritten = reopen(self, &bytes, "created form field")?;
        let fields = list_form_fields(&rewritten)?;
        let selected = fields.iter().rfind(|field| field.name == request.name);
        let widgets_reachable = selected.is_some_and(|field| {
            field.widget_refs.iter().any(|widget| {
                widget.object_number == widget_ref.number
                    && widget.object_generation == widget_ref.generation
            })
        }) && page_has_annot(rewritten.parsed(), page_ref, widget_ref)?;
        let reachable = normal_appearance_refs(rewritten.parsed(), widget_ref)?;
        let appearances_reachable = !reachable.is_empty()
            && reachable
                .iter()
                .all(|reference| rewritten.parsed().objects.contains_key(reference));
        lifecycle_outcome(
            self,
            &rewritten,
            bytes,
            FormLifecycleReport {
                operation: "create_form_field".into(),
                field_name: request.name,
                field_type: String::from_utf8_lossy(field_type(request.kind)).into_owned(),
                page_index: request.page_index,
                fields_affected: 1,
                widgets_affected: 1,
                appearances_placed: reachable.len(),
                input_bytes: self.source_len(),
                output_bytes: 0,
            },
            fields.len() == old_count + 1,
            widgets_reachable,
            appearances_reachable,
        )
    }

    pub fn remove_form_field(
        &self,
        request: FormFieldRemoveRequest,
    ) -> Result<FormLifecycleOutcome, PdfError> {
        refuse_form_lifecycle_security(self)?;
        if request.field_name.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "form field removal requires a name",
            ));
        }
        let fields = list_form_fields(self)?;
        let selected = fields
            .iter()
            .filter(|field| field.name == request.field_name)
            .nth(request.match_index)
            .ok_or_else(|| selection_not_found("form field", request.match_index))?;
        let field_ref = field_reference(selected)?;
        if selected.widget_refs.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "form field removal requires at least one indirect widget",
            ));
        }
        let widgets = selected
            .widget_refs
            .iter()
            .map(widget_reference)
            .collect::<Vec<_>>();
        let old_count = fields.len();
        let old_pages = self.page_refs()?;
        let root = root_ref(self.parsed())?;
        let mut parsed = self.parsed().clone();
        remove_top_level_field(&mut parsed, root, field_ref)?;
        for widget in &widgets {
            remove_widget_from_pages(&mut parsed, &old_pages, *widget)?;
            parsed.objects.remove(widget);
        }
        parsed.objects.remove(&field_ref);
        remove_unreferenced_appearances(&mut parsed, self.parsed(), &widgets)?;
        remove_empty_acroform(&mut parsed, root)?;
        let bytes = write_lifecycle(self, parsed)?;
        let rewritten = reopen(self, &bytes, "removed form field")?;
        let remaining = list_form_fields(&rewritten)?;
        let widgets_reachable = widgets.iter().all(|widget| {
            !old_pages
                .iter()
                .any(|page| page_has_annot(rewritten.parsed(), *page, *widget).unwrap_or(true))
        });
        lifecycle_outcome(
            self,
            &rewritten,
            bytes,
            FormLifecycleReport {
                operation: "remove_form_field".into(),
                field_name: request.field_name,
                field_type: selected.field_type.clone().unwrap_or_default(),
                page_index: 0,
                fields_affected: 1,
                widgets_affected: widgets.len(),
                appearances_placed: 0,
                input_bytes: self.source_len(),
                output_bytes: 0,
            },
            remaining.len() + 1 == old_count,
            widgets_reachable,
            true,
        )
    }

    pub fn flatten_form_fields(&self) -> Result<FormLifecycleOutcome, PdfError> {
        refuse_form_lifecycle_security(self)?;
        let fields = list_form_fields(self)?;
        if fields.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "PDF has no AcroForm fields to flatten",
            ));
        }
        let pages = self.page_refs()?;
        let root = root_ref(self.parsed())?;
        let mut parsed = self.parsed().clone();
        let mut paints: BTreeMap<ObjectRef, PagePaint> = BTreeMap::new();
        let mut widget_refs = BTreeSet::new();
        let mut field_refs = BTreeSet::new();
        let mut appearance_refs = BTreeSet::new();
        let mut placed = 0usize;
        for field in &fields {
            field_refs.insert(field_reference(field)?);
            for widget in &field.widget_refs {
                let widget_ref = widget_reference(widget);
                widget_refs.insert(widget_ref);
                let (page, appearance, bbox, rect) = flatten_target(self.parsed(), widget_ref)?;
                appearance_refs.insert(appearance);
                if !pages.contains(&page) {
                    return Err(PdfError::unsafe_rewrite(
                        "form widget /P does not reference a document page",
                    ));
                }
                let name = format!("Fm{}", appearance.number).into_bytes();
                let entry = paints.entry(page).or_default();
                entry.xobjects.insert(name.clone(), Value::Ref(appearance));
                entry
                    .commands
                    .extend_from_slice(&paint_command(&name, bbox, rect)?);
                placed += 1;
            }
        }
        let new_refs = allocate_references(&parsed, paints.len())?;
        for ((page, paint), content_ref) in paints.into_iter().zip(new_refs) {
            add_flattened_content(
                &mut parsed,
                page,
                content_ref,
                paint.xobjects,
                paint.commands,
            )?;
        }
        for widget in &widget_refs {
            remove_widget_from_pages(&mut parsed, &pages, *widget)?;
            parsed.objects.remove(widget);
        }
        for field in &field_refs {
            parsed.objects.remove(field);
        }
        remove_acroform(&mut parsed, root)?;
        let bytes = write_lifecycle(self, parsed)?;
        let rewritten = reopen(self, &bytes, "flattened form fields")?;
        let flattened = list_form_fields(&rewritten)?.is_empty();
        lifecycle_outcome(
            self,
            &rewritten,
            bytes,
            FormLifecycleReport {
                operation: "flatten_form_fields".into(),
                field_name: String::new(),
                field_type: String::new(),
                page_index: 0,
                fields_affected: fields.len(),
                widgets_affected: widget_refs.len(),
                appearances_placed: placed,
                input_bytes: self.source_len(),
                output_bytes: 0,
            },
            flattened,
            widget_refs
                .iter()
                .all(|reference| !rewritten.parsed().objects.contains_key(reference)),
            !appearance_refs.is_empty()
                && appearance_refs.iter().all(|reference| {
                    rewritten
                        .parsed()
                        .objects
                        .values()
                        .any(|object| contains_reference(&object.value, *reference))
                }),
        )
    }
}

fn validate_create_request(
    request: &FormFieldCreateRequest,
    limits: &crate::Limits,
) -> Result<(), PdfError> {
    if request.name.is_empty() || request.name.len() > limits.max_token_bytes {
        return Err(PdfError::unsafe_rewrite(
            "form field name is empty or exceeds limits",
        ));
    }
    if request.rect.iter().any(|value| !value.is_finite())
        || request.rect[0] >= request.rect[2]
        || request.rect[1] >= request.rect[3]
    {
        return Err(PdfError::unsafe_rewrite(
            "form field rectangle must be finite with x1 < x2 and y1 < y2",
        ));
    }
    if request.value.len() > limits.max_token_bytes
        || request.options.len() > limits.max_container_items
        || request
            .options
            .iter()
            .any(|value| value.len() > limits.max_token_bytes)
    {
        return Err(PdfError::limit("form field value or options exceed limits"));
    }
    if request.kind == FormFieldKind::Choice
        && !request.value.is_empty()
        && !request.options.iter().any(|value| value == &request.value)
    {
        return Err(PdfError::unsafe_rewrite(
            "choice field value must be present in options",
        ));
    }
    if request.kind != FormFieldKind::Choice && !request.options.is_empty() {
        return Err(PdfError::unsafe_rewrite(
            "only choice fields accept options",
        ));
    }
    if matches!(request.kind, FormFieldKind::Text | FormFieldKind::Choice)
        && !request.value.is_ascii()
    {
        return Err(PdfError::unsafe_rewrite(
            "generated text and choice appearances currently require ASCII values",
        ));
    }
    if matches!(
        request.kind,
        FormFieldKind::Checkbox | FormFieldKind::Radio | FormFieldKind::Signature
    ) && !request.value.is_empty()
    {
        return Err(PdfError::unsafe_rewrite(
            "button and signature field creation does not accept a text value",
        ));
    }
    Ok(())
}

fn field_type(kind: FormFieldKind) -> &'static [u8] {
    match kind {
        FormFieldKind::Text => b"Tx",
        FormFieldKind::Checkbox | FormFieldKind::Radio => b"Btn",
        FormFieldKind::Choice => b"Ch",
        FormFieldKind::Signature => b"Sig",
    }
}

fn create_object_count(kind: FormFieldKind) -> usize {
    match kind {
        FormFieldKind::Text | FormFieldKind::Choice => 4,
        FormFieldKind::Checkbox | FormFieldKind::Radio => 4,
        FormFieldKind::Signature => 3,
    }
}

fn allocate_references(parsed: &ParsedDocument, count: usize) -> Result<Vec<ObjectRef>, PdfError> {
    if count > parsed.limits.max_container_items
        || parsed
            .objects
            .len()
            .checked_add(count)
            .is_none_or(|value| value > parsed.limits.max_objects)
    {
        return Err(PdfError::limit(
            "form lifecycle object allocation exceeds limits",
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
            .ok_or_else(|| PdfError::limit("form object number overflows"))?;
        if usize::try_from(number)
            .ok()
            .and_then(|value| value.checked_add(1))
            .is_none_or(|value| value > parsed.limits.max_xref_entries)
        {
            return Err(PdfError::limit("form object allocation exceeds xref limit"));
        }
        output.push(ObjectRef {
            number,
            generation: 0,
        });
    }
    Ok(output)
}

fn create_appearances(
    request: &FormFieldCreateRequest,
    references: &[ObjectRef],
) -> Result<(Value, Vec<(ObjectRef, IndirectObject)>), PdfError> {
    let width = request.rect[2] - request.rect[0];
    let height = request.rect[3] - request.rect[1];
    let bbox = [0.0, 0.0, width, height];
    let form = |stream: Vec<u8>, resources: Value| IndirectObject {
        value: Value::Dict(BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Value::Name(b"Form".to_vec())),
            (b"FormType".to_vec(), Value::Integer(1)),
            (b"BBox".to_vec(), number_array(&bbox)),
            (b"Resources".to_vec(), resources),
        ])),
        stream: Some(stream),
        stream_offset: 0,
        offset: 0,
    };
    match request.kind {
        FormFieldKind::Text | FormFieldKind::Choice => {
            let [appearance, font] = references else {
                return Err(PdfError::verification(
                    "text appearance allocation mismatch",
                ));
            };
            let font_object = plain_object(Value::Dict(BTreeMap::from([
                (b"Type".to_vec(), Value::Name(b"Font".to_vec())),
                (b"Subtype".to_vec(), Value::Name(b"Type1".to_vec())),
                (b"BaseFont".to_vec(), Value::Name(b"Helvetica".to_vec())),
                (
                    b"Encoding".to_vec(),
                    Value::Name(b"WinAnsiEncoding".to_vec()),
                ),
            ])));
            let resources = Value::Dict(BTreeMap::from([(
                b"Font".to_vec(),
                Value::Dict(BTreeMap::from([(b"Helv".to_vec(), Value::Ref(*font))])),
            )]));
            let stream = format!(
                "q BT /Helv 12 Tf 2 2 Td ({}) Tj ET Q\n",
                escape_literal(&request.value)
            )
            .into_bytes();
            Ok((
                Value::Dict(BTreeMap::from([(b"N".to_vec(), Value::Ref(*appearance))])),
                vec![(*appearance, form(stream, resources)), (*font, font_object)],
            ))
        }
        FormFieldKind::Checkbox | FormFieldKind::Radio => {
            let [off, on] = references else {
                return Err(PdfError::verification(
                    "button appearance allocation mismatch",
                ));
            };
            let off_stream = format!("q 0 0 {width} {height} re S Q\n").into_bytes();
            let on_stream = format!(
                "q 0 0 {width} {height} re S 0 0 m {width} {height} l S 0 {height} m {width} 0 l S Q\n"
            )
            .into_bytes();
            let normal = Value::Dict(BTreeMap::from([
                (b"Off".to_vec(), Value::Ref(*off)),
                (b"Yes".to_vec(), Value::Ref(*on)),
            ]));
            Ok((
                Value::Dict(BTreeMap::from([(b"N".to_vec(), normal)])),
                vec![
                    (*off, form(off_stream, Value::Dict(BTreeMap::new()))),
                    (*on, form(on_stream, Value::Dict(BTreeMap::new()))),
                ],
            ))
        }
        FormFieldKind::Signature => {
            let [appearance] = references else {
                return Err(PdfError::verification(
                    "signature appearance allocation mismatch",
                ));
            };
            let stream = format!("q 0 0 {width} {height} re S Q\n").into_bytes();
            Ok((
                Value::Dict(BTreeMap::from([(b"N".to_vec(), Value::Ref(*appearance))])),
                vec![(*appearance, form(stream, Value::Dict(BTreeMap::new())))],
            ))
        }
    }
}

fn escape_literal(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'(' | b')' | b'\\' => vec![b'\\', byte],
            0x20..=0x7e => vec![byte],
            _ => vec![b'?'],
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

fn root_ref(parsed: &ParsedDocument) -> Result<ObjectRef, PdfError> {
    let trailer = dictionary(&parsed.trailer, None, "trailer")?;
    match trailer.get(b"Root".as_slice()) {
        Some(Value::Ref(reference)) => Ok(*reference),
        _ => Err(PdfError::unsafe_rewrite(
            "form lifecycle requires an indirect catalog",
        )),
    }
}

fn add_field_to_acroform(
    parsed: &mut ParsedDocument,
    root: ObjectRef,
    field: ObjectRef,
) -> Result<(), PdfError> {
    let catalog_object = parsed.object(root)?.clone();
    let mut catalog = dictionary(&catalog_object.value, Some(root), "catalog")?.clone();
    match catalog.get(b"AcroForm".as_slice()).cloned() {
        None | Some(Value::Null) => {
            let acro_ref = allocate_references(parsed, 1)?[0];
            let acro = Value::Dict(BTreeMap::from([
                (b"Fields".to_vec(), Value::Array(vec![Value::Ref(field)])),
                (b"NeedAppearances".to_vec(), Value::Bool(false)),
            ]));
            parsed.objects.insert(acro_ref, plain_object(acro));
            catalog.insert(b"AcroForm".to_vec(), Value::Ref(acro_ref));
        }
        Some(Value::Ref(reference)) => {
            let object = parsed.object(reference)?.clone();
            let mut acro = dictionary(&object.value, Some(reference), "AcroForm")?.clone();
            append_field(&mut acro, field, Some(reference))?;
            parsed
                .objects
                .insert(reference, plain_object(Value::Dict(acro)));
        }
        Some(Value::Dict(mut acro)) => {
            append_field(&mut acro, field, Some(root))?;
            catalog.insert(b"AcroForm".to_vec(), Value::Dict(acro));
        }
        Some(_) => {
            return Err(PdfError::unsafe_rewrite(
                "catalog /AcroForm must be a dictionary or reference",
            ));
        }
    }
    parsed
        .objects
        .insert(root, plain_object(Value::Dict(catalog)));
    Ok(())
}

fn append_field(
    acro: &mut BTreeMap<Vec<u8>, Value>,
    field: ObjectRef,
    owner: Option<ObjectRef>,
) -> Result<(), PdfError> {
    if acro.contains_key(b"XFA".as_slice()) {
        return Err(PdfError::unsafe_rewrite(
            "XFA forms are not supported by form lifecycle mutation",
        ));
    }
    match acro
        .entry(b"Fields".to_vec())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(fields) => fields.push(Value::Ref(field)),
        _ => return Err(malformed("AcroForm /Fields must be a direct array", owner)),
    }
    acro.insert(b"NeedAppearances".to_vec(), Value::Bool(false));
    Ok(())
}

fn add_widget_to_page(
    parsed: &mut ParsedDocument,
    page: ObjectRef,
    widget: ObjectRef,
) -> Result<(), PdfError> {
    let object = parsed.object(page)?.clone();
    let mut dict = dictionary(&object.value, Some(page), "page")?.clone();
    match dict
        .entry(b"Annots".to_vec())
        .or_insert_with(|| Value::Array(Vec::new()))
    {
        Value::Array(values) => values.push(Value::Ref(widget)),
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "page /Annots must be a direct array",
            ));
        }
    }
    parsed.objects.insert(page, plain_object(Value::Dict(dict)));
    Ok(())
}

fn field_reference(field: &FormField) -> Result<ObjectRef, PdfError> {
    match (field.object_number, field.object_generation) {
        (Some(number), Some(generation)) => Ok(ObjectRef { number, generation }),
        _ => Err(PdfError::unsafe_rewrite(
            "form lifecycle requires indirect fields",
        )),
    }
}

fn widget_reference(widget: &FormWidgetRef) -> ObjectRef {
    ObjectRef {
        number: widget.object_number,
        generation: widget.object_generation,
    }
}

fn mutate_acroform_fields(
    parsed: &mut ParsedDocument,
    root: ObjectRef,
    mutation: impl FnOnce(&mut Vec<Value>) -> Result<(), PdfError>,
) -> Result<(), PdfError> {
    let root_object = parsed.object(root)?.clone();
    let mut catalog = dictionary(&root_object.value, Some(root), "catalog")?.clone();
    let acro_value = catalog
        .get(b"AcroForm".as_slice())
        .cloned()
        .ok_or_else(|| PdfError::unsafe_rewrite("catalog has no /AcroForm"))?;
    match acro_value {
        Value::Ref(reference) => {
            let object = parsed.object(reference)?.clone();
            let mut acro = dictionary(&object.value, Some(reference), "AcroForm")?.clone();
            let Value::Array(fields) = acro
                .get_mut(b"Fields".as_slice())
                .ok_or_else(|| malformed("AcroForm has no /Fields", Some(reference)))?
            else {
                return Err(malformed(
                    "AcroForm /Fields must be a direct array",
                    Some(reference),
                ));
            };
            mutation(fields)?;
            parsed
                .objects
                .insert(reference, plain_object(Value::Dict(acro)));
        }
        Value::Dict(mut acro) => {
            let Value::Array(fields) = acro
                .get_mut(b"Fields".as_slice())
                .ok_or_else(|| malformed("AcroForm has no /Fields", Some(root)))?
            else {
                return Err(malformed(
                    "AcroForm /Fields must be a direct array",
                    Some(root),
                ));
            };
            mutation(fields)?;
            catalog.insert(b"AcroForm".to_vec(), Value::Dict(acro));
            parsed
                .objects
                .insert(root, plain_object(Value::Dict(catalog)));
        }
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "catalog /AcroForm must be a dictionary or reference",
            ));
        }
    }
    Ok(())
}

fn remove_top_level_field(
    parsed: &mut ParsedDocument,
    root: ObjectRef,
    field: ObjectRef,
) -> Result<(), PdfError> {
    mutate_acroform_fields(parsed, root, |fields| {
        let before = fields.len();
        fields.retain(|value| !matches!(value, Value::Ref(reference) if *reference == field));
        if fields.len() == before {
            return Err(PdfError::unsafe_rewrite(
                "selected field is not a top-level AcroForm field",
            ));
        }
        Ok(())
    })
}

fn remove_widget_from_pages(
    parsed: &mut ParsedDocument,
    pages: &[ObjectRef],
    widget: ObjectRef,
) -> Result<(), PdfError> {
    let mut removed = false;
    for page in pages {
        let object = parsed.object(*page)?.clone();
        let mut dict = dictionary(&object.value, Some(*page), "page")?.clone();
        if let Some(value) = dict.get_mut(b"Annots".as_slice()) {
            let Value::Array(annots) = value else {
                return Err(PdfError::unsafe_rewrite(
                    "page /Annots must be a direct array",
                ));
            };
            let before = annots.len();
            annots.retain(|value| !matches!(value, Value::Ref(reference) if *reference == widget));
            removed |= annots.len() != before;
            if annots.is_empty() {
                dict.remove(b"Annots".as_slice());
            }
            parsed
                .objects
                .insert(*page, plain_object(Value::Dict(dict)));
        }
    }
    if !removed {
        return Err(PdfError::unsafe_rewrite(
            "form widget is not present in any page /Annots",
        ));
    }
    Ok(())
}

fn page_has_annot(
    parsed: &ParsedDocument,
    page: ObjectRef,
    widget: ObjectRef,
) -> Result<bool, PdfError> {
    let dict = dictionary(&parsed.object(page)?.value, Some(page), "page")?;
    Ok(
        matches!(dict.get(b"Annots".as_slice()), Some(Value::Array(values)) if values.iter().any(|value| matches!(value, Value::Ref(reference) if *reference == widget))),
    )
}

fn remove_empty_acroform(parsed: &mut ParsedDocument, root: ObjectRef) -> Result<(), PdfError> {
    let catalog_object = parsed.object(root)?.clone();
    let catalog = dictionary(&catalog_object.value, Some(root), "catalog")?;
    let empty = match catalog.get(b"AcroForm".as_slice()) {
        Some(Value::Ref(reference)) => {
            let acro = dictionary(
                &parsed.object(*reference)?.value,
                Some(*reference),
                "AcroForm",
            )?;
            matches!(acro.get(b"Fields".as_slice()), Some(Value::Array(fields)) if fields.is_empty())
        }
        Some(Value::Dict(acro)) => {
            matches!(acro.get(b"Fields".as_slice()), Some(Value::Array(fields)) if fields.is_empty())
        }
        _ => false,
    };
    if empty {
        remove_acroform(parsed, root)?;
    }
    Ok(())
}

fn remove_acroform(parsed: &mut ParsedDocument, root: ObjectRef) -> Result<(), PdfError> {
    let object = parsed.object(root)?.clone();
    let mut catalog = dictionary(&object.value, Some(root), "catalog")?.clone();
    if let Some(Value::Ref(reference)) = catalog.remove(b"AcroForm".as_slice()) {
        parsed.objects.remove(&reference);
    }
    parsed
        .objects
        .insert(root, plain_object(Value::Dict(catalog)));
    Ok(())
}

fn normal_appearance_refs(
    parsed: &ParsedDocument,
    widget: ObjectRef,
) -> Result<Vec<ObjectRef>, PdfError> {
    let dict = dictionary(&parsed.object(widget)?.value, Some(widget), "widget")?;
    let (ap, _) = resolve_dict(
        parsed,
        dict.get(b"AP".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("widget has no /AP"))?,
        "widget /AP",
    )?;
    match ap.get(b"N".as_slice()) {
        Some(Value::Ref(reference)) => Ok(vec![*reference]),
        Some(Value::Dict(values)) => values
            .values()
            .map(|value| match value {
                Value::Ref(reference) => Ok(*reference),
                _ => Err(PdfError::unsafe_rewrite(
                    "widget state appearance must be indirect",
                )),
            })
            .collect(),
        _ => Err(PdfError::unsafe_rewrite(
            "widget /AP /N must be an indirect form or state dictionary",
        )),
    }
}

fn remove_unreferenced_appearances(
    parsed: &mut ParsedDocument,
    original: &ParsedDocument,
    widgets: &[ObjectRef],
) -> Result<(), PdfError> {
    let candidates = widgets
        .iter()
        .map(|widget| normal_appearance_refs(original, *widget))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>();
    for candidate in candidates {
        if !parsed
            .objects
            .values()
            .any(|object| contains_reference(&object.value, candidate))
        {
            parsed.objects.remove(&candidate);
        }
    }
    Ok(())
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

fn flatten_target(
    parsed: &ParsedDocument,
    widget: ObjectRef,
) -> Result<(ObjectRef, ObjectRef, [f64; 4], [f64; 4]), PdfError> {
    let dict = dictionary(&parsed.object(widget)?.value, Some(widget), "widget")?;
    let page = match dict.get(b"P".as_slice()) {
        Some(Value::Ref(reference)) => *reference,
        _ => return Err(PdfError::unsafe_rewrite("widget has no indirect /P page")),
    };
    let rect = numeric_box(dict.get(b"Rect".as_slice()), widget, "widget /Rect")?;
    let (ap, _) = resolve_dict(
        parsed,
        dict.get(b"AP".as_slice())
            .ok_or_else(|| PdfError::unsafe_rewrite("widget has no verified appearance"))?,
        "widget /AP",
    )?;
    let normal = ap
        .get(b"N".as_slice())
        .ok_or_else(|| PdfError::unsafe_rewrite("widget has no normal appearance"))?;
    let appearance = match normal {
        Value::Ref(reference) => *reference,
        Value::Dict(states) => {
            let state = match dict.get(b"AS".as_slice()) {
                Some(Value::Name(value)) => value.as_slice(),
                _ => b"Off",
            };
            match states.get(state) {
                Some(Value::Ref(reference)) => *reference,
                _ => {
                    return Err(PdfError::unsafe_rewrite(
                        "widget selected appearance state is missing",
                    ));
                }
            }
        }
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "widget normal appearance is unsupported",
            ));
        }
    };
    let object = parsed.object(appearance)?;
    let appearance_dict = dictionary(&object.value, Some(appearance), "appearance")?;
    if object.stream.is_none()
        || appearance_dict.contains_key(b"Filter".as_slice())
        || !matches!(appearance_dict.get(b"Subtype".as_slice()), Some(Value::Name(value)) if value == b"Form")
    {
        return Err(PdfError::unsafe_rewrite(
            "flattening requires an unfiltered Form XObject appearance",
        ));
    }
    if let Some(matrix) = appearance_dict.get(b"Matrix".as_slice())
        && numeric_values(matrix) != Some(vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    {
        return Err(PdfError::unsafe_rewrite(
            "flattening requires an absent or identity appearance /Matrix",
        ));
    }
    if dict.contains_key(b"MK".as_slice()) {
        return Err(PdfError::unsafe_rewrite(
            "flattening widgets with appearance characteristics is not supported",
        ));
    }
    let bbox = numeric_box(
        appearance_dict.get(b"BBox".as_slice()),
        appearance,
        "appearance /BBox",
    )?;
    if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
        return Err(PdfError::unsafe_rewrite(
            "appearance /BBox must have positive dimensions",
        ));
    }
    Ok((page, appearance, bbox, rect))
}

fn numeric_box(value: Option<&Value>, owner: ObjectRef, label: &str) -> Result<[f64; 4], PdfError> {
    let Some(Value::Array(values)) = value else {
        return Err(malformed(format!("{label} must be an array"), Some(owner)));
    };
    if values.len() != 4 {
        return Err(malformed(
            format!("{label} must contain four numbers"),
            Some(owner),
        ));
    }
    let mut output = [0.0; 4];
    for (index, value) in values.iter().enumerate() {
        output[index] = match value {
            Value::Integer(value) => *value as f64,
            Value::Real(value) => *value,
            _ => {
                return Err(malformed(
                    format!("{label} must contain numbers"),
                    Some(owner),
                ));
            }
        };
        if !output[index].is_finite() {
            return Err(malformed(format!("{label} must be finite"), Some(owner)));
        }
    }
    Ok(output)
}

fn numeric_values(value: &Value) -> Option<Vec<f64>> {
    let Value::Array(values) = value else {
        return None;
    };
    values
        .iter()
        .map(|value| match value {
            Value::Integer(value) => Some(*value as f64),
            Value::Real(value) if value.is_finite() => Some(*value),
            _ => None,
        })
        .collect()
}

fn paint_command(name: &[u8], bbox: [f64; 4], rect: [f64; 4]) -> Result<Vec<u8>, PdfError> {
    let sx = (rect[2] - rect[0]) / (bbox[2] - bbox[0]);
    let sy = (rect[3] - rect[1]) / (bbox[3] - bbox[1]);
    let tx = rect[0] - bbox[0] * sx;
    let ty = rect[1] - bbox[1] * sy;
    if [sx, sy, tx, ty].iter().any(|value| !value.is_finite()) {
        return Err(PdfError::unsafe_rewrite(
            "appearance placement matrix is not finite",
        ));
    }
    Ok(format!(
        "q {sx} 0 0 {sy} {tx} {ty} cm /{} Do Q\n",
        String::from_utf8_lossy(name)
    )
    .into_bytes())
}

fn add_flattened_content(
    parsed: &mut ParsedDocument,
    page: ObjectRef,
    content: ObjectRef,
    xobjects: BTreeMap<Vec<u8>, Value>,
    commands: Vec<u8>,
) -> Result<(), PdfError> {
    if commands.len() > parsed.limits.max_stream_bytes {
        return Err(PdfError::limit(
            "flattened appearance content exceeds stream limit",
        ));
    }
    let object = parsed.object(page)?.clone();
    let mut dict = dictionary(&object.value, Some(page), "page")?.clone();
    let mut resources = inherited_page_resources(parsed, page)?;
    let mut existing = match resources.get(b"XObject".as_slice()) {
        None => BTreeMap::new(),
        Some(value) => resolve_dict(parsed, value, "page /Resources /XObject")?
            .0
            .clone(),
    };
    for (name, value) in xobjects {
        if existing.insert(name, value).is_some() {
            return Err(PdfError::unsafe_rewrite(
                "flattened appearance resource name collides",
            ));
        }
    }
    resources.insert(b"XObject".to_vec(), Value::Dict(existing));
    dict.insert(b"Resources".to_vec(), Value::Dict(resources));
    match dict.remove(b"Contents".as_slice()) {
        None => {
            dict.insert(b"Contents".to_vec(), Value::Ref(content));
        }
        Some(Value::Ref(reference)) => {
            dict.insert(
                b"Contents".to_vec(),
                Value::Array(vec![Value::Ref(reference), Value::Ref(content)]),
            );
        }
        Some(Value::Array(mut values)) => {
            values.push(Value::Ref(content));
            dict.insert(b"Contents".to_vec(), Value::Array(values));
        }
        Some(_) => {
            return Err(PdfError::unsafe_rewrite(
                "page /Contents must be an indirect stream or direct array",
            ));
        }
    }
    parsed.objects.insert(page, plain_object(Value::Dict(dict)));
    parsed.objects.insert(
        content,
        IndirectObject {
            value: Value::Dict(BTreeMap::new()),
            stream: Some(commands),
            stream_offset: 0,
            offset: 0,
        },
    );
    Ok(())
}

fn inherited_page_resources(
    parsed: &ParsedDocument,
    page: ObjectRef,
) -> Result<BTreeMap<Vec<u8>, Value>, PdfError> {
    let mut current = page;
    let mut seen = BTreeSet::new();
    loop {
        if seen.len() > parsed.limits.max_parser_depth {
            return Err(PdfError::limit("page resource inheritance exceeds limit"));
        }
        if !seen.insert(current) {
            return Err(PdfError::unsafe_rewrite(
                "cycle in page resource inheritance",
            ));
        }
        let dict = dictionary(
            &parsed.object(current)?.value,
            Some(current),
            "page resource owner",
        )?;
        if let Some(value) = dict.get(b"Resources".as_slice()) {
            return Ok(resolve_dict(parsed, value, "page /Resources")?.0.clone());
        }
        current = match dict.get(b"Parent".as_slice()) {
            Some(Value::Ref(reference)) => *reference,
            _ => return Ok(BTreeMap::new()),
        };
    }
}

fn write_lifecycle(document: &PdfDocument, parsed: ParsedDocument) -> Result<Vec<u8>, PdfError> {
    write_encrypted_pdf(document, &parsed)
}

fn reopen(document: &PdfDocument, bytes: &[u8], label: &str) -> Result<PdfDocument, PdfError> {
    PdfEngine::new(document.engine_config().clone())
        .open(bytes, OpenOptions::default())
        .map_err(|error| PdfError::verification(format!("{label} output did not reparse: {error}")))
}

fn lifecycle_outcome(
    original: &PdfDocument,
    rewritten: &PdfDocument,
    bytes: Vec<u8>,
    mut report: FormLifecycleReport,
    expected_field_count: bool,
    widgets_reachable: bool,
    appearances_reachable: bool,
) -> Result<FormLifecycleOutcome, PdfError> {
    let page_count_unchanged = original.page_count()? == rewritten.page_count()?;
    let no_dangling_references = all_references_resolve(rewritten.parsed());
    let verification = FormLifecycleVerification {
        passed: page_count_unchanged
            && expected_field_count
            && widgets_reachable
            && appearances_reachable
            && no_dangling_references,
        reparsed: true,
        page_count_unchanged,
        expected_field_count,
        widgets_reachable,
        appearances_reachable,
        no_dangling_references,
    };
    if !verification.passed {
        return Err(PdfError::verification(
            "form lifecycle mutation failed post-write verification",
        ));
    }
    report.output_bytes = bytes.len();
    Ok(FormLifecycleOutcome {
        bytes,
        report,
        verification,
    })
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

fn refuse_form_lifecycle_security(document: &PdfDocument) -> Result<(), PdfError> {
    let trailer = dictionary(&document.parsed().trailer, None, "trailer")?;
    if trailer.contains_key(b"Encrypt".as_slice()) {
        return Err(PdfError::unsafe_rewrite(
            "form lifecycle mutation of encrypted PDFs is not implemented",
        ));
    }
    if document
        .parsed()
        .objects
        .values()
        .any(|object| contains_signed_value(&object.value))
    {
        return Err(PdfError::unsafe_rewrite(
            "form lifecycle mutation of signed PDFs requires an explicit signature policy",
        ));
    }
    Ok(())
}

fn contains_signed_value(value: &Value) -> bool {
    match value {
        Value::Array(values) => values.iter().any(contains_signed_value),
        Value::Dict(values) => {
            values.contains_key(b"ByteRange".as_slice())
                || matches!(values.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Sig")
                || values.values().any(contains_signed_value)
        }
        _ => false,
    }
}

fn require_need_appearances(parsed: &ParsedDocument) -> Result<(), PdfError> {
    let trailer = dictionary(&parsed.trailer, None, "trailer")?;
    let root = trailer
        .get(b"Root".as_slice())
        .ok_or_else(|| malformed("trailer has no /Root", None))?;
    let (catalog, catalog_ref) = resolve_dict(parsed, root, "catalog")?;
    let acro_form = catalog
        .get(b"AcroForm".as_slice())
        .ok_or_else(|| malformed("catalog has no /AcroForm", catalog_ref))?;
    let (acro_form, _) = resolve_dict(parsed, acro_form, "catalog /AcroForm")?;
    if !matches!(
        acro_form.get(b"NeedAppearances".as_slice()),
        Some(Value::Bool(true))
    ) {
        return Err(PdfError::unsafe_rewrite(
            "form mutation requires AcroForm /NeedAppearances true",
        ));
    }
    Ok(())
}

fn field_or_widget_has_appearance(
    parsed: &ParsedDocument,
    dict: &BTreeMap<Vec<u8>, Value>,
    reference: Option<ObjectRef>,
) -> Result<bool, PdfError> {
    if dict.contains_key(b"AP".as_slice()) {
        return Ok(true);
    }
    let Some(kids) = dict.get(b"Kids".as_slice()) else {
        return Ok(false);
    };
    for kid in resolve_array(parsed, kids, reference, "field /Kids")? {
        let (kid, _) = resolve_dict(parsed, kid, "field widget")?;
        if matches!(kid.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Widget")
            && kid.contains_key(b"AP".as_slice())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn button_mutation_plan(
    parsed: &ParsedDocument,
    field_ref: ObjectRef,
    field: &BTreeMap<Vec<u8>, Value>,
) -> Result<ButtonMutationPlan, PdfError> {
    refuse_button_actions(field, field_ref, "button field")?;
    if field.contains_key(b"AP".as_slice()) {
        if field.contains_key(b"Kids".as_slice()) {
            return Err(PdfError::unsafe_rewrite(
                "button field with both /AP and /Kids is unsupported",
            ));
        }
        let on_state = button_widget_on_state(parsed, field, field_ref)?;
        validate_button_state(
            field,
            b"V",
            std::slice::from_ref(&on_state),
            field_ref,
            "button field",
        )?;
        validate_button_state(
            field,
            b"AS",
            std::slice::from_ref(&on_state),
            field_ref,
            "button widget",
        )?;
        return Ok(ButtonMutationPlan {
            field_is_widget: true,
            widgets: vec![ButtonWidget {
                reference: field_ref,
                dictionary: field.clone(),
                on_state,
            }],
        });
    }

    if field.contains_key(b"AS".as_slice()) {
        return Err(PdfError::unsafe_rewrite(
            "parent button field has /AS without a direct normal appearance",
        ));
    }
    let kids = field
        .get(b"Kids".as_slice())
        .ok_or_else(|| PdfError::unsafe_rewrite("button field has no /AP proof or widget /Kids"))?;
    let kids = resolve_array(parsed, kids, Some(field_ref), "button field /Kids")?;
    if kids.is_empty() {
        return Err(PdfError::unsafe_rewrite(
            "button field has no widget children",
        ));
    }
    if kids.len() > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "button field /Kids exceeds container limit",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut widgets = Vec::with_capacity(kids.len());
    for child in kids {
        let Value::Ref(widget_ref) = child else {
            return Err(PdfError::unsafe_rewrite(
                "button field widgets must be indirect references",
            ));
        };
        if !seen.insert(*widget_ref) {
            return Err(PdfError::unsafe_rewrite(
                "button field contains a duplicate widget reference",
            ));
        }
        let object = parsed.object(*widget_ref)?;
        if object.stream.is_some() {
            return Err(PdfError::unsafe_rewrite(
                "button widget dictionary must not be a stream",
            ));
        }
        let widget = dictionary(&object.value, Some(*widget_ref), "button widget")?.clone();
        if !matches!(widget.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Widget")
        {
            return Err(PdfError::unsafe_rewrite(
                "button field child is not a widget annotation",
            ));
        }
        if !matches!(widget.get(b"Parent".as_slice()), Some(Value::Ref(parent)) if *parent == field_ref)
        {
            return Err(PdfError::unsafe_rewrite(
                "button widget does not point back to its selected parent field",
            ));
        }
        if widget.contains_key(b"Kids".as_slice())
            || widget.contains_key(b"FT".as_slice())
            || widget.contains_key(b"Ff".as_slice())
        {
            return Err(PdfError::unsafe_rewrite(
                "nested or field-owning button widgets are unsupported",
            ));
        }
        refuse_button_actions(&widget, *widget_ref, "button widget")?;
        let on_state = button_widget_on_state(parsed, &widget, *widget_ref)?;
        validate_button_state(
            &widget,
            b"AS",
            std::slice::from_ref(&on_state),
            *widget_ref,
            "button widget",
        )?;
        widgets.push(ButtonWidget {
            reference: *widget_ref,
            dictionary: widget,
            on_state,
        });
    }

    let states = widgets
        .iter()
        .map(|widget| widget.on_state.clone())
        .collect::<Vec<_>>();
    if states.iter().collect::<BTreeSet<_>>().len() != states.len() {
        return Err(PdfError::unsafe_rewrite(
            "button field has duplicate checked appearance states",
        ));
    }
    validate_button_state(field, b"V", &states, field_ref, "button field")?;
    Ok(ButtonMutationPlan {
        field_is_widget: false,
        widgets,
    })
}

fn refuse_button_actions(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    reference: ObjectRef,
    label: &str,
) -> Result<(), PdfError> {
    if dictionary.contains_key(b"A".as_slice()) || dictionary.contains_key(b"AA".as_slice()) {
        return Err(PdfError::unsafe_rewrite(format!(
            "{label} actions require an explicit action policy ({})",
            reference.number
        )));
    }
    Ok(())
}

fn button_widget_on_state(
    parsed: &ParsedDocument,
    widget: &BTreeMap<Vec<u8>, Value>,
    widget_ref: ObjectRef,
) -> Result<Vec<u8>, PdfError> {
    let appearance = widget
        .get(b"AP".as_slice())
        .ok_or_else(|| PdfError::unsafe_rewrite("button widget has no /AP proof"))?;
    let (appearance, appearance_ref) = resolve_dict(parsed, appearance, "button widget /AP")?;
    if appearance_ref
        .and_then(|reference| parsed.object(reference).ok())
        .is_some_and(|object| object.stream.is_some())
    {
        return Err(PdfError::unsafe_rewrite(
            "button widget /AP must be a dictionary, not a stream",
        ));
    }
    if appearance.len() != 1 || !appearance.contains_key(b"N".as_slice()) {
        return Err(PdfError::unsafe_rewrite(
            "button widget supports only a normal (/AP /N) appearance",
        ));
    }
    let normal = appearance
        .get(b"N".as_slice())
        .ok_or_else(|| PdfError::verification("button normal appearance disappeared"))?;
    let (normal, normal_ref) = resolve_dict(parsed, normal, "button widget /AP /N")?;
    if normal_ref
        .and_then(|reference| parsed.object(reference).ok())
        .is_some_and(|object| object.stream.is_some())
    {
        return Err(PdfError::unsafe_rewrite(
            "button widget /AP /N must be a state dictionary",
        ));
    }
    if normal.len() > parsed.limits.max_container_items {
        return Err(PdfError::limit(
            "button widget normal appearance states exceed container limit",
        ));
    }

    let mut on_state = None;
    let mut has_off = false;
    for (state, appearance) in normal {
        if state.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "button widget has an empty appearance state name",
            ));
        }
        validate_button_appearance_stream(parsed, appearance, widget_ref)?;
        if state.as_slice() == b"Off" {
            has_off = true;
            continue;
        }
        if on_state.replace(state.clone()).is_some() {
            return Err(PdfError::unsafe_rewrite(
                "button widget has ambiguous checked appearance states",
            ));
        }
    }
    if !has_off {
        return Err(PdfError::unsafe_rewrite(
            "button widget has no /Off appearance state",
        ));
    }
    on_state.ok_or_else(|| {
        PdfError::unsafe_rewrite("button widget has no checked normal appearance state")
    })
}

fn validate_button_appearance_stream(
    parsed: &ParsedDocument,
    value: &Value,
    widget_ref: ObjectRef,
) -> Result<(), PdfError> {
    let Value::Ref(reference) = value else {
        return Err(PdfError::unsafe_rewrite(
            "button appearance states must be indirect Form XObject streams",
        ));
    };
    let object = parsed.object(*reference)?;
    let dictionary = dictionary(&object.value, Some(*reference), "button appearance state")?;
    if object.stream.is_none()
        || !matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"XObject")
        || !matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Form")
    {
        return Err(PdfError::unsafe_rewrite(format!(
            "button widget {} appearance state is not a Form XObject stream",
            widget_ref.number
        )));
    }
    Ok(())
}

fn validate_button_state(
    dictionary: &BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    on_states: &[Vec<u8>],
    reference: ObjectRef,
    label: &str,
) -> Result<(), PdfError> {
    let Some(value) = dictionary.get(key) else {
        return Ok(());
    };
    let Value::Name(state) = value else {
        return Err(malformed(
            format!("{label} /{} is not a name", String::from_utf8_lossy(key)),
            Some(reference),
        ));
    };
    if state.as_slice() == b"Off" || on_states.iter().any(|candidate| candidate == state) {
        return Ok(());
    }
    Err(PdfError::unsafe_rewrite(format!(
        "{label} /{} is not represented by a proven normal appearance state",
        String::from_utf8_lossy(key)
    )))
}

fn button_field_value_is(
    document: &PdfDocument,
    reference: ObjectRef,
    expected: &[u8],
) -> Result<bool, PdfError> {
    let object = document.parsed().object(reference)?;
    let dictionary = dictionary(&object.value, Some(reference), "button field")?;
    Ok(matches!(dictionary.get(b"V".as_slice()), Some(Value::Name(value)) if value == expected))
}

fn button_widget_state_is(
    document: &PdfDocument,
    reference: ObjectRef,
    expected: &[u8],
) -> Result<bool, PdfError> {
    let object = document.parsed().object(reference)?;
    let dictionary = dictionary(&object.value, Some(reference), "button widget")?;
    Ok(matches!(dictionary.get(b"AS".as_slice()), Some(Value::Name(value)) if value == expected))
}

pub(crate) fn refuse_unsafe_interactive_edit(document: &PdfDocument) -> Result<(), PdfError> {
    let trailer = dictionary(&document.parsed().trailer, None, "trailer")?;
    if trailer.contains_key(b"Encrypt".as_slice()) {
        return Err(PdfError::unsafe_rewrite(
            "interactive mutation of encrypted PDFs is not implemented",
        ));
    }
    if document
        .parsed()
        .objects
        .values()
        .any(|object| contains_signature(&object.value))
    {
        return Err(PdfError::unsafe_rewrite(
            "interactive mutation of signed PDFs requires an explicit signature policy",
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

fn selection_not_found(kind: &str, match_index: usize) -> PdfError {
    PdfError {
        code: PdfErrorCode::SelectionNotFound,
        message: format!("{kind} match index {match_index} was not found"),
        span: None,
        object: None,
    }
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

#[allow(clippy::too_many_arguments)]
fn walk_field(
    parsed: &ParsedDocument,
    value: &Value,
    inherited: &FieldContext,
    depth: usize,
    path: &mut BTreeSet<ObjectRef>,
    walked: &mut usize,
    output: &mut Vec<FormField>,
) -> Result<(), PdfError> {
    if depth > parsed.limits.max_parser_depth {
        return Err(PdfError::limit("AcroForm field tree depth exceeds limit"));
    }
    *walked = walked
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("AcroForm field count overflows"))?;
    if *walked > parsed.limits.max_container_items {
        return Err(PdfError::limit("AcroForm field count exceeds limit"));
    }

    let (dict, reference) = resolve_dict(parsed, value, "AcroForm field")?;
    if let Some(reference) = reference
        && !path.insert(reference)
    {
        return Err(malformed(
            "cycle in AcroForm field /Kids tree",
            Some(reference),
        ));
    }

    let result = (|| {
        let mut context = inherited.clone();
        if let Some(value) = dict.get(b"FT".as_slice()) {
            context.field_type = Some(name(value, reference, "field /FT")?);
        }
        if let Some(value) = dict.get(b"Ff".as_slice()) {
            context.flags = Some(nonnegative_u32(value, reference, "field /Ff")?);
        }

        let local_name = dict
            .get(b"T".as_slice())
            .map(|value| text_string(value, reference, "field /T"))
            .transpose()?;
        if let Some(local_name) = local_name {
            context.name = if inherited.name.is_empty() {
                local_name
            } else {
                format!("{}.{}", inherited.name, local_name)
            };
            let widgets = widget_refs(parsed, dict, reference)?;
            output.push(FormField {
                index: 0,
                name: context.name.clone(),
                object_number: reference.map(|value| value.number),
                object_generation: reference.map(|value| value.generation),
                field_type: context.field_type.clone(),
                flags: context.flags,
                value: field_value(dict.get(b"V".as_slice()), reference, "field /V")?,
                default_value: field_value(dict.get(b"DV".as_slice()), reference, "field /DV")?,
                widget_refs: widgets,
            });
        }

        if let Some(kids) = dict.get(b"Kids".as_slice()) {
            let kids = resolve_array(parsed, kids, reference, "field /Kids")?;
            if kids.len() > parsed.limits.max_container_items {
                return Err(PdfError::limit("field /Kids exceeds container limit"));
            }
            for kid in kids {
                walk_field(parsed, kid, &context, depth + 1, path, walked, output)?;
            }
        }
        Ok(())
    })();

    if let Some(reference) = reference {
        path.remove(&reference);
    }
    result
}

fn widget_refs(
    parsed: &ParsedDocument,
    dict: &BTreeMap<Vec<u8>, Value>,
    reference: Option<ObjectRef>,
) -> Result<Vec<FormWidgetRef>, PdfError> {
    let mut widgets = Vec::new();
    if dict
        .get(b"Subtype".as_slice())
        .is_some_and(|value| matches!(value, Value::Name(name) if name.as_slice() == b"Widget"))
        && let Some(reference) = reference
    {
        widgets.push(FormWidgetRef {
            object_number: reference.number,
            object_generation: reference.generation,
        });
    }
    let Some(kids) = dict.get(b"Kids".as_slice()) else {
        return Ok(widgets);
    };
    let kids = resolve_array(parsed, kids, reference, "field /Kids")?;
    for kid in kids {
        let (kid, kid_ref) = resolve_dict(parsed, kid, "field kid")?;
        if kid
            .get(b"Subtype".as_slice())
            .is_some_and(|value| matches!(value, Value::Name(name) if name.as_slice() == b"Widget"))
            && let Some(kid_ref) = kid_ref
        {
            widgets.push(FormWidgetRef {
                object_number: kid_ref.number,
                object_generation: kid_ref.generation,
            });
        }
    }
    Ok(widgets)
}

fn field_value(
    value: Option<&Value>,
    reference: Option<ObjectRef>,
    label: &str,
) -> Result<Option<String>, PdfError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(decode_text(value))),
        Some(Value::Name(value)) => Ok(Some(String::from_utf8_lossy(value).into_owned())),
        // Arrays and dictionaries are valid for some field types, but this inspection slice only
        // promises scalar string/name values.
        Some(Value::Array(_) | Value::Dict(_) | Value::Ref(_)) => Ok(None),
        Some(_) => Err(malformed(
            format!("{label} is not a string or name"),
            reference,
        )),
    }
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

fn name(value: &Value, reference: Option<ObjectRef>, label: &str) -> Result<String, PdfError> {
    match value {
        Value::Name(value) => Ok(String::from_utf8_lossy(value).into_owned()),
        _ => Err(malformed(format!("{label} is not a name"), reference)),
    }
}

fn text_string(
    value: &Value,
    reference: Option<ObjectRef>,
    label: &str,
) -> Result<String, PdfError> {
    match value {
        Value::String(value) => Ok(decode_text(value)),
        _ => Err(malformed(format!("{label} is not a string"), reference)),
    }
}

fn nonnegative_u32(
    value: &Value,
    reference: Option<ObjectRef>,
    label: &str,
) -> Result<u32, PdfError> {
    match value {
        Value::Integer(value) => u32::try_from(*value)
            .map_err(|_| malformed(format!("{label} is outside u32"), reference)),
        _ => Err(malformed(format!("{label} is not an integer"), reference)),
    }
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

fn malformed(message: impl Into<String>, reference: Option<ObjectRef>) -> PdfError {
    PdfError {
        code: PdfErrorCode::InvalidSyntax,
        message: message.into(),
        span: None,
        object: reference.map(|value| (value.number, value.generation)),
    }
}
