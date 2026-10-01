//! One retained page occurrence with explicitly independent, named Tx fields.
//! Original page targets remain original targets. Encoded content, appearance
//! and resource objects are shared; page-owned annotations and fields are new.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SnapshotId,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};
use serde::{Deserialize, Serialize};

use crate::{
    PdfDocument, PdfError, PdfReferenceIndex, PdfReferenceOrigin, PdfReferencePathStep,
    PdfReferenceTarget,
    annotations::encode_text,
    forms::flatten::{self, Registry, dict, object_ref, present, public_ref, reserve, resolve},
    object::{IndirectObject, ObjectRef, Value},
    store::ObjectStore,
};

type Dict = BTreeMap<Vec<u8>, Value>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PdfDuplicateFieldName {
    pub field_name: String,
    pub new_field_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PdfPageDuplicateRequest {
    pub page_index: usize,
    /// Every terminal field on this page requires a unique explicit new name.
    pub fields: Vec<PdfDuplicateFieldName>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDuplicatedAnnotationEffect {
    pub source_annotation: PdfReferenceTarget,
    pub annotation: PdfReferenceTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDuplicatedFieldEffect {
    pub source_field: PdfReferenceTarget,
    pub field: PdfReferenceTarget,
    pub field_name: String,
    pub new_field_name: String,
    pub widgets: Vec<PdfDuplicatedAnnotationEffect>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfPageDuplicateEffect {
    pub source_page: PdfReferenceTarget,
    pub page: PdfReferenceTarget,
    pub page_index: usize,
    pub inserted_index: usize,
    pub fields: Vec<PdfDuplicatedFieldEffect>,
    pub annotations: Vec<PdfDuplicatedAnnotationEffect>,
}

#[derive(Debug)]
pub(crate) struct DuplicatePlan {
    snapshot: SnapshotId,
    store: ObjectStore,
    request: PdfPageDuplicateRequest,
    effect: PdfPageDuplicateEffect,
    replacements: BTreeMap<ObjectRef, IndirectObject>,
    authorized: BTreeSet<ObjectRef>,
    expected_order: Vec<ObjectRef>,
    source_indices: Vec<usize>,
    changes_catalog_acroform: bool,
    _reservations: Vec<Reservation>,
}

impl DuplicatePlan {
    pub(crate) fn effects(&self) -> &PdfPageDuplicateEffect {
        &self.effect
    }
    pub(crate) fn changed_refs(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.authorized.iter().copied()
    }
    pub(crate) fn expected_order(&self) -> &[ObjectRef] {
        &self.expected_order
    }
    pub(crate) fn source_indices(&self) -> &[usize] {
        &self.source_indices
    }
    pub(crate) fn changes_catalog_acroform(&self) -> bool {
        self.changes_catalog_acroform
    }
}

struct Field {
    source: ObjectRef,
    name: String,
    new_name: String,
    local_name: String,
    parent: Option<ObjectRef>,
    combined: bool,
    widgets: Vec<ObjectRef>,
}

struct Selection {
    page: ObjectRef,
    pages: Vec<ObjectRef>,
    ancestors: Vec<ObjectRef>,
    annotations: Vec<ObjectRef>,
    fields: Vec<Field>,
    registry: Option<Registry>,
    catalog: ObjectRef,
    _reservations: Vec<Reservation>,
}

pub(crate) fn plan(
    source: &PdfDocument,
    snapshot: SnapshotId,
    index: &PdfReferenceIndex,
    request: &PdfPageDuplicateRequest,
    staged: &BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<DuplicatePlan, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if snapshot.source() != source.source_id() || index.snapshot() != snapshot {
        return Err(PdfError::selection("duplicate source/revision mismatch"));
    }
    let fresh = PdfReferenceIndex::build(source, context)?;
    work(context, fresh.records().len())?;
    if fresh.records() != index.records() {
        return Err(PdfError::selection(
            "duplicate index belongs to another object-store branch",
        ));
    }
    let selected = select(source, request, &fresh, context)?;
    let mut reservations = Vec::new();
    lookup_work(&selected, context)?;
    reserve(
        &mut reservations,
        selection_cost(&selected, request)?,
        context,
    )?;
    let mut next = flatten::next_number(source, staged)?;
    let mut count = 0usize;
    let mut allocate = || {
        count = count
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("duplicate object count overflows"))?;
        flatten::allocate(&mut next, source, staged, count)
    };
    let page = allocate()?;
    let mut field_map = BTreeMap::new();
    let mut annotation_map = BTreeMap::new();
    for annotation in &selected.annotations {
        work(context, 1)?;
        let field = selected
            .fields
            .iter()
            .find(|field| field.widgets.contains(annotation));
        if let Some(field) = field {
            if let std::collections::btree_map::Entry::Vacant(entry) = field_map.entry(field.source)
            {
                entry.insert(allocate()?);
            }
            if field.combined {
                annotation_map.insert(*annotation, field_map[&field.source]);
                continue;
            }
        }
        annotation_map.insert(*annotation, allocate()?);
    }
    let effect = effects(&selected, request, page, &field_map, &annotation_map);
    let mut replacements = source_changes(
        source,
        &selected,
        page,
        &field_map,
        &mut reservations,
        context,
    )?;
    let mut page_object = clone_dictionary(source, selected.page, &mut reservations, context)?;
    if present(dict(&page_object.value)?, b"Annots").is_some() {
        dictionary_mut(&mut page_object)?.insert(
            b"Annots".to_vec(),
            Value::Array(
                selected
                    .annotations
                    .iter()
                    .map(|old| Value::Ref(annotation_map[old]))
                    .collect(),
            ),
        );
    }
    replacements.insert(page, page_object);
    for field in &selected.fields {
        let mut object = clone_dictionary(source, field.source, &mut reservations, context)?;
        let values = dictionary_mut(&mut object)?;
        values.insert(b"T".to_vec(), Value::String(encode_text(&field.local_name)));
        if field.combined {
            values.insert(b"P".to_vec(), Value::Ref(page));
        } else {
            values.insert(
                b"Kids".to_vec(),
                Value::Array(
                    field
                        .widgets
                        .iter()
                        .map(|widget| Value::Ref(annotation_map[widget]))
                        .collect(),
                ),
            );
        }
        replacements.insert(field_map[&field.source], object);
    }
    for annotation in &selected.annotations {
        if selected
            .fields
            .iter()
            .any(|field| field.combined && field.source == *annotation)
        {
            continue;
        }
        let mut object = clone_dictionary(source, *annotation, &mut reservations, context)?;
        let values = dictionary_mut(&mut object)?;
        values.insert(b"P".to_vec(), Value::Ref(page));
        if let Some(field) = selected
            .fields
            .iter()
            .find(|field| field.widgets.contains(annotation))
        {
            values.insert(b"Parent".to_vec(), Value::Ref(field_map[&field.source]));
        }
        replacements.insert(annotation_map[annotation], object);
    }
    let authorized = authorization(&selected, page, &field_map, &annotation_map);
    for (reference, object) in &replacements {
        work(context, 1)?;
        if staged.contains_key(reference) {
            return Err(PdfError::unsafe_rewrite(
                "duplicate overlaps another staged object",
            ));
        }
        flatten::reserve_value(&object.value, source, &mut reservations, context)?;
    }
    let mut expected_order = selected.pages.clone();
    expected_order.insert(request.page_index + 1, page);
    let mut source_indices: Vec<_> = (0..selected.pages.len()).collect();
    source_indices.insert(request.page_index + 1, request.page_index);
    let changes_catalog_acroform = selected.registry.as_ref().is_some_and(|registry| {
        registry.acro_direct && selected.fields.iter().any(|field| field.parent.is_none())
    });
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(DuplicatePlan {
        snapshot,
        store: source.parsed().objects.clone(),
        request: request.clone(),
        effect,
        replacements,
        authorized,
        expected_order,
        source_indices,
        changes_catalog_acroform,
        _reservations: reservations,
    })
}

pub(crate) fn apply(
    source: &PdfDocument,
    plan: &DuplicatePlan,
    changes: &mut BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    bound(source, plan)?;
    for reference in plan.replacements.keys() {
        work(context, 1)?;
        if changes.contains_key(reference) {
            return Err(PdfError::unsafe_rewrite(
                "duplicate overlaps staged objects",
            ));
        }
    }
    for (reference, object) in &plan.replacements {
        work(
            context,
            flatten::value_cost(&object.value, source, 0, context)?,
        )?;
        changes.insert(*reference, object.clone());
    }
    Ok(())
}

const CHECKS: [&str; 5] = [
    "page_occurrence",
    "independent_fields",
    "annotation_ownership",
    "source_preservation",
    "authorized_changes",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.duplicate", name).expect("static duplicate check")
}
pub(crate) fn required_checks() -> RequiredChecks {
    RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
        .expect("static duplicate requirements")
}

pub(crate) fn verify(
    source: &PdfDocument,
    candidate: &PdfDocument,
    plan: &DuplicatePlan,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    bound(source, plan)?;
    if candidate.source_id() != source.source_id() {
        return Err(PdfError::selection("duplicate candidate source mismatch"));
    }
    let fresh = PdfReferenceIndex::build(source, context)?;
    let selected = select(source, &plan.request, &fresh, context)?;
    let mut reservations = Vec::new();
    lookup_work(&selected, context)?;
    reserve(
        &mut reservations,
        selection_cost(&selected, &plan.request)?,
        context,
    )?;
    let page = object_ref(plan.effect.page);
    let mut field_map = BTreeMap::new();
    let mut annotation_map = BTreeMap::new();
    for field in &plan.effect.fields {
        field_map.insert(object_ref(field.source_field), object_ref(field.field));
    }
    for annotation in &plan.effect.annotations {
        annotation_map.insert(
            object_ref(annotation.source_annotation),
            object_ref(annotation.annotation),
        );
    }
    let mapping_keys = field_map.keys().copied().collect::<BTreeSet<_>>()
        == selected.fields.iter().map(|field| field.source).collect()
        && annotation_map.keys().copied().collect::<BTreeSet<_>>()
            == selected.annotations.iter().copied().collect()
        && field_map.len() == plan.effect.fields.len()
        && annotation_map.len() == plan.effect.annotations.len();
    if !mapping_keys {
        return Ok(report([false; 5]));
    }
    let new_objects: BTreeSet<_> = std::iter::once(page)
        .chain(field_map.values().copied())
        .chain(annotation_map.values().copied())
        .collect();
    let combined = selected
        .fields
        .iter()
        .filter(|field| field.combined)
        .count();
    let floor = flatten::next_number(source, &BTreeMap::new())?;
    let fresh_ids = new_objects.len()
        == 1 + selected.fields.len() + selected.annotations.len() - combined
        && new_objects.iter().all(|reference| {
            reference.generation == 0
                && reference.number >= floor
                && !source.parsed().objects.contains_key(reference)
        })
        && selected
            .fields
            .iter()
            .filter(|field| field.combined)
            .all(|field| field_map[&field.source] == annotation_map[&field.source]);
    if !fresh_ids {
        return Ok(report([false; 5]));
    }
    let expected_effect = effects(&selected, &plan.request, page, &field_map, &annotation_map);
    let mut order = selected.pages.clone();
    order.insert(plan.request.page_index + 1, page);
    let mut indices: Vec<_> = (0..selected.pages.len()).collect();
    indices.insert(plan.request.page_index + 1, plan.request.page_index);
    let page_values = dict(&source.parsed().object(selected.page)?.value)?;
    let page_annots = present(page_values, b"Annots").map(|_| {
        Value::Array(
            selected
                .annotations
                .iter()
                .map(|old| Value::Ref(annotation_map[old]))
                .collect(),
        )
    });
    let page_ok = candidate.page_refs()? == order
        && plan.expected_order == order
        && plan.source_indices == indices
        && compare_clone(
            source,
            candidate,
            selected.page,
            page,
            &BTreeMap::from_iter(page_annots.map(|value| (b"Annots".to_vec(), value))),
            context,
        )?;
    super::pages::validate_tree(candidate, context)?;
    let mut fields_ok = expected_effect == plan.effect;
    for field in &selected.fields {
        work(context, 1)?;
        let mut changed =
            BTreeMap::from([(b"T".to_vec(), Value::String(encode_text(&field.local_name)))]);
        if field.combined {
            changed.insert(b"P".to_vec(), Value::Ref(page));
        } else {
            changed.insert(
                b"Kids".to_vec(),
                Value::Array(
                    field
                        .widgets
                        .iter()
                        .map(|widget| Value::Ref(annotation_map[widget]))
                        .collect(),
                ),
            );
        }
        fields_ok &= compare_clone(
            source,
            candidate,
            field.source,
            field_map[&field.source],
            &changed,
            context,
        )?;
    }
    let mut annotations_ok = true;
    for annotation in &selected.annotations {
        work(context, 1)?;
        if selected
            .fields
            .iter()
            .any(|field| field.combined && field.source == *annotation)
        {
            continue;
        }
        let mut changed = BTreeMap::from([(b"P".to_vec(), Value::Ref(page))]);
        if let Some(field) = selected
            .fields
            .iter()
            .find(|field| field.widgets.contains(annotation))
        {
            changed.insert(b"Parent".to_vec(), Value::Ref(field_map[&field.source]));
        }
        annotations_ok &= compare_clone(
            source,
            candidate,
            *annotation,
            annotation_map[annotation],
            &changed,
            context,
        )?;
    }
    let expected_source = source_changes(
        source,
        &selected,
        page,
        &field_map,
        &mut reservations,
        context,
    )?;
    let mut label_roles = BTreeSet::new();
    super::labels::visit_tree_objects(source, context, |reference| {
        reserve(&mut reservations, std::mem::size_of::<ObjectRef>(), context)?;
        label_roles.insert(reference);
        Ok(())
    })?;
    let mut preserved = true;
    for (reference, before) in &source.parsed().objects {
        work(context, 1)?;
        let after = candidate.parsed().object(*reference)?;
        let expected = expected_source.get(reference).unwrap_or(before);
        if *reference == selected.catalog {
            // The enclosing transaction independently checks the exact source
            // label occurrence map and all other catalog keys.
            work(
                context,
                flatten::value_cost(&expected.value, source, 0, context)?,
            )?;
            work(
                context,
                flatten::value_cost(&after.value, source, 0, context)?,
            )?;
            preserved &= equal_except(
                dict(&expected.value)?,
                dict(&after.value)?,
                &[b"PageLabels"],
                &BTreeMap::new(),
            ) && after.stream.is_none();
        } else if !label_roles.contains(reference) {
            preserved &= flatten::objects_equal(expected, after, source, context)?;
        }
    }
    let authorized = authorization(&selected, page, &field_map, &annotation_map);
    let catalog_change = selected.registry.as_ref().is_some_and(|registry| {
        registry.acro_direct && selected.fields.iter().any(|field| field.parent.is_none())
    });
    let authorized_ok = authorized == plan.authorized
        && plan
            .replacements
            .keys()
            .all(|reference| authorized.contains(reference))
        && catalog_change == plan.changes_catalog_acroform
        && candidate.parsed().objects.keys().all(|reference| {
            source.parsed().objects.contains_key(reference) || new_objects.contains(reference)
        });
    Ok(report([
        page_ok,
        fields_ok,
        annotations_ok,
        preserved,
        authorized_ok,
    ]))
}

fn report(results: [bool; 5]) -> EvidenceReport<String> {
    EvidenceReport::new(CHECKS.into_iter().zip(results).map(|(name, passed)| CheckEvidence {
        code: code(name), status: if passed { CheckStatus::Passed } else { CheckStatus::Failed },
        detail: "One adjacent page occurrence; explicit independent Tx names, new page-owned annotations, unchanged original targets and immutable encoded appearances/content. Labels are checked separately. No tag remapping, shared-value fields, appearance regeneration or cross-document import assurance.".into(),
    }).collect())
}

fn select(
    source: &PdfDocument,
    request: &PdfPageDuplicateRequest,
    index: &PdfReferenceIndex,
    context: &OperationContext,
) -> Result<Selection, PdfError> {
    crate::writer::refuse_security_boundaries_with_context(source.parsed(), Some(context))?;
    super::pages::validate_tree(source, context)?;
    let limits = &source.parsed().limits;
    if request.fields.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "duplicate field request count exceeds limit",
        ));
    }
    let pages = source.page_refs()?;
    if pages.len() >= limits.max_pages || pages.len() >= limits.max_container_items {
        return Err(PdfError::limit(
            "duplicated page exceeds page/container limit",
        ));
    }
    let page = *pages
        .get(request.page_index)
        .ok_or_else(|| PdfError::selection("duplicate page index is out of range"))?;
    let catalog = match dict(&source.parsed().trailer)?.get(b"Root".as_slice()) {
        Some(Value::Ref(reference)) => *reference,
        _ => {
            return Err(PdfError::unsupported(
                "duplicate requires an indirect catalog",
            ));
        }
    };
    let catalog_values = dict(&source.parsed().object(catalog)?.value)?;
    if present(catalog_values, b"StructTreeRoot").is_some() {
        return Err(PdfError::unsupported(
            "duplicate does not remap structure trees or tagged associations",
        ));
    }
    let registry = present(catalog_values, b"AcroForm")
        .map(|_| flatten::registry(source, context))
        .transpose()?;
    if registry.is_none() {
        for object in source.parsed().objects.values() {
            flatten::refuse_actions(&object.value, source, 0, context)?;
        }
    }
    let values = dict(&source.parsed().object(page)?.value)?;
    let mut reservations = Vec::new();
    reserve(
        &mut reservations,
        pages
            .len()
            .checked_mul(std::mem::size_of::<ObjectRef>() * 4)
            .ok_or_else(|| PdfError::limit("duplicate page workspace overflows"))?,
        context,
    )?;
    let mut ancestors = Vec::new();
    let mut current = direct_ref(present(values, b"Parent"), "page Parent")?;
    loop {
        work(context, 1)?;
        if ancestors.len() > limits.max_parser_depth || ancestors.contains(&current) {
            return Err(PdfError::unsafe_rewrite(
                "duplicate ancestor path is cyclic or too deep",
            ));
        }
        reserve(
            &mut reservations,
            std::mem::size_of::<ObjectRef>() * 2,
            context,
        )?;
        ancestors.push(current);
        let parent = dict(&source.parsed().object(current)?.value)?;
        if !matches!(present(parent, b"Type"), Some(Value::Name(kind)) if kind == b"Pages") {
            return Err(PdfError::unsafe_rewrite(
                "duplicate ancestor is not a Pages node",
            ));
        }
        match present(parent, b"Parent") {
            None => break,
            parent => current = direct_ref(parent, "Pages Parent")?,
        }
    }
    let mut owned: BTreeSet<_> = pages
        .iter()
        .copied()
        .chain(ancestors.iter().copied())
        .collect();
    if let Some(registry) = &registry {
        reserve(
            &mut reservations,
            registry
                .nodes
                .len()
                .checked_add(registry.containers.len())
                .and_then(|count| count.checked_mul(std::mem::size_of::<ObjectRef>()))
                .ok_or_else(|| PdfError::limit("duplicate ownership workspace overflows"))?,
            context,
        )?;
        owned.extend(registry.nodes.keys().copied());
        owned.extend(registry.containers.iter().copied());
    }
    if present(values, b"Resources").is_none() {
        for ancestor in &ancestors {
            work(context, 1)?;
            if let Some(resources) = present(
                dict(&source.parsed().object(*ancestor)?.value)?,
                b"Resources",
            ) {
                asset_closure(
                    source,
                    resources,
                    &owned,
                    &mut BTreeSet::new(),
                    0,
                    context,
                    &mut reservations,
                )?;
                break;
            }
        }
    }
    for key in [
        b"AA".as_slice(),
        b"StructParents",
        b"StructParent",
        b"B",
        b"PieceInfo",
        b"SeparationInfo",
        b"PresSteps",
        b"AF",
        b"VP",
    ] {
        if present(values, key).is_some() {
            return Err(PdfError::unsupported(
                "duplicate page has unsupported actions, tags or occurrence-dependent extensions",
            ));
        }
    }
    for (key, value) in values {
        work(context, 1)?;
        if matches!(key.as_slice(), b"Type" | b"Parent" | b"Annots") {
            continue;
        }
        if matches!(
            key.as_slice(),
            b"Resources" | b"Contents" | b"Metadata" | b"Group"
        ) {
            asset_closure(
                source,
                value,
                &owned,
                &mut BTreeSet::new(),
                0,
                context,
                &mut reservations,
            )?;
        } else {
            opaque_value(value, source, 0, context)?;
        }
    }
    let (annotations, annotation_owner, annotation_path) =
        annotation_array(source, page, values, context)?;
    reserve(
        &mut reservations,
        annotations
            .len()
            .checked_mul(std::mem::size_of::<ObjectRef>() * 8)
            .ok_or_else(|| PdfError::limit("duplicate annotation workspace overflows"))?,
        context,
    )?;
    work(
        context,
        annotations
            .len()
            .checked_mul(annotations.len().saturating_add(request.fields.len()))
            .ok_or_else(|| PdfError::limit("duplicate selection search work overflows"))?,
    )?;
    let mut seen = BTreeSet::new();
    let mut field_order = Vec::new();
    for (position, annotation) in annotations.iter().enumerate() {
        work(context, 1)?;
        if !seen.insert(*annotation) {
            return Err(PdfError::unsafe_rewrite(
                "duplicate page repeats an annotation",
            ));
        }
        let object = source.parsed().object(*annotation)?;
        if object.stream.is_some() {
            return Err(PdfError::unsupported(
                "duplicate annotation cannot be a stream",
            ));
        }
        let values = dict(&object.value)?;
        if present(values, b"P") != Some(&Value::Ref(page)) {
            return Err(PdfError::unsafe_rewrite(
                "duplicate requires exact annotation page ownership",
            ));
        }
        flatten::rectangle(resolve(
            source,
            present(values, b"Rect")
                .ok_or_else(|| PdfError::unsupported("duplicate annotation has no Rect"))?,
            context,
        )?)?;
        match present(values, b"Subtype") {
            Some(Value::Name(kind)) if kind == b"Widget" => {
                let registry = registry.as_ref().ok_or_else(|| {
                    PdfError::unsafe_rewrite("duplicate widget has no AcroForm registration")
                })?;
                let node = registry
                    .nodes
                    .get(annotation)
                    .filter(|node| node.widget)
                    .ok_or_else(|| {
                        PdfError::unsafe_rewrite("duplicate widget is not actively registered")
                    })?;
                if registry
                    .pages
                    .get(annotation)
                    .is_none_or(|pages| pages.as_slice() != [page])
                {
                    return Err(PdfError::unsafe_rewrite(
                        "duplicate widget has ambiguous page ownership",
                    ));
                }
                let field = if present(values, b"T").is_some() {
                    *annotation
                } else {
                    node.parent.ok_or_else(|| {
                        PdfError::unsupported(
                            "duplicate unnamed root widget is not a terminal field",
                        )
                    })?
                };
                if !field_order.contains(&field) {
                    field_order.push(field);
                }
                flatten::check_incoming(registry, index, *annotation, context)?;
            }
            Some(Value::Name(kind)) if kind == b"Link" => {
                let mut path = annotation_path.clone();
                path.push(PdfReferencePathStep::Index(position));
                let incoming = index.incoming(index.snapshot(), public_ref(*annotation))?;
                if incoming.len() != 1
                    || incoming[0].origin
                        != PdfReferenceOrigin::Object(public_ref(annotation_owner))
                    || incoming[0].path != path
                    || incoming[0].dangling
                {
                    return Err(PdfError::unsafe_rewrite(
                        "duplicate link has additional or ambiguous owners",
                    ));
                }
                validate_link(source, values, &pages, context)?;
            }
            _ => {
                return Err(PdfError::unsupported(
                    "duplicate supports only registered Tx widgets and simple links",
                ));
            }
        }
    }
    if request.fields.len() != field_order.len() {
        return Err(PdfError::selection(
            "duplicate requires one explicit new name for every selected-page terminal field",
        ));
    }
    let mut names = BTreeMap::new();
    let mut new_names = BTreeSet::new();
    for rename in &request.fields {
        let bytes = rename
            .field_name
            .len()
            .checked_add(rename.new_field_name.len())
            .ok_or_else(|| PdfError::limit("duplicate name bytes overflow"))?;
        work(context, bytes)?;
        reserve(
            &mut reservations,
            bytes
                .checked_mul(4)
                .ok_or_else(|| PdfError::limit("duplicate names workspace overflows"))?,
            context,
        )?;
        if rename.field_name.is_empty()
            || rename.new_field_name.is_empty()
            || rename.field_name.len() > limits.max_token_bytes
            || rename.new_field_name.len() > limits.max_token_bytes
            || rename.new_field_name.chars().any(char::is_control)
            || rename.new_field_name.split('.').any(str::is_empty)
        {
            return Err(PdfError::selection(
                "duplicate requires bounded nonempty explicit field names without controls",
            ));
        }
        let registry = registry
            .as_ref()
            .ok_or_else(|| PdfError::selection("duplicate names supplied without a form"))?;
        let field = *registry
            .names
            .get(&rename.field_name)
            .ok_or_else(|| PdfError::selection("duplicate source field name is not registered"))?;
        if !field_order.contains(&field)
            || names.insert(field, rename).is_some()
            || registry.names.contains_key(&rename.new_field_name)
            || !new_names.insert(rename.new_field_name.clone())
        {
            return Err(PdfError::selection(
                "duplicate field names overlap, collide or select a different page",
            ));
        }
    }
    let mut fields = Vec::new();
    if let Some(registry) = &registry {
        for field in field_order {
            work(context, 1)?;
            let rename = names[&field];
            let (old_prefix, _) = split_name(&rename.field_name);
            let (new_prefix, local_name) = split_name(&rename.new_field_name);
            if old_prefix != new_prefix || local_name.is_empty() {
                return Err(PdfError::selection(
                    "duplicate independent names must retain the existing parent-qualified prefix",
                ));
            }
            if encode_text(local_name).len() > limits.max_token_bytes {
                return Err(PdfError::limit(
                    "duplicate encoded partial field name exceeds token limit",
                ));
            }
            let node = &registry.nodes[&field];
            if !matches!(flatten::inherited(source, registry, field, b"FT", context)?, Some(Value::Name(kind)) if kind == b"Tx")
            {
                return Err(PdfError::unsupported(
                    "duplicate independent fields currently require Tx",
                ));
            }
            let flags = flatten::inherited(source, registry, field, b"Ff", context)?;
            if !matches!(flags, None | Some(Value::Integer(0 | 2 | 4 | 6))) {
                return Err(PdfError::unsupported(
                    "duplicate requires ordinary writable Tx flags",
                ));
            }
            let field_values = dict(&source.parsed().object(field)?.value)?;
            if present(field_values, b"T").is_none() || present(field_values, b"TM").is_some() {
                return Err(PdfError::unsupported(
                    "duplicate requires a locally named field without export mapping aliases",
                ));
            }
            let mut ancestor = Some(field);
            while let Some(reference) = ancestor {
                work(context, 1)?;
                flatten::selected_policy(dict(&source.parsed().object(reference)?.value)?)?;
                flatten::check_incoming(registry, index, reference, context)?;
                ancestor = registry.nodes[&reference].parent;
            }
            let all_widgets = if node.widget {
                vec![field]
            } else {
                if node.children.is_empty()
                    || node.children.iter().any(|child| {
                        !registry.nodes[child].widget
                            || !registry.nodes[child].children.is_empty()
                            || present(
                                dict(
                                    &source
                                        .parsed()
                                        .object(*child)
                                        .expect("registered widget")
                                        .value,
                                )
                                .expect("registered dictionary"),
                                b"T",
                            )
                            .is_some()
                    })
                {
                    return Err(PdfError::unsupported(
                        "duplicate selects a terminal Tx field with widget-only children",
                    ));
                }
                node.children.clone()
            };
            let mut widgets = Vec::new();
            for widget in all_widgets {
                if !seen.contains(&widget) {
                    continue;
                }
                let values = dict(&source.parsed().object(widget)?.value)?;
                validate_widget(
                    source,
                    values,
                    !node.widget,
                    &owned,
                    context,
                    &mut reservations,
                )?;
                widgets.push(widget);
            }
            if widgets.is_empty() {
                return Err(PdfError::unsafe_rewrite(
                    "duplicate terminal field has no selected-page widgets",
                ));
            }
            for (key, value) in field_values {
                if matches!(key.as_slice(), b"Parent" | b"Kids" | b"P" | b"AP") {
                    continue;
                }
                if !matches!(
                    key.as_slice(),
                    b"Type"
                        | b"Subtype"
                        | b"T"
                        | b"TU"
                        | b"FT"
                        | b"Ff"
                        | b"V"
                        | b"DV"
                        | b"DA"
                        | b"Q"
                        | b"MaxLen"
                        | b"Rect"
                        | b"F"
                        | b"BS"
                        | b"Border"
                        | b"C"
                ) {
                    opaque_value(value, source, 0, context)?;
                }
            }
            fields.push(Field {
                source: field,
                name: rename.field_name.clone(),
                new_name: rename.new_field_name.clone(),
                local_name: local_name.to_owned(),
                parent: node.parent,
                combined: node.widget,
                widgets,
            });
        }
        if fields.iter().any(|field| field.parent.is_none()) && !registry.acro_direct {
            flatten::check_incoming(registry, index, registry.acro_owner, context)?;
        }
        for container in &registry.containers {
            flatten::check_incoming(registry, index, *container, context)?;
        }
    }
    Ok(Selection {
        page,
        pages,
        ancestors,
        annotations,
        fields,
        registry,
        catalog,
        _reservations: reservations,
    })
}

