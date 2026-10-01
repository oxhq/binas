//! Retained fixed-template text-field filling. Value changes reuse explicit
//! AP-local encodings and conservative supplied metrics; glyph programs,
//! shaping, font substitution and automatic appearance layout are not inferred.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SnapshotId, Span,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};
use serde::{Deserialize, Serialize};

use super::flatten::{
    self, Registry, dict, present, public_ref, reserve, reserve_value, resolve, value_cost,
};
use crate::{
    PdfDocument, PdfError, PdfReferenceIndex, PdfReferenceTarget,
    annotations::encode_text,
    content::operations::{ContentOperand, OperationState, read_operations_part},
    edit::encode_hex,
    filters::encode_pdf_stream,
    fonts::{
        SimpleFontTextMetrics, decode_existing_cmap_resource_text,
        encode_existing_cmap_resource_text, simple_font_text_metrics,
    },
    object::{IndirectObject, ObjectRef, Value},
    parser::{ParseBudget, decode_stream},
    payload::StreamPayload,
    store::ObjectStore,
};

type Dict = BTreeMap<Vec<u8>, Value>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PdfTextFieldFillRequest {
    pub field_name: String,
    pub value: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfTextWidgetFillEffect {
    pub widget: PdfReferenceTarget,
    pub page: PdfReferenceTarget,
    pub source_appearance: PdfReferenceTarget,
    pub output_appearance: PdfReferenceTarget,
    pub font: Option<PdfReferenceTarget>,
    pub cmap: PdfReferenceTarget,
    pub font_size: f64,
    pub original_advance: f64,
    pub replacement_advance: f64,
    /// Conservative replacement ink bounds in appearance-local coordinates.
    pub ink_bounds: [f64; 4],
    pub clip_bounds: [f64; 4],
    pub available_width: f64,
    pub encoded_bytes: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfTextFieldFillEffect {
    pub field: PdfReferenceTarget,
    pub before_value: String,
    pub after_value: String,
    pub widgets: Vec<PdfTextWidgetFillEffect>,
}

#[derive(Debug)]
pub(crate) struct TextFillPlan {
    snapshot: SnapshotId,
    store: ObjectStore,
    requests: Vec<PdfTextFieldFillRequest>,
    effects: Vec<PdfTextFieldFillEffect>,
    widgets: BTreeMap<ObjectRef, WidgetChange>,
    replacements: BTreeMap<ObjectRef, IndirectObject>,
    authorized: BTreeSet<ObjectRef>,
    _reservations: Vec<Reservation>,
}

#[derive(Debug)]
struct WidgetChange {
    appearance: ObjectRef,
    dictionary: Option<ObjectRef>,
}

impl TextFillPlan {
    pub(crate) fn effects(&self) -> &[PdfTextFieldFillEffect] {
        &self.effects
    }
    pub(crate) fn changed_refs(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.authorized.iter().copied()
    }
}

#[derive(Debug)]
struct SelectedField {
    field: ObjectRef,
    value: String,
    widgets: Vec<ObjectRef>,
}

struct Template {
    span: Span,
    raw: Vec<u8>,
    font_name: Vec<u8>,
    font_size: f64,
    origin: [f64; 2],
    clip: [f64; 4],
}

struct PreparedAppearance {
    source: ObjectRef,
    dictionary: Option<ObjectRef>,
    ap: Dict,
    decoded: Vec<u8>,
    effect: PdfTextWidgetFillEffect,
}

pub(crate) fn plan(
    source: &PdfDocument,
    snapshot: SnapshotId,
    index: &PdfReferenceIndex,
    requests: &[PdfTextFieldFillRequest],
    staged: &BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<TextFillPlan, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if snapshot.source() != source.source_id() || index.snapshot() != snapshot {
        return Err(PdfError::selection(
            "text field plan source/revision mismatch",
        ));
    }
    if requests.is_empty() || requests.len() > source.parsed().limits.max_container_items {
        return Err(PdfError::limit(
            "text field request count is empty or exceeds limit",
        ));
    }
    crate::writer::refuse_security_boundaries_with_context(source.parsed(), Some(context))?;
    crate::preservation::pages::validate_tree(source, context)?;
    let fresh = PdfReferenceIndex::build(source, context)?;
    context
        .charge_work(fresh.records().len() as u64)
        .map_err(PdfError::from_context)?;
    if fresh.records() != index.records() {
        return Err(PdfError::selection(
            "text field index describes a different object-store branch",
        ));
    }
    let registry = flatten::registry(source, context)?;
    let mut reservations = Vec::new();
    reserve(
        &mut reservations,
        registry
            .nodes
            .len()
            .checked_mul(std::mem::size_of::<PdfTextWidgetFillEffect>() * 2)
            .ok_or_else(|| PdfError::limit("text field selection workspace overflows"))?,
        context,
    )?;
    let mut effects = Vec::new();
    let mut widgets = BTreeMap::new();
    let mut replacements = BTreeMap::new();
    let mut authorized = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut next = flatten::next_number(source, staged)?;
    let mut added = 0usize;
    for request in requests {
        let selected = select(source, &registry, request, context)?;
        if !seen.insert(selected.field) {
            return Err(PdfError::unsafe_rewrite("text field requests overlap"));
        }
        for reference in std::iter::once(&selected.field).chain(selected.widgets.iter()) {
            flatten::check_incoming(&registry, &fresh, *reference, context)?;
            if staged.contains_key(reference) {
                return Err(PdfError::unsafe_rewrite(
                    "text field fill conflicts with another field/widget mutation",
                ));
            }
        }
        let mut field = source.parsed().object(selected.field)?.clone();
        reserve_value(&field.value, source, &mut reservations, context)?;
        let Value::Dict(ref mut field_values) = field.value else {
            unreachable!("selected field dictionary")
        };
        field_values.insert(b"V".to_vec(), Value::String(encode_text(&request.value)));
        replacements.insert(selected.field, field);
        authorized.insert(selected.field);
        let mut widget_effects = Vec::new();
        for widget in selected.widgets {
            let mut prepared = prepare(
                source,
                &registry,
                selected.field,
                widget,
                &selected.value,
                &request.value,
                context,
            )?;
            added = added
                .checked_add(1)
                .ok_or_else(|| PdfError::limit("text appearance allocation count overflows"))?;
            let output = flatten::allocate(&mut next, source, staged, added)?;
            prepared.effect.output_appearance = public_ref(output);
            let original = source.parsed().object(prepared.source)?;
            let mut replacement = original.clone();
            replacement.offset = 0;
            replacement.stream_offset = 0;
            let encoded =
                encode_pdf_stream(&original.value, &prepared.decoded, &source.parsed().limits)?;
            context
                .charge_work(encoded.len() as u64 + prepared.decoded.len() as u64)
                .map_err(PdfError::from_context)?;
            let reservation = context
                .reserve(encoded.len() as u64)
                .map_err(PdfError::from_context)?;
            replacement.stream = Some(StreamPayload::from_reserved(encoded, vec![reservation])?);
            let Value::Dict(ref mut values) = replacement.value else {
                unreachable!("appearance dictionary")
            };
            values.insert(
                b"Length".to_vec(),
                Value::Integer(
                    replacement
                        .stream
                        .as_ref()
                        .expect("encoded appearance")
                        .len() as i64,
                ),
            );
            if matches!(present(values,b"DL"),Some(Value::Integer(value)) if *value>=0) {
                values.insert(
                    b"DL".to_vec(),
                    Value::Integer(prepared.decoded.len() as i64),
                );
            }
            reserve_value(&replacement.value, source, &mut reservations, context)?;
            replacements.insert(output, replacement);
            authorized.insert(output);
            prepared.ap.insert(b"N".to_vec(), Value::Ref(output));
            let dictionary = if prepared.dictionary.is_some() {
                added = added
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("text AP allocation count overflows"))?;
                let output = flatten::allocate(&mut next, source, staged, added)?;
                let value = Value::Dict(prepared.ap.clone());
                reserve_value(&value, source, &mut reservations, context)?;
                replacements.insert(
                    output,
                    IndirectObject {
                        value,
                        stream: None,
                        offset: 0,
                        stream_offset: 0,
                    },
                );
                authorized.insert(output);
                Some(output)
            } else {
                None
            };
            let widget_object = if let Some(object) = replacements.get_mut(&widget) {
                object
            } else {
                let object = source.parsed().object(widget)?.clone();
                reserve_value(&object.value, source, &mut reservations, context)?;
                replacements.entry(widget).or_insert(object)
            };
            let Value::Dict(ref mut values) = widget_object.value else {
                unreachable!("selected widget dictionary")
            };
            values.insert(
                b"AP".to_vec(),
                dictionary.map_or(Value::Dict(prepared.ap), Value::Ref),
            );
            authorized.insert(widget);
            widgets.insert(
                widget,
                WidgetChange {
                    appearance: output,
                    dictionary,
                },
            );
            widget_effects.push(prepared.effect);
        }
        reserve(
            &mut reservations,
            request.field_name.len()
                + request.value.len() * 4
                + selected.value.len()
                + std::mem::size_of::<PdfTextFieldFillRequest>()
                + std::mem::size_of::<PdfTextFieldFillEffect>(),
            context,
        )?;
        effects.push(PdfTextFieldFillEffect {
            field: public_ref(selected.field),
            before_value: selected.value,
            after_value: request.value.clone(),
            widgets: widget_effects,
        });
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(TextFillPlan {
        snapshot,
        store: source.parsed().objects.clone(),
        requests: requests.to_vec(),
        effects,
        widgets,
        replacements,
        authorized,
        _reservations: reservations,
    })
}

pub(crate) fn apply(
    source: &PdfDocument,
    plan: &TextFillPlan,
    changes: &mut BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    bound(source, plan)?;
    for reference in plan.replacements.keys() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if changes.contains_key(reference) {
            return Err(PdfError::unsafe_rewrite(
                "text field fill overlaps staged objects",
            ));
        }
    }
    for (reference, object) in &plan.replacements {
        context
            .charge_work(value_cost(&object.value, source, 0, context)? as u64)
            .map_err(PdfError::from_context)?;
        changes.insert(*reference, object.clone());
    }
    Ok(())
}

const CHECKS: [&str; 4] = [
    "requested_values",
    "appearance_text_and_fit",
    "source_preservation",
    "authorized_changes",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.text_field", name).expect("static text field code")
}
pub(crate) fn required_checks() -> RequiredChecks {
    RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
        .expect("static text field requirements")
}

pub(crate) fn verify(
    source: &PdfDocument,
    candidate: &PdfDocument,
    plan: &TextFillPlan,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    bound(source, plan)?;
    if candidate.source_id() != source.source_id() {
        return Err(PdfError::selection("text field candidate source mismatch"));
    }
    crate::preservation::pages::validate_tree(candidate, context)?;
    let registry = flatten::registry(source, context)?;
    let mut expected = Vec::new();
    let allocation_floor = flatten::next_number(source, &BTreeMap::new())?;
    let mut authorized = BTreeSet::new();
    let mut new_objects = BTreeSet::new();
    let mut values_ok = true;
    let mut appearances_ok = true;
    let mut preserved = true;
    let mut expected_fields = BTreeMap::new();
    let mut expected_widgets = BTreeMap::new();
    let mut protected: BTreeSet<ObjectRef> = registry
        .nodes
        .keys()
        .chain(registry.containers.iter())
        .copied()
        .collect();
    protected.insert(registry.acro_owner);
    for request in &plan.requests {
        let selected = select(source, &registry, request, context)?;
        if expected_fields
            .insert(selected.field, encode_text(&request.value))
            .is_some()
        {
            return Err(PdfError::verification(
                "text field evidence requests overlap",
            ));
        }
        authorized.insert(selected.field);
        let mut effects = Vec::new();
        for widget in selected.widgets {
            let prepared = prepare(
                source,
                &registry,
                selected.field,
                widget,
                &selected.value,
                &request.value,
                context,
            )?;
            flatten::collect_closure(source, prepared.source, &mut protected, context)?;
            if let Some(dictionary) = prepared.dictionary {
                flatten::collect_closure(source, dictionary, &mut protected, context)?;
            }
            let Some(change) = plan.widgets.get(&widget) else {
                appearances_ok = false;
                continue;
            };
            for reference in std::iter::once(change.appearance).chain(change.dictionary) {
                appearances_ok &= reference.number >= allocation_floor
                    && reference.generation == 0
                    && !source.parsed().objects.contains_key(&reference)
                    && new_objects.insert(reference);
                authorized.insert(reference);
            }
            appearances_ok &= change.dictionary.is_some() == prepared.dictionary.is_some();
            let after = candidate.parsed().object(change.appearance)?;
            appearances_ok &= appearance_matches(
                source,
                candidate,
                prepared.source,
                after,
                &prepared.decoded,
                context,
            )?;
            let mut ap = prepared.ap;
            ap.insert(b"N".to_vec(), Value::Ref(change.appearance));
            let ap = if let Some(dictionary) = change.dictionary {
                let object = candidate.parsed().object(dictionary)?;
                appearances_ok &= object.value == Value::Dict(ap) && object.stream.is_none();
                Value::Ref(dictionary)
            } else {
                Value::Dict(ap)
            };
            expected_widgets.insert(widget, ap);
            authorized.insert(widget);
            let mut effect = prepared.effect;
            effect.output_appearance = public_ref(change.appearance);
            effects.push(effect);
        }
        expected.push(PdfTextFieldFillEffect {
            field: public_ref(selected.field),
            before_value: selected.value,
            after_value: request.value.clone(),
            widgets: effects,
        });
    }
    for (reference, before) in &source.parsed().objects {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let field = expected_fields.get(reference);
        let widget = expected_widgets.get(reference);
        if field.is_none() && widget.is_none() {
            continue;
        }
        let after = candidate.parsed().object(*reference)?;
        let original = dict(&before.value)?;
        let actual = dict(&after.value)?;
        preserved &= before.stream.as_deref() == after.stream.as_deref();
        for (key, value) in original {
            context
                .charge_work(value_cost(value, source, 0, context)? as u64)
                .map_err(PdfError::from_context)?;
            let expected = match key.as_slice() {
                b"V" if field.is_some() => Value::String(field.expect("field").clone()),
                b"AP" if widget.is_some() => widget.expect("widget").clone(),
                _ => value.clone(),
            };
            preserved &= actual.get(key) == Some(&expected);
        }
        for (key, value) in actual {
            match key.as_slice() {
                b"V" if field.is_some() => {
                    values_ok &= value == &Value::String(field.expect("field").clone())
                }
                b"AP" if widget.is_some() => appearances_ok &= Some(value) == widget,
                _ => preserved &= original.get(key) == Some(value),
            }
        }
        if let Some(value) = field {
            values_ok &= actual.get(b"V".as_slice()) == Some(&Value::String(value.clone()));
        }
        if let Some(value) = widget {
            appearances_ok &= actual.get(b"AP".as_slice()) == Some(value);
        }
    }
    for reference in protected {
        if expected_fields.contains_key(&reference) || expected_widgets.contains_key(&reference) {
            continue;
        }
        let before = source.parsed().object(reference)?;
        let after = candidate.parsed().object(reference)?;
        if reference == registry.catalog {
            preserved &= dict(&before.value)?.get(b"AcroForm".as_slice())
                == dict(&after.value)?.get(b"AcroForm".as_slice());
        } else {
            preserved &= flatten::objects_equal(before, after, source, context)?;
        }
    }
    preserved &= dict(&source.parsed().object(registry.catalog)?.value)?
        .get(b"AcroForm".as_slice())
        == dict(&candidate.parsed().object(registry.catalog)?.value)?.get(b"AcroForm".as_slice());
    appearances_ok &= expected == plan.effects;
    let authorized_ok = authorized == plan.authorized
        && plan
            .replacements
            .keys()
            .all(|reference| authorized.contains(reference))
        && expected_widgets.len() == plan.widgets.len();
    Ok(EvidenceReport::new(CHECKS.into_iter().zip([values_ok,appearances_ok,preserved,authorized_ok]).map(|(name,passed)|CheckEvidence{code:code(name),status:if passed{CheckStatus::Passed}else{CheckStatus::Failed},detail:"Requested text value and one existing AP-local Tj token with exact explicit mapping and conservative supplied-font metric fit; original styles, defaults, form registration and source AP closures preserved. No glyph-program, shaping, font substitution or standards assurance.".into()}).collect()))
}

fn select(
    source: &PdfDocument,
    registry: &Registry,
    request: &PdfTextFieldFillRequest,
    context: &OperationContext,
) -> Result<SelectedField, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if request.match_index != 0 || request.field_name.is_empty() {
        return Err(PdfError::selection(
            "text fill requires one unique fully qualified field name",
        ));
    }
    if request.field_name.len() > source.parsed().limits.max_token_bytes
        || request.value.len() > source.parsed().limits.max_token_bytes
    {
        return Err(PdfError::limit("text field name/value exceeds token limit"));
    }
    let scan_work = request
        .value
        .len()
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(request.field_name.len()))
        .ok_or_else(|| PdfError::limit("text field request scan work overflows"))?;
    context
        .charge_work(scan_work as u64)
        .map_err(PdfError::from_context)?;
    if request.value.chars().any(char::is_control) {
        return Err(PdfError::unsupported(
            "fixed-template text fields do not support control characters",
        ));
    }
    let field = *registry
        .names
        .get(&request.field_name)
        .ok_or_else(|| PdfError::selection("text field name is not registered"))?;
    if !matches!(flatten::inherited(source,registry,field,b"FT",context)?,Some(Value::Name(kind)) if kind==b"Tx")
    {
        return Err(PdfError::unsupported(
            "retained text fill requires a terminal Tx field",
        ));
    }
    let flags = flatten::inherited(source, registry, field, b"Ff", context)?;
    if !matches!(flags, None | Some(Value::Integer(0 | 2 | 4 | 6))) {
        return Err(PdfError::unsupported(
            "text field flags require unsupported read-only, multiline, password, rich-text or layout behavior",
        ));
    }
    if !matches!(
        flatten::inherited(source, registry, field, b"Q", context)?,
        None | Some(Value::Integer(0))
    ) {
        return Err(PdfError::unsupported(
            "retained text fill requires left alignment",
        ));
    }
    if let Some(value) = flatten::inherited(source, registry, field, b"MaxLen", context)? {
        let Value::Integer(maximum) = value else {
            return Err(PdfError::unsupported(
                "text field MaxLen must be an integer",
            ));
        };
        if *maximum < 0 || request.value.chars().count() as u64 > *maximum as u64 {
            return Err(PdfError::selection("text field replacement exceeds MaxLen"));
        }
    }
    let value = match flatten::inherited(source, registry, field, b"V", context)? {
        None => String::new(),
        Some(value) => text_string(source, value, context)?,
    };
    let node = &registry.nodes[&field];
    let widgets = if node.widget {
        vec![field]
    } else {
        if node.children.is_empty() {
            return Err(PdfError::unsupported(
                "text fill requires at least one current widget appearance",
            ));
        }
        for child in &node.children {
            let child_node = &registry.nodes[child];
            let values = dict(&source.parsed().object(*child)?.value)?;
            if !child_node.widget
                || !child_node.children.is_empty()
                || [b"T".as_slice(), b"FT", b"Ff", b"V", b"DV"]
                    .iter()
                    .any(|key| present(values, key).is_some())
            {
                return Err(PdfError::unsupported(
                    "text fill selects a terminal field with separate plain widgets",
                ));
            }
        }
        node.children.clone()
    };
    let mut parent = Some(field);
    while let Some(reference) = parent {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let values = dict(&source.parsed().object(reference)?.value)?;
        flatten::selected_policy(values)?;
        if [b"RV".as_slice(), b"DS"]
            .iter()
            .any(|key| present(values, key).is_some())
        {
            return Err(PdfError::unsupported(
                "text fill does not rewrite rich text or default styles",
            ));
        }
        parent = registry.nodes[&reference].parent;
    }
    for widget in &widgets {
        let values = dict(&source.parsed().object(*widget)?.value)?;
        flatten::selected_policy(values)?;
        if let Some(value) = present(values, b"F") {
            let Value::Integer(flags) = resolve(source, value, context)? else {
                return Err(PdfError::unsupported("text widget flags must be integers"));
            };
            if *flags < 0 || *flags > u32::MAX as i64 || flags & (64 | 128 | 512) != 0 {
                return Err(PdfError::unsupported("text widget is read-only or locked"));
            }
        }
        let pages = registry.pages.get(widget).ok_or_else(|| {
            PdfError::unsafe_rewrite("text widget has no active page registration")
        })?;
        if pages.len() != 1 || present(values, b"P") != Some(&Value::Ref(pages[0])) {
            return Err(PdfError::unsafe_rewrite(
                "text widget ownership is ambiguous",
            ));
        }
    }
    Ok(SelectedField {
        field,
        value,
        widgets,
    })
}