fn source_changes(
    source: &PdfDocument,
    selected: &Selection,
    page: ObjectRef,
    fields: &BTreeMap<ObjectRef, ObjectRef>,
    reservations: &mut Vec<Reservation>,
    context: &OperationContext,
) -> Result<BTreeMap<ObjectRef, IndirectObject>, PdfError> {
    let mut changes = BTreeMap::new();
    for (depth, reference) in selected.ancestors.iter().enumerate() {
        work(context, 1)?;
        flatten::reserve_value(
            &source.parsed().object(*reference)?.value,
            source,
            reservations,
            context,
        )?;
        let mut object = source.parsed().object(*reference)?.clone();
        let values = dictionary_mut(&mut object)?;
        let Some(Value::Integer(count)) = values.get_mut(b"Count".as_slice()) else {
            return Err(PdfError::unsafe_rewrite(
                "duplicate source Pages Count is malformed",
            ));
        };
        *count = count
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("duplicate Pages Count overflows"))?;
        if depth == 0 {
            let Some(Value::Array(kids)) = values.get_mut(b"Kids".as_slice()) else {
                return Err(PdfError::unsafe_rewrite(
                    "duplicate source Pages Kids is malformed",
                ));
            };
            if kids.len() >= source.parsed().limits.max_container_items {
                return Err(PdfError::limit("duplicate Pages Kids exceeds limit"));
            }
            let position = kids
                .iter()
                .position(|value| *value == Value::Ref(selected.page))
                .ok_or_else(|| {
                    PdfError::unsafe_rewrite("duplicate source leaf is absent from its parent")
                })?;
            kids.insert(position + 1, Value::Ref(page));
        }
        changes.insert(*reference, object);
    }
    let Some(registry) = &selected.registry else {
        return Ok(changes);
    };
    let mut appended: BTreeMap<Option<ObjectRef>, Vec<Value>> = BTreeMap::new();
    for field in &selected.fields {
        appended
            .entry(field.parent)
            .or_default()
            .push(Value::Ref(fields[&field.source]));
    }
    for (parent, added) in appended {
        work(context, 1)?;
        let owner = parent.unwrap_or(registry.acro_owner);
        flatten::reserve_value(
            &source.parsed().object(owner)?.value,
            source,
            reservations,
            context,
        )?;
        let (owner, mut object) = match parent {
            Some(parent) => (parent, source.parsed().object(parent)?.clone()),
            None => (
                registry.acro_owner,
                source.parsed().object(registry.acro_owner)?.clone(),
            ),
        };
        let values = dictionary_mut(&mut object)?;
        let (values, key, old) = if let Some(parent) = parent {
            (
                values,
                b"Kids".as_slice(),
                &registry.nodes[&parent].children,
            )
        } else if registry.acro_direct {
            let Some(Value::Dict(acro)) = values.get_mut(b"AcroForm".as_slice()) else {
                return Err(PdfError::unsafe_rewrite(
                    "duplicate direct AcroForm is malformed",
                ));
            };
            (acro, b"Fields".as_slice(), &registry.roots)
        } else {
            (values, b"Fields".as_slice(), &registry.roots)
        };
        if old
            .len()
            .checked_add(added.len())
            .is_none_or(|count| count > source.parsed().limits.max_container_items)
        {
            return Err(PdfError::limit(
                "duplicate field registration exceeds container limit",
            ));
        }
        let mut kids: Vec<_> = old.iter().copied().map(Value::Ref).collect();
        kids.extend(added);
        values.insert(key.to_vec(), Value::Array(kids));
        if changes.insert(owner, object).is_some() {
            return Err(PdfError::unsafe_rewrite(
                "duplicate source roles alias one object",
            ));
        }
    }
    Ok(changes)
}

fn authorization(
    selected: &Selection,
    page: ObjectRef,
    fields: &BTreeMap<ObjectRef, ObjectRef>,
    annotations: &BTreeMap<ObjectRef, ObjectRef>,
) -> BTreeSet<ObjectRef> {
    std::iter::once(page)
        .chain(selected.ancestors.iter().copied())
        .chain(fields.values().copied())
        .chain(annotations.values().copied())
        .chain(selected.fields.iter().map(|field| {
            field.parent.unwrap_or_else(|| {
                selected
                    .registry
                    .as_ref()
                    .expect("selected form registry")
                    .acro_owner
            })
        }))
        .collect()
}

fn effects(
    selected: &Selection,
    request: &PdfPageDuplicateRequest,
    page: ObjectRef,
    fields: &BTreeMap<ObjectRef, ObjectRef>,
    annotations: &BTreeMap<ObjectRef, ObjectRef>,
) -> PdfPageDuplicateEffect {
    let annotation = |source| PdfDuplicatedAnnotationEffect {
        source_annotation: public_ref(source),
        annotation: public_ref(annotations[&source]),
    };
    PdfPageDuplicateEffect {
        source_page: public_ref(selected.page),
        page: public_ref(page),
        page_index: request.page_index,
        inserted_index: request.page_index + 1,
        fields: selected
            .fields
            .iter()
            .map(|field| PdfDuplicatedFieldEffect {
                source_field: public_ref(field.source),
                field: public_ref(fields[&field.source]),
                field_name: field.name.clone(),
                new_field_name: field.new_name.clone(),
                widgets: field.widgets.iter().copied().map(&annotation).collect(),
            })
            .collect(),
        annotations: selected
            .annotations
            .iter()
            .copied()
            .map(annotation)
            .collect(),
    }
}