fn prepare(
    source: &PdfDocument,
    registry: &Registry,
    _field: ObjectRef,
    widget: ObjectRef,
    before_value: &str,
    replacement: &str,
    context: &OperationContext,
) -> Result<PreparedAppearance, PdfError> {
    let widget_values = dict(&source.parsed().object(widget)?.value)?;
    let raw_ap = present(widget_values, b"AP")
        .ok_or_else(|| PdfError::unsupported("text widget has no current appearance"))?;
    let (dictionary, ap) = match raw_ap {
        Value::Ref(reference) => {
            let object = source.parsed().object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "text widget AP dictionary cannot be a stream",
                ));
            }
            (Some(*reference), dict(&object.value)?)
        }
        Value::Dict(values) => (None, values),
        _ => {
            return Err(PdfError::unsupported(
                "text widget AP must be a direct or singly indirect dictionary",
            ));
        }
    };
    if present(ap, b"R").is_some() || present(ap, b"D").is_some() {
        return Err(PdfError::unsupported(
            "text field fill does not update rollover or down appearances",
        ));
    }
    let Some(Value::Ref(appearance)) = present(ap, b"N") else {
        return Err(PdfError::unsupported(
            "text field requires one indirect normal Form appearance",
        ));
    };
    let object = source.parsed().object(*appearance)?;
    let values = dict(&object.value)?;
    if object.stream.is_none()
        || !matches!(present(values,b"Subtype"),Some(Value::Name(kind)) if kind==b"Form")
    {
        return Err(PdfError::unsupported(
            "text field normal appearance must be a Form stream",
        ));
    }
    if let Some(value) = present(values, b"DL")
        && !matches!(value,Value::Integer(value) if *value>=0)
    {
        return Err(PdfError::unsupported(
            "text field appearance DL must be an optional nonnegative direct integer",
        ));
    }
    flatten::validate_opaque_appearance(source, *appearance, context)?;
    let bbox = flatten::rectangle(resolve(
        source,
        present(values, b"BBox")
            .ok_or_else(|| PdfError::unsupported("text appearance has no BBox"))?,
        context,
    )?)?;
    let matrix = flatten::appearance_matrix(source, values, context)?;
    let rect = flatten::rectangle(resolve(
        source,
        present(widget_values, b"Rect")
            .ok_or_else(|| PdfError::unsupported("text widget has no Rect"))?,
        context,
    )?)?;
    flatten::placement(rect, bbox, matrix)?;
    let resources = present(values, b"Resources")
        .ok_or_else(|| PdfError::unsupported("text appearance needs explicit font resources"))?;
    let document = source.with_cancellation(context.cancellation().clone());
    let mut budget = ParseBudget::for_document(document.parsed());
    let decoded = decode_stream(
        &object.value,
        object.stream.as_deref().expect("appearance stream"),
        &source.parsed().limits,
        &mut budget,
    )?;
    let template = template(source, &decoded, bbox, &mut budget, context)?;
    let current = decode_existing_cmap_resource_text(
        source,
        resources,
        &template.font_name,
        &template.raw,
        context,
    )?;
    if current != before_value {
        return Err(PdfError::unsafe_rewrite(
            "text field current appearance does not exactly represent effective V",
        ));
    }
    let encoded = encode_existing_cmap_resource_text(
        source,
        resources,
        &template.font_name,
        replacement,
        context,
    )?;
    let original_metrics = simple_font_text_metrics(
        source,
        resources,
        &template.font_name,
        &template.raw,
        context,
    )?;
    let replacement_metrics = simple_font_text_metrics(
        source,
        resources,
        &template.font_name,
        &encoded.bytes,
        context,
    )?;
    fit(&original_metrics, &template, template.raw.is_empty())?;
    let ink_bounds = fit(&replacement_metrics, &template, encoded.bytes.is_empty())?;
    let token = encode_hex(&encoded.bytes);
    let range = usize::try_from(template.span.start())
        .map_err(|_| PdfError::limit("text appearance token start exceeds address space"))?
        ..usize::try_from(template.span.end())
            .map_err(|_| PdfError::limit("text appearance token end exceeds address space"))?;
    if range.end > decoded.len() {
        return Err(PdfError::verification(
            "text appearance token lies outside decoded bytes",
        ));
    }
    let length = decoded
        .len()
        .checked_sub(range.len())
        .and_then(|len| len.checked_add(token.len()))
        .ok_or_else(|| PdfError::limit("text appearance replacement length overflows"))?;
    if length > source.parsed().limits.max_stream_bytes {
        return Err(PdfError::limit(
            "text appearance replacement exceeds stream limit",
        ));
    }
    let _scratch = context
        .reserve(length as u64)
        .map_err(PdfError::from_context)?;
    let mut output = Vec::with_capacity(length);
    output.extend_from_slice(&decoded[..range.start]);
    output.extend_from_slice(&token);
    output.extend_from_slice(&decoded[range.end..]);
    let scale = template.font_size / 1000.;
    let effect = PdfTextWidgetFillEffect {
        widget: public_ref(widget),
        page: public_ref(registry.pages[&widget][0]),
        source_appearance: public_ref(*appearance),
        output_appearance: public_ref(*appearance),
        font: encoded.font.map(public_ref),
        cmap: public_ref(encoded.cmap),
        font_size: template.font_size,
        original_advance: original_metrics.advance * scale,
        replacement_advance: replacement_metrics.advance * scale,
        ink_bounds,
        clip_bounds: template.clip,
        available_width: template.clip[2] - template.origin[0],
        encoded_bytes: encoded.bytes.len(),
    };
    Ok(PreparedAppearance {
        source: *appearance,
        dictionary,
        ap: ap.clone(),
        decoded: output,
        effect,
    })
}

fn fit(
    metrics: &SimpleFontTextMetrics,
    template: &Template,
    empty: bool,
) -> Result<[f64; 4], PdfError> {
    if empty {
        return Ok([
            template.origin[0],
            template.origin[1],
            template.origin[0],
            template.origin[1],
        ]);
    }
    let scale = template.font_size / 1000.;
    let bbox = metrics.bbox;
    let bounds = [
        template.origin[0] + bbox[0] * scale,
        template.origin[1] + bbox[1] * scale,
        template.origin[0] + bbox[2] * scale,
        template.origin[1] + bbox[3] * scale,
    ];
    if bounds.iter().any(|value| !value.is_finite())
        || bounds[0] < template.clip[0]
        || bounds[1] < template.clip[1]
        || bounds[2] > template.clip[2]
        || bounds[3] > template.clip[3]
    {
        return Err(PdfError::unsafe_rewrite(
            "text field glyph metric bounds do not fit the existing appearance clip",
        ));
    }
    Ok(bounds)
}

fn template(
    source: &PdfDocument,
    bytes: &[u8],
    bbox: [f64; 4],
    budget: &mut ParseBudget,
    context: &OperationContext,
) -> Result<Template, PdfError> {
    let mut parser = OperationState::new();
    let operations = read_operations_part(bytes, 0, &mut parser, &source.parsed().limits, budget)?;
    parser.finish()?;
    let mut clip = bbox;
    let mut saved = Vec::new();
    let mut path: Option<[f64; 4]> = None;
    let mut unknown_path = false;
    let mut pending_clip = false;
    let mut in_text = false;
    let mut seen_text = false;
    let mut font = None;
    let mut origin = None;
    let mut token = None;
    let mut marked = 0usize;
    for operation in operations {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let op = operation.operator.as_slice();
        let args = operation.operands.as_slice();
        if pending_clip
            && !matches!(
                op,
                b"n" | b"f" | b"F" | b"f*" | b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*"
            )
        {
            return Err(PdfError::unsupported(
                "text appearance clipping operator must immediately terminate its proven rectangular path",
            ));
        }
        match op {
            b"BMC" if !in_text => {
                if !matches!(args, [ContentOperand::Name(name)] if name == b"Tx") {
                    return Err(PdfError::unsupported(
                        "text appearance permits only the Tx marked-content wrapper",
                    ));
                }
                marked = marked
                    .checked_add(1)
                    .ok_or_else(|| PdfError::limit("text appearance marker depth overflows"))?;
            }
            b"EMC" if !in_text => {
                if !args.is_empty() {
                    return Err(PdfError::unsupported("text appearance EMC has operands"));
                }
                marked = marked
                    .checked_sub(1)
                    .ok_or_else(|| PdfError::unsafe_rewrite("text appearance marker underflows"))?;
            }
            b"q" if !in_text => {
                if !args.is_empty() {
                    return Err(PdfError::unsupported("text appearance q has operands"));
                }
                saved.push(clip);
            }
            b"Q" if !in_text => {
                if !args.is_empty() {
                    return Err(PdfError::unsupported("text appearance Q has operands"));
                }
                clip = saved.pop().ok_or_else(|| {
                    PdfError::unsafe_rewrite("text appearance graphics stack underflow")
                })?;
            }
            b"BT" => {
                if seen_text || in_text || !args.is_empty() {
                    return Err(PdfError::unsupported(
                        "text appearance requires one simple text object",
                    ));
                }
                seen_text = true;
                in_text = true;
            }
            b"ET" => {
                if !in_text || !args.is_empty() {
                    return Err(PdfError::unsupported("text appearance ET is malformed"));
                }
                in_text = false;
            }
            b"Tf" if in_text && token.is_none() => {
                let [ContentOperand::Name(name), ContentOperand::Number(size)] = args else {
                    return Err(PdfError::unsupported(
                        "text appearance Tf needs font name and size",
                    ));
                };
                if font.is_some() || *size <= 0. || !size.is_finite() {
                    return Err(PdfError::unsupported(
                        "text appearance needs one positive fixed font size",
                    ));
                }
                font = Some((name.clone(), *size));
            }
            b"Tm" if in_text && token.is_none() => {
                let values = operand_numbers::<6>(args)?;
                if origin.is_some() || values[..4] != [1., 0., 0., 1.] {
                    return Err(PdfError::unsupported(
                        "text appearance needs one horizontal identity Tm",
                    ));
                }
                origin = Some([values[4], values[5]]);
            }
            b"Td" if in_text && token.is_none() => {
                let values = operand_numbers::<2>(args)?;
                if origin.is_some() {
                    return Err(PdfError::unsupported(
                        "text appearance permits one initial Td",
                    ));
                }
                origin = Some(values);
            }
            b"Tj" if in_text => {
                let [
                    ContentOperand::Bytes {
                        bytes,
                        part_index: 0,
                        decoded_span,
                    },
                ] = args
                else {
                    return Err(PdfError::unsupported(
                        "text appearance needs one complete Tj string",
                    ));
                };
                if token.is_some() {
                    return Err(PdfError::unsupported(
                        "text appearance has multiple text-show tokens",
                    ));
                }
                let (font_name, font_size) = font.clone().ok_or_else(|| {
                    PdfError::unsupported("text appearance has no explicit in-text font")
                })?;
                token = Some(Template {
                    span: *decoded_span,
                    raw: bytes.clone(),
                    font_name,
                    font_size,
                    origin: origin.ok_or_else(|| {
                        PdfError::unsupported("text appearance has no fixed text origin")
                    })?,
                    clip,
                });
            }
            b"Tc" | b"Tw" | b"Ts" | b"Tr" if in_text && token.is_none() => {
                if operand_numbers::<1>(args)? != [0.] {
                    return Err(PdfError::unsupported(
                        "text appearance needs default text spacing, rise and fill rendering",
                    ));
                }
            }
            b"Tz" if in_text && token.is_none() => {
                if operand_numbers::<1>(args)? != [100.] {
                    return Err(PdfError::unsupported(
                        "text appearance needs default horizontal scaling",
                    ));
                }
            }
            b"re" if !in_text => {
                let [x, y, width, height] = operand_numbers::<4>(args)?;
                if width <= 0.
                    || height <= 0.
                    || !(x + width).is_finite()
                    || !(y + height).is_finite()
                {
                    return Err(PdfError::unsupported(
                        "text appearance rectangle is nonpositive or nonfinite",
                    ));
                }
                if path.is_some() {
                    unknown_path = true;
                }
                path = Some([x, y, x + width, y + height]);
            }
            b"W" | b"W*" if !in_text => {
                if !args.is_empty() || pending_clip || unknown_path || path.is_none() {
                    return Err(PdfError::unsupported(
                        "text appearance clip requires one explicit rectangle",
                    ));
                }
                pending_clip = true;
            }
            b"n" | b"f" | b"F" | b"f*" | b"S" | b"s" | b"B" | b"B*" | b"b" | b"b*" if !in_text => {
                if !args.is_empty() {
                    return Err(PdfError::unsupported(
                        "text appearance path terminator has operands",
                    ));
                }
                if pending_clip {
                    let rect = path.ok_or_else(|| {
                        PdfError::unsupported("text appearance clipping path disappeared")
                    })?;
                    clip = [
                        clip[0].max(rect[0]),
                        clip[1].max(rect[1]),
                        clip[2].min(rect[2]),
                        clip[3].min(rect[3]),
                    ];
                    if clip[2] <= clip[0] || clip[3] <= clip[1] {
                        return Err(PdfError::unsafe_rewrite("text appearance clip is empty"));
                    }
                }
                path = None;
                unknown_path = false;
                pending_clip = false;
            }
            b"m" | b"l" | b"c" | b"v" | b"y" | b"h" if !in_text => {
                unknown_path = true;
            }
            b"g" | b"G" | b"rg" | b"RG" | b"k" | b"K" | b"w" | b"J" | b"j" | b"M" | b"d"
            | b"ri" | b"i" => {}
            _ => {
                return Err(PdfError::unsupported(
                    "text appearance is outside the supported fixed horizontal single-Tj template",
                ));
            }
        }
        if saved.len() > source.parsed().limits.max_parser_depth
            || marked > source.parsed().limits.max_parser_depth
        {
            return Err(PdfError::limit(
                "text appearance graphics depth exceeds limit",
            ));
        }
    }
    if in_text || !saved.is_empty() || pending_clip || marked != 0 {
        return Err(PdfError::unsafe_rewrite(
            "text appearance template has unbalanced state",
        ));
    }
    token.ok_or_else(|| PdfError::unsupported("text appearance has no unique text-show token"))
}