fn annotation_array(
    source: &PdfDocument,
    page: ObjectRef,
    values: &Dict,
    context: &OperationContext,
) -> Result<(Vec<ObjectRef>, ObjectRef, Vec<PdfReferencePathStep>), PdfError> {
    let Some(mut value) = present(values, b"Annots") else {
        return Ok((Vec::new(), page, Vec::new()));
    };
    let mut owner = page;
    let mut path = vec![PdfReferencePathStep::Key(b"Annots".to_vec())];
    for _ in 0..=source.parsed().limits.max_parser_depth {
        work(context, 1)?;
        match value {
            Value::Ref(reference) => {
                let object = source.parsed().object(*reference)?;
                if object.stream.is_some() {
                    return Err(PdfError::unsupported(
                        "duplicate Annots container cannot be stream",
                    ));
                }
                owner = *reference;
                path.clear();
                value = &object.value;
            }
            Value::Array(values) if values.len() <= source.parsed().limits.max_container_items => {
                let refs = values
                    .iter()
                    .map(|value| direct_ref(Some(value), "annotation"))
                    .collect::<Result<_, _>>()?;
                return Ok((refs, owner, path));
            }
            _ => {
                return Err(PdfError::unsupported(
                    "duplicate Annots requires a bounded array of references",
                ));
            }
        }
    }
    Err(PdfError::limit(
        "duplicate Annots indirection exceeds limit",
    ))
}