fn operand_numbers<const N: usize>(values: &[ContentOperand]) -> Result<[f64; N], PdfError> {
    if values.len() != N {
        return Err(PdfError::unsupported(
            "text appearance numeric operand count is wrong",
        ));
    }
    let mut result = [0.; N];
    for (output, value) in result.iter_mut().zip(values) {
        let ContentOperand::Number(value) = value else {
            return Err(PdfError::unsupported(
                "text appearance operand is not numeric",
            ));
        };
        if !value.is_finite() {
            return Err(PdfError::unsupported("text appearance number is nonfinite"));
        }
        *output = *value;
    }
    Ok(result)
}

fn appearance_matches(
    source: &PdfDocument,
    candidate: &PdfDocument,
    reference: ObjectRef,
    after: &IndirectObject,
    expected: &[u8],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    let before = source.parsed().object(reference)?;
    let original = dict(&before.value)?;
    let actual = dict(&after.value)?;
    let Some(encoded) = after.stream.as_deref() else {
        return Ok(false);
    };
    let mut passed = true;
    for (key, value) in original {
        context
            .charge_work(value_cost(value, source, 0, context)? as u64)
            .map_err(PdfError::from_context)?;
        match key.as_slice() {
            b"Length" => passed &= actual.get(key) == Some(&Value::Integer(encoded.len() as i64)),
            b"DL" if matches!(value,Value::Integer(value) if *value>=0) => {
                passed &= actual.get(key) == Some(&Value::Integer(expected.len() as i64))
            }
            _ => passed &= actual.get(key) == Some(value),
        }
    }
    passed &= actual.keys().all(|key| original.contains_key(key));
    let document = candidate.with_cancellation(context.cancellation().clone());
    let mut budget = ParseBudget::for_document(document.parsed());
    let decoded = decode_stream(
        &after.value,
        encoded,
        &candidate.parsed().limits,
        &mut budget,
    )?;
    context
        .charge_work(decoded.len() as u64)
        .map_err(PdfError::from_context)?;
    Ok(passed && decoded.as_ref() == expected)
}