fn validate_widget(
    source: &PdfDocument,
    values: &Dict,
    separate: bool,
    owned: &BTreeSet<ObjectRef>,
    context: &OperationContext,
    reservations: &mut Vec<Reservation>,
) -> Result<(), PdfError> {
    flatten::selected_policy(values)?;
    if separate
        && [b"FT".as_slice(), b"Ff", b"V", b"DV", b"T"]
            .iter()
            .any(|key| present(values, key).is_some())
    {
        return Err(PdfError::unsupported(
            "duplicate separate widget also owns field values",
        ));
    }
    for key in [b"Popup".as_slice(), b"IRT", b"RT", b"Dest", b"AS"] {
        if present(values, key).is_some() {
            return Err(PdfError::unsupported(
                "duplicate widget has unsupported associations or state",
            ));
        }
    }
    let ap_value = present(values, b"AP").ok_or_else(|| {
        PdfError::unsupported("duplicate Tx widget requires an existing normal appearance")
    })?;
    let ap = dict(resolve(source, ap_value, context)?)?;
    if present(ap, b"R").is_some() || present(ap, b"D").is_some() {
        return Err(PdfError::unsupported(
            "duplicate Tx widget requires only a normal appearance",
        ));
    }
    let normal = direct_ref(present(ap, b"N"), "normal appearance")?;
    let object = source.parsed().object(normal)?;
    let normal_values = dict(&object.value)?;
    if object.stream.is_none()
        || !matches!(present(normal_values, b"Subtype"), Some(Value::Name(kind)) if kind == b"Form")
    {
        return Err(PdfError::unsupported(
            "duplicate normal appearance must be a Form stream",
        ));
    }
    let rect = flatten::rectangle(resolve(
        source,
        present(values, b"Rect").expect("checked Rect"),
        context,
    )?)?;
    let bbox = flatten::rectangle(resolve(
        source,
        present(normal_values, b"BBox")
            .ok_or_else(|| PdfError::unsupported("duplicate normal appearance has no BBox"))?,
        context,
    )?)?;
    flatten::placement(
        rect,
        bbox,
        flatten::appearance_matrix(source, normal_values, context)?,
    )?;
    asset_closure(
        source,
        ap_value,
        owned,
        &mut BTreeSet::new(),
        0,
        context,
        reservations,
    )?;
    for (key, value) in values {
        if !matches!(
            key.as_slice(),
            b"Type"
                | b"Subtype"
                | b"Parent"
                | b"P"
                | b"AP"
                | b"FT"
                | b"Ff"
                | b"T"
                | b"TU"
                | b"V"
                | b"DV"
                | b"DA"
                | b"Q"
                | b"MaxLen"
                | b"Rect"
                | b"F"
                | b"BS"
                | b"Border"
                | b"C"
        ) {
            opaque_value(value, source, 0, context)?;
        }
    }
    Ok(())
}

fn validate_link(
    source: &PdfDocument,
    values: &Dict,
    pages: &[ObjectRef],
    context: &OperationContext,
) -> Result<(), PdfError> {
    for key in [
        b"AA".as_slice(),
        b"StructParent",
        b"StructParents",
        b"OC",
        b"Popup",
        b"IRT",
        b"RT",
        b"AP",
        b"Parent",
    ] {
        if present(values, key).is_some() {
            return Err(PdfError::unsupported(
                "duplicate link has unsupported appearance or ownership associations",
            ));
        }
    }
    match (present(values, b"Dest"), present(values, b"A")) {
        (Some(Value::Array(destination)), None) if destination.len() == 2 => {
            let target = direct_ref(destination.first(), "local destination page")?;
            if !pages.contains(&target) || destination[1] != Value::Name(b"Fit".to_vec()) {
                return Err(PdfError::unsupported(
                    "duplicate local link requires an existing original page and Fit destination",
                ));
            }
        }
        (None, Some(action)) => {
            let action = dict(resolve(source, action, context)?)?;
            if !matches!(present(action, b"S"), Some(Value::Name(kind)) if kind == b"URI")
                || !matches!(present(action, b"URI"), Some(Value::String(_)))
                || action
                    .keys()
                    .any(|key| !matches!(key.as_slice(), b"Type" | b"S" | b"URI" | b"IsMap"))
            {
                return Err(PdfError::unsupported(
                    "duplicate link supports a simple URI action without chained actions",
                ));
            }
            if let Some(value) = present(action, b"IsMap")
                && !matches!(value, Value::Bool(_))
            {
                return Err(PdfError::unsupported("duplicate URI IsMap must be boolean"));
            }
        }
        _ => {
            return Err(PdfError::unsupported(
                "duplicate link requires exactly one direct local Fit destination or URI action",
            ));
        }
    }
    for (key, value) in values {
        if !matches!(
            key.as_slice(),
            b"Type"
                | b"Subtype"
                | b"P"
                | b"Rect"
                | b"F"
                | b"Dest"
                | b"A"
                | b"Border"
                | b"BS"
                | b"C"
                | b"H"
                | b"Contents"
        ) {
            opaque_value(value, source, 0, context)?;
        }
    }
    Ok(())
}