fn text_string(
    source: &PdfDocument,
    value: &Value,
    context: &OperationContext,
) -> Result<String, PdfError> {
    let Value::String(bytes) = resolve(source, value, context)? else {
        return Err(PdfError::unsupported(
            "text field value must be a PDF text string",
        ));
    };
    context
        .charge_work(bytes.len() as u64)
        .map_err(PdfError::from_context)?;
    if bytes.len() > source.parsed().limits.max_token_bytes {
        return Err(PdfError::limit("text field value exceeds token limit"));
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err(PdfError::unsupported(
                "text field value has truncated UTF16",
            ));
        }
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .map_err(|_| PdfError::unsupported("text field value has invalid UTF16"))
    } else if bytes.is_ascii() {
        Ok(String::from_utf8(bytes.clone()).expect("ASCII"))
    } else {
        Err(PdfError::unsupported(
            "text field value encoding cannot be decoded without loss",
        ))
    }
}

fn bound(source: &PdfDocument, plan: &TextFillPlan) -> Result<(), PdfError> {
    if plan.snapshot.source() != source.source_id()
        || !source.parsed().objects.shares_snapshot_with(&plan.store)
    {
        Err(PdfError::selection(
            "text field plan source/object-store mismatch",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        OpenOptions, PdfEngine, PdfErrorCode, PdfFieldFlattenRequest, PdfRevision,
        PdfRevisionRequest,
    };
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget, RevisionId};

    fn fixture() -> PdfDocument {
        PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/text-field-fill.pdf"),
                OpenOptions::default(),
            )
            .unwrap()
    }
    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 128 * 1024 * 1024,
                max_work_units: 512 * 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }
    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn request(value: &str) -> PdfTextFieldFillRequest {
        PdfTextFieldFillRequest {
            field_name: "group.name".into(),
            value: value.into(),
            match_index: 0,
        }
    }
    fn planned(source: &PdfDocument, ctx: &OperationContext) -> Result<TextFillPlan, PdfError> {
        let index = PdfReferenceIndex::build(source, ctx)?;
        plan(
            source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &index,
            &[request("Café (A) \\ B")],
            &BTreeMap::new(),
            ctx,
        )
    }
    fn staged(source: &PdfDocument, plan: &TextFillPlan, ctx: &OperationContext) -> PdfDocument {
        let mut changes = BTreeMap::new();
        apply(source, plan, &mut changes, ctx).unwrap();
        let mut parsed = source.parsed().clone();
        for (reference, object) in changes {
            parsed.objects.insert(reference, object);
        }
        source.with_parsed(parsed)
    }
    fn changed(source: &PdfDocument, number: u32, change: impl FnOnce(&mut Dict)) -> PdfDocument {
        let mut parsed = source.parsed().clone();
        let Value::Dict(values) = &mut parsed.objects.get_mut(&reference(number)).unwrap().value
        else {
            panic!("dictionary")
        };
        change(values);
        source.with_parsed(parsed)
    }
    fn rejected(
        source: &PdfDocument,
        candidate: &PdfDocument,
        plan: &TextFillPlan,
        ctx: &OperationContext,
    ) {
        if let Ok(report) = verify(source, candidate, plan, ctx) {
            assert!(report.validate(&required_checks()).is_err());
        }
    }
    fn appearance_bytes(source: &PdfDocument) -> Vec<u8> {
        let object = source.parsed().object(reference(14)).unwrap();
        let mut budget = ParseBudget::for_document(source.parsed());
        decode_stream(
            &object.value,
            object.stream.as_deref().unwrap(),
            &source.parsed().limits,
            &mut budget,
        )
        .unwrap()
        .to_vec()
    }
    fn with_appearance(source: &PdfDocument, bytes: Vec<u8>) -> PdfDocument {
        let mut parsed = source.parsed().clone();
        let object = parsed.objects.get_mut(&reference(14)).unwrap();
        let Value::Dict(values) = &mut object.value else {
            panic!("AP dictionary")
        };
        values.remove(b"Filter".as_slice());
        values.remove(b"DecodeParms".as_slice());
        values.insert(b"Length".to_vec(), Value::Integer(bytes.len() as i64));
        object.stream = Some(bytes.into());
        source.with_parsed(parsed)
    }

    #[test]
    fn owner_local_clones_preserve_source_defaults_private_data_and_shared_owner() {
        let source = fixture();
        let ctx = context();
        let plan = planned(&source, &ctx).unwrap();
        let candidate = staged(&source, &plan, &ctx);
        verify(&source, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        assert_eq!(plan.effects[0].widgets[0].replacement_advance, 72.);
        assert_eq!(plan.effects[0].widgets[0].ink_bounds, [5.5, 7.5, 78.5, 19.]);
        assert_ne!(
            plan.effects[0].widgets[0].output_appearance,
            plan.effects[0].widgets[1].output_appearance
        );
        for number in [11, 14, 19, 20, 22, 33, 34] {
            assert!(
                flatten::objects_equal(
                    source.parsed().object(reference(number)).unwrap(),
                    candidate.parsed().object(reference(number)).unwrap(),
                    &source,
                    &ctx
                )
                .unwrap()
            );
        }
        for (number, key) in [(8, b"DV".as_slice()), (9, b"Rect".as_slice())] {
            let wrong = changed(&candidate, number, |values| {
                values.insert(key.to_vec(), Value::Null);
            });
            rejected(&source, &wrong, &plan, &ctx);
        }
    }

    #[test]
    fn self_consistent_appearance_and_authorization_corruption_fails_source_checks() {
        let source = fixture();
        let ctx = context();
        let mut plan = planned(&source, &ctx).unwrap();
        let output = plan.widgets[&reference(9)].appearance;
        let Value::Dict(values) = &mut plan.replacements.get_mut(&output).unwrap().value else {
            panic!("appearance")
        };
        values.insert(
            b"BBox".to_vec(),
            Value::Array(vec![
                Value::Integer(0),
                Value::Integer(0),
                Value::Integer(300),
                Value::Integer(300),
            ]),
        );
        let candidate = staged(&source, &plan, &ctx);
        rejected(&source, &candidate, &plan, &ctx);
        let mut plan = planned(&source, &ctx).unwrap();
        let invented = reference(300);
        plan.authorized.insert(invented);
        plan.replacements.insert(
            invented,
            IndirectObject {
                value: Value::Dict(Dict::new()),
                stream: None,
                offset: 0,
                stream_offset: 0,
            },
        );
        let candidate = staged(&source, &plan, &ctx);
        rejected(&source, &candidate, &plan, &ctx);
    }

    #[test]
    fn stale_appearance_opaque_ownership_and_unproven_fit_are_refused() {
        let source = fixture();
        let stale = changed(&source, 8, |values| {
            values.insert(b"V".to_vec(), Value::String(b"Different".to_vec()));
        });
        assert!(planned(&stale, &context()).is_err());
        let aliased = changed(&source, 1, |values| {
            values.insert(b"OpaqueWidget".to_vec(), Value::Ref(reference(9)));
        });
        assert!(planned(&aliased, &context()).is_err());
        let narrow = changed(&source, 14, |values| {
            values.insert(
                b"BBox".to_vec(),
                Value::Array([0, 0, 70, 32].map(Value::Integer).to_vec()),
            );
        });
        assert!(planned(&narrow, &context()).is_err());
        let no_widths = changed(&source, 19, |values| {
            values.remove(b"Widths".as_slice());
        });
        assert!(planned(&no_widths, &context()).is_err());
    }

    #[test]
    fn tx_marked_content_supports_fill_then_flatten_without_accepting_other_tags() {
        let source = fixture();
        let original = appearance_bytes(&source);
        let mut bytes = b"/Tx BMC\n".to_vec();
        bytes.extend_from_slice(&original);
        bytes.extend_from_slice(b"\nEMC\n");
        let source = with_appearance(&source, bytes);
        let ctx = context();
        let mut revision = PdfRevision::new(source);
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    text_fields: vec![request("Ready")],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        revision.commit(&plan, &ctx).unwrap();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    flatten_fields: vec![PdfFieldFlattenRequest {
                        field_name: "group.name".into(),
                        match_index: 0,
                    }],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        revision.commit(&plan, &ctx).unwrap();
        for prefix in [
            b"/Other BMC\n".as_slice(),
            b"/Tx << /MCID 0 >> BDC\n".as_slice(),
        ] {
            let mut bytes = prefix.to_vec();
            bytes.extend_from_slice(&original);
            bytes.extend_from_slice(b"\nEMC\n");
            assert!(planned(&with_appearance(&fixture(), bytes), &context()).is_err());
        }
    }

    #[test]
    fn clipping_path_cannot_change_after_the_clip_operator() {
        let source = fixture();
        let bytes =
            b"q 2 2 176 28 re W* 2 2 176 28 re n BT /F1 10 Tf 1 0 0 1 6 10 Tm (Old) Tj ET Q\n"
                .to_vec();
        assert!(planned(&with_appearance(&source, bytes), &context()).is_err());
    }

    #[test]
    fn rejected_large_value_scans_are_precharged_and_cancelled() {
        let source = fixture();
        let registry = flatten::registry(&source, &context()).unwrap();
        let request = request(&("A".repeat(2048) + "\n"));
        let ctx = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1_000_000,
                max_work_units: 1024,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            select(&source, &registry, &request, &ctx).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        let ctx = context();
        ctx.cancellation().cancel();
        assert_eq!(
            select(&source, &registry, &request, &ctx).unwrap_err().code,
            PdfErrorCode::Cancelled
        );
    }

    #[test]
    fn fresh_appearance_identity_must_respect_source_allocation_floor() {
        let source = fixture();
        let ctx = context();
        let mut plan = planned(&source, &ctx).unwrap();
        let widget = reference(9);
        let original = plan.widgets[&widget].appearance;
        let invalid = reference(0);
        let appearance = plan.replacements.remove(&original).unwrap();
        plan.replacements.insert(invalid, appearance);
        plan.authorized.remove(&original);
        plan.authorized.insert(invalid);
        plan.widgets.get_mut(&widget).unwrap().appearance = invalid;
        let dictionary = plan.widgets[&widget].dictionary.unwrap();
        let Value::Dict(values) = &mut plan.replacements.get_mut(&dictionary).unwrap().value else {
            panic!("AP dictionary")
        };
        values.insert(b"N".to_vec(), Value::Ref(invalid));
        plan.effects[0].widgets[0].output_appearance = public_ref(invalid);
        let candidate = staged(&source, &plan, &ctx);
        rejected(&source, &candidate, &plan, &ctx);
    }
}