fn asset_closure(
    source: &PdfDocument,
    value: &Value,
    owned: &BTreeSet<ObjectRef>,
    seen: &mut BTreeSet<ObjectRef>,
    depth: usize,
    context: &OperationContext,
    reservations: &mut Vec<Reservation>,
) -> Result<(), PdfError> {
    work(context, 1)?;
    if depth > source.parsed().limits.max_parser_depth
        || seen.len() > source.parsed().limits.max_container_items
    {
        return Err(PdfError::limit(
            "duplicate shared asset closure exceeds limit",
        ));
    }
    match value {
        Value::Ref(reference) => {
            if owned.contains(reference) {
                return Err(PdfError::unsupported(
                    "duplicate shared asset references a page or form-owned object",
                ));
            }
            if !seen.contains(reference) {
                reserve(reservations, std::mem::size_of::<ObjectRef>(), context)?;
                seen.insert(*reference);
                let object = source.parsed().object(*reference)?;
                if let Value::Dict(values) = &object.value
                    && (matches!(present(values, b"Type"), Some(Value::Name(kind)) if matches!(kind.as_slice(), b"Page" | b"Pages" | b"Catalog" | b"Annot"))
                        || present(values, b"StructParent").is_some()
                        || present(values, b"StructParents").is_some())
                {
                    return Err(PdfError::unsupported(
                        "duplicate shared asset has an occurrence-dependent role",
                    ));
                }
                asset_closure(
                    source,
                    &object.value,
                    owned,
                    seen,
                    depth + 1,
                    context,
                    reservations,
                )?;
            }
        }
        Value::Array(values) => {
            for value in values {
                asset_closure(source, value, owned, seen, depth + 1, context, reservations)?;
            }
        }
        Value::Dict(values) => {
            for value in values.values() {
                asset_closure(source, value, owned, seen, depth + 1, context, reservations)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn opaque_value(
    value: &Value,
    source: &PdfDocument,
    depth: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    work(context, 1)?;
    if depth > source.parsed().limits.max_parser_depth {
        return Err(PdfError::limit(
            "duplicate opaque value depth exceeds limit",
        ));
    }
    match value {
        Value::Ref(_) => Err(PdfError::unsupported(
            "duplicate cannot determine ownership of an opaque extension reference",
        )),
        Value::Array(values) => {
            for value in values {
                opaque_value(value, source, depth + 1, context)?;
            }
            Ok(())
        }
        Value::Dict(values) => {
            for value in values.values() {
                opaque_value(value, source, depth + 1, context)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn compare_clone(
    source: &PdfDocument,
    candidate: &PdfDocument,
    before: ObjectRef,
    after: ObjectRef,
    replacements: &Dict,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    let before = source.parsed().object(before)?;
    let after = candidate.parsed().object(after)?;
    work(
        context,
        flatten::value_cost(&before.value, source, 0, context)?,
    )?;
    work(
        context,
        flatten::value_cost(&after.value, source, 0, context)?,
    )?;
    Ok(before.stream.is_none()
        && after.stream.is_none()
        && equal_except(dict(&before.value)?, dict(&after.value)?, &[], replacements))
}

fn equal_except(before: &Dict, after: &Dict, skipped: &[&[u8]], changed: &Dict) -> bool {
    before.iter().all(|(key, value)| {
        skipped.contains(&key.as_slice())
            || after.get(key) == Some(changed.get(key).unwrap_or(value))
    }) && after.iter().all(|(key, value)| {
        skipped.contains(&key.as_slice())
            || Some(value) == changed.get(key).or_else(|| before.get(key))
    }) && changed
        .iter()
        .all(|(key, value)| after.get(key) == Some(value))
}

fn clone_dictionary(
    source: &PdfDocument,
    reference: ObjectRef,
    reservations: &mut Vec<Reservation>,
    context: &OperationContext,
) -> Result<IndirectObject, PdfError> {
    let object = source.parsed().object(reference)?;
    if object.stream.is_some() {
        return Err(PdfError::unsupported(
            "duplicate dictionary role cannot be a stream",
        ));
    }
    flatten::reserve_value(&object.value, source, reservations, context)?;
    let mut object = object.clone();
    object.offset = 0;
    object.stream_offset = 0;
    Ok(object)
}

fn dictionary_mut(object: &mut IndirectObject) -> Result<&mut Dict, PdfError> {
    match &mut object.value {
        Value::Dict(values) => Ok(values),
        _ => Err(PdfError::unsupported("duplicate requires a dictionary")),
    }
}
fn direct_ref(value: Option<&Value>, label: &str) -> Result<ObjectRef, PdfError> {
    match value {
        Some(Value::Ref(reference)) => Ok(*reference),
        _ => Err(PdfError::unsupported(format!(
            "duplicate {label} must be indirect"
        ))),
    }
}
fn split_name(name: &str) -> (&str, &str) {
    name.rsplit_once('.').unwrap_or(("", name))
}
fn work(context: &OperationContext, amount: usize) -> Result<(), PdfError> {
    context
        .charge_work(amount as u64)
        .map_err(PdfError::from_context)
}
fn lookup_work(selected: &Selection, context: &OperationContext) -> Result<(), PdfError> {
    let items = selected
        .annotations
        .len()
        .checked_add(selected.fields.len())
        .ok_or_else(|| PdfError::limit("duplicate lookup work overflows"))?;
    let work_units = items
        .checked_mul(items)
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| PdfError::limit("duplicate lookup work overflows"))?;
    work(context, work_units)
}
fn bound(source: &PdfDocument, plan: &DuplicatePlan) -> Result<(), PdfError> {
    if plan.snapshot.source() != source.source_id()
        || !source.parsed().objects.shares_snapshot_with(&plan.store)
    {
        Err(PdfError::selection(
            "duplicate plan source/object-store mismatch",
        ))
    } else {
        Ok(())
    }
}
fn selection_cost(
    selected: &Selection,
    request: &PdfPageDuplicateRequest,
) -> Result<usize, PdfError> {
    selected
        .fields
        .len()
        .checked_mul(
            std::mem::size_of::<Field>() + std::mem::size_of::<PdfDuplicatedFieldEffect>() * 2,
        )
        .and_then(|count| {
            selected
                .annotations
                .len()
                .checked_mul(std::mem::size_of::<PdfDuplicatedAnnotationEffect>() * 4)
                .and_then(|extra| count.checked_add(extra))
        })
        .and_then(|count| {
            selected
                .pages
                .len()
                .checked_mul((std::mem::size_of::<ObjectRef>() + std::mem::size_of::<usize>()) * 2)
                .and_then(|extra| count.checked_add(extra))
        })
        .and_then(|count| {
            request.fields.iter().try_fold(count, |sum, field| {
                field
                    .field_name
                    .len()
                    .checked_add(field.new_field_name.len())
                    .and_then(|bytes| bytes.checked_mul(4))
                    .and_then(|bytes| sum.checked_add(bytes))
            })
        })
        .ok_or_else(|| PdfError::limit("duplicate plan workspace overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine};
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
                max_live_bytes: 32 * 1024 * 1024,
                max_work_units: 64 * 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }
    fn request() -> PdfPageDuplicateRequest {
        PdfPageDuplicateRequest {
            page_index: 0,
            fields: vec![
                PdfDuplicateFieldName {
                    field_name: "group.name".into(),
                    new_field_name: "group.name_copy".into(),
                },
                PdfDuplicateFieldName {
                    field_name: "group.keep".into(),
                    new_field_name: "group.keep_copy".into(),
                },
            ],
        }
    }
    fn planned(source: &PdfDocument, context: &OperationContext) -> DuplicatePlan {
        plan(
            source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &PdfReferenceIndex::build(source, context).unwrap(),
            &request(),
            &BTreeMap::new(),
            context,
        )
        .unwrap()
    }
    fn staged(
        source: &PdfDocument,
        plan: &DuplicatePlan,
        context: &OperationContext,
    ) -> PdfDocument {
        let mut changes = BTreeMap::new();
        apply(source, plan, &mut changes, context).unwrap();
        let mut parsed = source.parsed().clone();
        for (reference, object) in changes {
            parsed.objects.insert(reference, object);
        }
        source.with_parsed(parsed)
    }
    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn changed(source: &PdfDocument, number: u32, change: impl FnOnce(&mut Dict)) -> PdfDocument {
        let mut parsed = source.parsed().clone();
        change(dictionary_mut(parsed.objects.get_mut(&reference(number)).unwrap()).unwrap());
        source.with_parsed(parsed)
    }
    fn try_plan(
        source: &PdfDocument,
        request: &PdfPageDuplicateRequest,
        context: &OperationContext,
    ) -> Result<DuplicatePlan, PdfError> {
        plan(
            source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &PdfReferenceIndex::build(source, context)?,
            request,
            &BTreeMap::new(),
            context,
        )
    }
    fn rejected(
        source: &PdfDocument,
        candidate: &PdfDocument,
        plan: &DuplicatePlan,
        context: &OperationContext,
    ) {
        if let Ok(report) = verify(source, candidate, plan, context) {
            assert!(report.validate(&required_checks()).is_err());
        }
    }
    #[test]
    fn duplicate_owns_new_widgets_and_keeps_original_field_and_destination() {
        let source = fixture();
        let context = context();
        let plan = planned(&source, &context);
        let candidate = staged(&source, &plan, &context);
        verify(&source, &candidate, &plan, &context)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        assert_eq!(plan.effect.fields.len(), 2);
        assert_eq!(plan.effect.annotations.len(), 3);
        assert_eq!(plan.effect.fields[0].widgets.len(), 1);
        assert_eq!(plan.source_indices(), [0, 0, 1]);
        for number in [3, 4, 8, 9, 10, 11, 14, 18, 19, 33, 34] {
            let reference = ObjectRef {
                number,
                generation: 0,
            };
            assert!(
                flatten::objects_equal(
                    source.parsed().object(reference).unwrap(),
                    candidate.parsed().object(reference).unwrap(),
                    &source,
                    &context
                )
                .unwrap()
            );
        }
        let copied_link = candidate
            .parsed()
            .object(object_ref(plan.effect.annotations[2].annotation))
            .unwrap();
        assert_eq!(
            dict(&copied_link.value).unwrap().get(b"Dest".as_slice()),
            Some(&Value::Array(vec![
                Value::Ref(ObjectRef {
                    number: 4,
                    generation: 0
                }),
                Value::Name(b"Fit".to_vec())
            ]))
        );
    }

    #[test]
    fn self_consistent_clone_registration_and_source_corruption_fails() {
        let source = fixture();
        let ctx = context();
        for role in [
            "page",
            "widget",
            "field",
            "link",
            "registration",
            "count",
            "source",
            "opaque",
        ] {
            let mut plan = planned(&source, &ctx);
            let (target, key, value) = match role {
                "page" => (
                    object_ref(plan.effect.page),
                    b"Private".as_slice(),
                    Value::Null,
                ),
                "widget" => (
                    object_ref(plan.effect.annotations[0].annotation),
                    b"P".as_slice(),
                    Value::Ref(reference(4)),
                ),
                "field" => (
                    object_ref(plan.effect.fields[0].field),
                    b"Kids".as_slice(),
                    Value::Array(vec![Value::Ref(reference(10))]),
                ),
                "link" => (
                    object_ref(plan.effect.annotations[2].annotation),
                    b"Dest".as_slice(),
                    Value::Array(vec![
                        Value::Ref(object_ref(plan.effect.page)),
                        Value::Name(b"Fit".to_vec()),
                    ]),
                ),
                "registration" => (
                    reference(7),
                    b"Kids".as_slice(),
                    Value::Array(vec![Value::Ref(reference(8)), Value::Ref(reference(11))]),
                ),
                "count" => (reference(2), b"Count".as_slice(), Value::Integer(4)),
                "source" => (
                    reference(14),
                    b"BBox".as_slice(),
                    Value::Array([0, 0, 1, 1].map(Value::Integer).to_vec()),
                ),
                _ => (reference(34), b"Meaning".as_slice(), Value::Null),
            };
            if let std::collections::btree_map::Entry::Vacant(entry) =
                plan.replacements.entry(target)
            {
                entry.insert(source.parsed().object(target).unwrap().clone());
                plan.authorized.insert(target);
            }
            dictionary_mut(plan.replacements.get_mut(&target).unwrap())
                .unwrap()
                .insert(key.to_vec(), value);
            rejected(&source, &staged(&source, &plan, &ctx), &plan, &ctx);
        }
        let mut plan = planned(&source, &ctx);
        let invented = reference(500);
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
        rejected(&source, &staged(&source, &plan, &ctx), &plan, &ctx);
    }

    #[test]
    fn duplicated_identity_cannot_enter_a_source_free_slot_below_size() {
        let mut source = fixture();
        let mut parsed = source.parsed().clone();
        parsed.source_size = Some(100);
        source = source.with_parsed(parsed);
        let ctx = context();
        let mut plan = planned(&source, &ctx);
        let old = object_ref(plan.effect.annotations[2].annotation);
        let below = reference(50);
        let object = plan.replacements.remove(&old).unwrap();
        plan.replacements.insert(below, object);
        plan.authorized.remove(&old);
        plan.authorized.insert(below);
        plan.effect.annotations[2].annotation = public_ref(below);
        let new_page = dictionary_mut(
            plan.replacements
                .get_mut(&object_ref(plan.effect.page))
                .unwrap(),
        )
        .unwrap();
        let Value::Array(annotations) = new_page.get_mut(b"Annots".as_slice()).unwrap() else {
            panic!("Annots");
        };
        annotations[2] = Value::Ref(below);
        rejected(&source, &staged(&source, &plan, &ctx), &plan, &ctx);
    }

    #[test]
    fn ownership_actions_tags_and_opaque_page_dependencies_refuse() {
        let source = fixture();
        for (number, key, value) in [
            (1, b"OpaqueWidget".as_slice(), Value::Ref(reference(9))),
            (1, b"OpaqueLink".as_slice(), Value::Ref(reference(18))),
            (3, b"PrivateRef".as_slice(), Value::Ref(reference(8))),
            (32, b"OpaqueOwner".as_slice(), Value::Ref(reference(3))),
            (9, b"Popup".as_slice(), Value::Ref(reference(18))),
            (9, b"StructParent".as_slice(), Value::Integer(0)),
            (1, b"StructTreeRoot".as_slice(), Value::Dict(Dict::new())),
            (
                23,
                b"XFA".as_slice(),
                Value::String(b"unsupported".to_vec()),
            ),
            (
                23,
                b"CO".as_slice(),
                Value::Array(vec![Value::Ref(reference(8))]),
            ),
            (
                1,
                b"OpenAction".as_slice(),
                Value::Dict(Dict::from([(
                    b"S".to_vec(),
                    Value::Name(b"ResetForm".to_vec()),
                )])),
            ),
        ] {
            let changed = changed(&source, number, |values| {
                values.insert(key.to_vec(), value);
            });
            assert!(
                try_plan(&changed, &request(), &context()).is_err(),
                "number {number}, key {:?}",
                key
            );
        }
        let wrong_link = changed(&source, 18, |values| {
            values.insert(
                b"Dest".to_vec(),
                Value::Array(vec![
                    Value::Ref(reference(99)),
                    Value::Name(b"Fit".to_vec()),
                ]),
            );
        });
        assert!(try_plan(&wrong_link, &request(), &context()).is_err());
    }

    #[test]
    fn explicit_names_and_selected_field_coverage_are_enforced() {
        let source = fixture();
        for name in [
            "",
            "group.",
            "group..copy",
            "group.keep",
            "other.copy",
            "group.bad\nname",
        ] {
            let mut request = request();
            request.fields[0].new_field_name = name.into();
            assert!(try_plan(&source, &request, &context()).is_err(), "{name:?}");
        }
        let mut missing = request();
        missing.fields.pop();
        assert!(try_plan(&source, &missing, &context()).is_err());
        let mut duplicate = request();
        duplicate.fields[1].new_field_name = duplicate.fields[0].new_field_name.clone();
        assert!(try_plan(&source, &duplicate, &context()).is_err());
        let mut unicode = request();
        unicode.fields[0].new_field_name = "group.名前".into();
        let ctx = context();
        let plan = try_plan(&source, &unicode, &ctx).unwrap();
        let candidate = staged(&source, &plan, &ctx);
        verify(&source, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        let registry = flatten::registry(&candidate, &ctx).unwrap();
        assert_eq!(
            registry.names["group.名前"],
            object_ref(plan.effect.fields[0].field)
        );
    }

    #[test]
    fn plain_pages_and_nested_ancestor_counts_are_supported() {
        let source = fixture();
        let source = changed(&source, 1, |values| {
            values.remove(b"AcroForm".as_slice());
        });
        let source = changed(&source, 3, |values| {
            values.remove(b"Annots".as_slice());
            values.insert(b"Parent".to_vec(), Value::Ref(reference(35)));
        });
        let source = changed(&source, 4, |values| {
            values.remove(b"Annots".as_slice());
        });
        let source = changed(&source, 2, |values| {
            values.insert(
                b"Kids".to_vec(),
                Value::Array(vec![Value::Ref(reference(35)), Value::Ref(reference(4))]),
            );
        });
        let mut parsed = source.parsed().clone();
        parsed.objects.insert(
            reference(35),
            IndirectObject {
                value: Value::Dict(Dict::from([
                    (b"Type".to_vec(), Value::Name(b"Pages".to_vec())),
                    (b"Parent".to_vec(), Value::Ref(reference(2))),
                    (
                        b"Kids".to_vec(),
                        Value::Array(vec![Value::Ref(reference(3))]),
                    ),
                    (b"Count".to_vec(), Value::Integer(1)),
                ])),
                stream: None,
                offset: 0,
                stream_offset: 0,
            },
        );
        let source = source.with_parsed(parsed);
        let request = PdfPageDuplicateRequest {
            page_index: 0,
            fields: Vec::new(),
        };
        let ctx = context();
        let plan = try_plan(&source, &request, &ctx).unwrap();
        let candidate = staged(&source, &plan, &ctx);
        verify(&source, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        assert_eq!(
            dict(&candidate.parsed().object(reference(35)).unwrap().value).unwrap()
                [b"Count".as_slice()],
            Value::Integer(2)
        );
        assert_eq!(
            dict(&candidate.parsed().object(reference(2)).unwrap().value).unwrap()
                [b"Count".as_slice()],
            Value::Integer(3)
        );
        assert!(plan.effect.fields.is_empty());
        assert!(plan.effect.annotations.is_empty());
    }

    #[test]
    fn root_field_with_direct_acroform_keeps_independent_inherited_values() {
        let source = fixture();
        let source = changed(&source, 3, |values| {
            values.insert(
                b"Annots".to_vec(),
                Value::Array(vec![Value::Ref(reference(18))]),
            );
        });
        let source = changed(&source, 4, |values| {
            values.insert(
                b"Annots".to_vec(),
                Value::Array(vec![Value::Ref(reference(12))]),
            );
        });
        let source = changed(&source, 23, |values| {
            values.insert(
                b"Fields".to_vec(),
                Value::Array(vec![Value::Ref(reference(12))]),
            );
        });
        let acro = dict(&source.parsed().object(reference(23)).unwrap().value)
            .unwrap()
            .clone();
        let source = changed(&source, 1, |values| {
            values.insert(b"AcroForm".to_vec(), Value::Dict(acro));
        });
        let request = PdfPageDuplicateRequest {
            page_index: 1,
            fields: vec![PdfDuplicateFieldName {
                field_name: "code".into(),
                new_field_name: "code_copy".into(),
            }],
        };
        let ctx = context();
        let plan = try_plan(&source, &request, &ctx).unwrap();
        let candidate = staged(&source, &plan, &ctx);
        verify(&source, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        assert!(plan.changes_catalog_acroform());
        let root = dict(&candidate.parsed().object(reference(1)).unwrap().value).unwrap();
        let fields = dict(&root[b"AcroForm".as_slice()])
            .unwrap()
            .get(b"Fields".as_slice())
            .unwrap();
        assert_eq!(
            fields,
            &Value::Array(vec![
                Value::Ref(reference(12)),
                Value::Ref(object_ref(plan.effect.fields[0].field))
            ])
        );
        let mut bad = request.clone();
        bad.fields[0].new_field_name = ".copy".into();
        assert!(try_plan(&source, &bad, &context()).is_err());
        let inherited = changed(&fixture(), 7, |values| {
            values.insert(b"V".to_vec(), Value::String(b"Old".to_vec()));
            values.insert(b"DV".to_vec(), Value::String(b"Inherited default".to_vec()));
        });
        let inherited = changed(&inherited, 8, |values| {
            values.remove(b"V".as_slice());
            values.remove(b"DV".as_slice());
        });
        let plan = planned(&inherited, &ctx);
        let candidate = staged(&inherited, &plan, &ctx);
        let registry = flatten::registry(&candidate, &ctx).unwrap();
        assert_eq!(
            flatten::inherited(
                &candidate,
                &registry,
                object_ref(plan.effect.fields[0].field),
                b"DV",
                &ctx
            )
            .unwrap(),
            Some(&Value::String(b"Inherited default".to_vec()))
        );
    }

    #[test]
    fn cancellation_work_and_live_limits_do_not_publish_partial_changes() {
        let source = fixture();
        let token = CancellationToken::new();
        token.cancel();
        let cancelled = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 32 * 1024 * 1024,
                max_work_units: 64 * 1024 * 1024,
            }),
            token,
        );
        assert!(try_plan(&source, &request(), &cancelled).is_err());
        for limits in [
            BudgetLimits {
                max_live_bytes: 64,
                max_work_units: 64 * 1024 * 1024,
            },
            BudgetLimits {
                max_live_bytes: 32 * 1024 * 1024,
                max_work_units: 50,
            },
        ] {
            let limited =
                OperationContext::new(ResourceBudget::new(limits), CancellationToken::new());
            assert!(try_plan(&source, &request(), &limited).is_err());
        }
        let ctx = context();
        let plan = planned(&source, &ctx);
        let mut changes = BTreeMap::new();
        assert!(apply(&source, &plan, &mut changes, &cancelled).is_err());
        assert!(changes.is_empty());
        let candidate = staged(&source, &plan, &ctx);
        verify(&source, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
    }
}
