//! Selected terminal-field flattening in a retained revision. Existing normal
//! appearances become page Form invocations; detached field/widget objects are
//! retained verbatim. This is not redaction, appearance regeneration or signing.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SnapshotId,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};
use serde::{Deserialize, Serialize};

use crate::{
    PdfDocument, PdfError, PdfReferenceIndex, PdfReferenceOrigin, PdfReferencePathStep,
    PdfReferenceTarget,
    content::operations::{OperationState, read_operations_part},
    object::{IndirectObject, ObjectRef, Value},
    parser::{ParseBudget, decode_stream},
    payload::StreamPayload,
    store::ObjectStore,
};

type Dict = BTreeMap<Vec<u8>, Value>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PdfFieldFlattenRequest {
    pub field_name: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfFlattenedWidgetEffect {
    pub widget: PdfReferenceTarget,
    pub page: PdfReferenceTarget,
    pub appearance: PdfReferenceTarget,
    pub resource_name: Vec<u8>,
    /// Page placement preceding the Form's own Matrix.
    pub placement_matrix: [f64; 6],
    pub annotation_flags: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfFieldFlattenEffect {
    pub field: PdfReferenceTarget,
    pub widgets: Vec<PdfFlattenedWidgetEffect>,
}

#[derive(Debug)]
pub(crate) struct FlattenPlan {
    snapshot: SnapshotId,
    store: ObjectStore,
    requests: Vec<PdfFieldFlattenRequest>,
    effects: Vec<PdfFieldFlattenEffect>,
    replacements: BTreeMap<ObjectRef, IndirectObject>,
    authorized: BTreeSet<ObjectRef>,
    pages: BTreeMap<ObjectRef, PaintPage>,
    changes_catalog_acroform: bool,
    _reservations: Vec<Reservation>,
}

#[derive(Debug)]
struct PaintPage {
    prefix: ObjectRef,
    paint: ObjectRef,
    contents: Vec<ObjectRef>,
}

impl FlattenPlan {
    pub(crate) fn effects(&self) -> &[PdfFieldFlattenEffect] {
        &self.effects
    }
    pub(crate) fn changed_refs(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.authorized.iter().copied()
    }
    pub(crate) fn allows_page_key(&self, page: ObjectRef, key: &[u8]) -> bool {
        self.pages.contains_key(&page) && matches!(key, b"Annots" | b"Contents" | b"Resources")
    }
    pub(crate) fn changes_catalog_acroform(&self) -> bool {
        self.changes_catalog_acroform
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum Step {
    Key(Vec<u8>),
    Index(usize),
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct Edge {
    owner: ObjectRef,
    path: Vec<Step>,
}
impl Edge {
    fn key(owner: ObjectRef, key: &[u8]) -> Self {
        Self {
            owner,
            path: vec![Step::Key(key.to_vec())],
        }
    }
    fn child(&self, index: usize) -> Self {
        let mut edge = self.clone();
        edge.path.push(Step::Index(index));
        edge
    }
}
#[derive(Debug)]
pub(crate) struct FieldNode {
    pub(crate) parent: Option<ObjectRef>,
    pub(crate) children: Vec<ObjectRef>,
    pub(crate) widget: bool,
}
#[derive(Debug)]
pub(crate) struct Registry {
    pub(crate) catalog: ObjectRef,
    pub(crate) acro_owner: ObjectRef,
    pub(crate) acro_direct: bool,
    pub(crate) roots: Vec<ObjectRef>,
    pub(crate) nodes: BTreeMap<ObjectRef, FieldNode>,
    pub(crate) names: BTreeMap<String, ObjectRef>,
    incoming: BTreeMap<ObjectRef, BTreeSet<Edge>>,
    pub(crate) pages: BTreeMap<ObjectRef, Vec<ObjectRef>>,
    pub(crate) containers: BTreeSet<ObjectRef>,
    reachable: BTreeSet<ObjectRef>,
    reservations: Vec<Reservation>,
}

pub(crate) fn plan(
    source: &PdfDocument,
    snapshot: SnapshotId,
    index: &PdfReferenceIndex,
    requests: &[PdfFieldFlattenRequest],
    staged: &BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<FlattenPlan, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if snapshot.source() != source.source_id() || index.snapshot() != snapshot {
        return Err(PdfError::selection("flatten source/revision mismatch"));
    }
    if requests.is_empty() || requests.len() > source.parsed().limits.max_container_items {
        return Err(PdfError::limit(
            "flatten request count exceeds limit or is empty",
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
            "flatten index belongs to a different object-store branch",
        ));
    }
    let mut registry = registry(source, context)?;
    let selection_bytes = registry
        .nodes
        .len()
        .checked_mul(std::mem::size_of::<PdfFlattenedWidgetEffect>() * 2)
        .ok_or_else(|| PdfError::limit("flatten selection workspace overflows"))?;
    reserve(&mut registry.reservations, selection_bytes, context)?;
    let mut effects = select(source, &registry, requests, context)?;
    let removed = removed_nodes(&registry, &effects, context)?;
    for reference in removed.iter().chain(registry.containers.iter()) {
        check_incoming(&registry, &fresh, *reference, context)?;
    }
    // Altering a registration container also requires proven incoming ownership.
    let registration = registration_changes(source, &registry, &removed, context)?;
    let changes_catalog_acroform =
        registry.acro_direct && registration.contains_key(&registry.catalog);
    for reference in registration.keys() {
        if *reference != registry.catalog {
            check_incoming(&registry, &fresh, *reference, context)?;
        }
    }
    let mut replacements = BTreeMap::new();
    let mut authorized = BTreeSet::new();
    for (reference, value) in registration {
        let mut object = composed_object(
            source,
            staged,
            reference,
            &mut registry.reservations,
            context,
        )?;
        if reference == registry.catalog {
            let Value::Dict(ref mut values) = object.value else {
                unreachable!()
            };
            values.insert(b"AcroForm".to_vec(), value);
        } else {
            if staged.contains_key(&reference) {
                return Err(PdfError::unsafe_rewrite(
                    "flatten conflicts with a field registration mutation",
                ));
            }
            object.value = value;
        }
        authorized.insert(reference);
        replacements.insert(reference, object);
    }
    let mut selected_pages: BTreeMap<ObjectRef, Vec<(usize, usize)>> = BTreeMap::new();
    for (field_index, field) in effects.iter().enumerate() {
        for (widget_index, widget) in field.widgets.iter().enumerate() {
            selected_pages
                .entry(object_ref(widget.page))
                .or_default()
                .push((field_index, widget_index));
        }
    }
    let mut next = next_number(source, staged)?;
    let mut pages = BTreeMap::new();
    for (page, mut positions) in selected_pages {
        order_positions(source, page, &mut positions, &effects, context)?;
        let mut object =
            composed_object(source, staged, page, &mut registry.reservations, context)?;
        let original = dict(&source.parsed().object(page)?.value)?;
        let values = dict(&object.value)?;
        for key in [b"Contents".as_slice(), b"Annots"] {
            if original.get(key) != values.get(key) {
                return Err(PdfError::unsafe_rewrite(
                    "flatten overlaps another page content or annotation mutation",
                ));
            }
        }
        let resources = resources(source, page, context)?;
        if values.get(b"Resources".as_slice()) != original.get(b"Resources".as_slice()) {
            let staged_resources = values
                .get(b"Resources".as_slice())
                .map(|value| resolve(source, value, context))
                .transpose()?;
            if staged_resources != Some(&Value::Dict(resources.clone())) {
                return Err(PdfError::unsafe_rewrite(
                    "flatten conflicts with changed effective page resources",
                ));
            }
        }
        let mut resources = resources;
        let mut xobjects = match present(&resources, b"XObject") {
            Some(value) => dict(resolve(source, value, context)?)?.clone(),
            None => Dict::new(),
        };
        for (field_index, widget_index) in &positions {
            let name = unused_resource_name(&xobjects, source, context)?;
            let effect = &mut effects[*field_index].widgets[*widget_index];
            effect.resource_name = name.clone();
            xobjects.insert(name, Value::Ref(object_ref(effect.appearance)));
        }
        resources.insert(b"XObject".to_vec(), Value::Dict(xobjects));
        let contents = contents(source, page, context)?;
        validate_content_balance(source, &contents, context)?;
        let prefix = allocate(&mut next, source, staged, pages.len() * 2 + 1)?;
        let paint = allocate(&mut next, source, staged, pages.len() * 2 + 2)?;
        let prefix_object = stream_object(b"q\n".to_vec(), context)?;
        let paint_bytes = paint_program(
            positions
                .iter()
                .map(|(field, widget)| &effects[*field].widgets[*widget]),
            source,
            context,
        )?;
        let paint_object = stream_object(paint_bytes, context)?;
        let mut ordered = Vec::with_capacity(contents.len() + 2);
        ordered.push(Value::Ref(prefix));
        ordered.extend(contents.iter().copied().map(Value::Ref));
        ordered.push(Value::Ref(paint));
        let Value::Dict(ref mut values) = object.value else {
            unreachable!()
        };
        values.insert(b"Contents".to_vec(), Value::Array(ordered));
        values.insert(b"Resources".to_vec(), Value::Dict(resources));
        let annots = array(
            source,
            original
                .get(b"Annots".as_slice())
                .ok_or_else(|| PdfError::verification("selected widget page lost Annots"))?,
            context,
        )?;
        values.insert(b"Annots".to_vec(), Value::Array(annots.iter().filter(|value| !matches!(value, Value::Ref(reference) if removed.contains(reference))).cloned().collect()));
        reserve_value(&object.value, source, &mut registry.reservations, context)?;
        authorized.extend([page, prefix, paint]);
        replacements.extend([
            (page, object),
            (prefix, prefix_object),
            (paint, paint_object),
        ]);
        pages.insert(
            page,
            PaintPage {
                prefix,
                paint,
                contents,
            },
        );
    }
    for request in requests {
        reserve(
            &mut registry.reservations,
            request.field_name.len() + std::mem::size_of::<PdfFieldFlattenRequest>(),
            context,
        )?;
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(FlattenPlan {
        snapshot,
        store: source.parsed().objects.clone(),
        requests: requests.to_vec(),
        effects,
        replacements,
        authorized,
        pages,
        changes_catalog_acroform,
        _reservations: registry.reservations,
    })
}

pub(crate) fn apply(
    source: &PdfDocument,
    plan: &FlattenPlan,
    changes: &mut BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    bound(source, plan)?;
    for (reference, object) in &plan.replacements {
        context
            .charge_work(value_cost(&object.value, source, 0, context)? as u64)
            .map_err(PdfError::from_context)?;
        changes.insert(*reference, object.clone());
    }
    Ok(())
}

const CHECKS: [&str; 4] = [
    "selected_registration",
    "page_appearance_placement",
    "source_objects",
    "authorized_changes",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.flatten", name).expect("static flatten code")
}
pub(crate) fn required_checks() -> RequiredChecks {
    RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
        .expect("static flatten requirements")
}

pub(crate) fn verify(
    source: &PdfDocument,
    candidate: &PdfDocument,
    plan: &FlattenPlan,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    bound(source, plan)?;
    if candidate.source_id() != source.source_id() {
        return Err(PdfError::selection("flatten candidate source mismatch"));
    }
    crate::preservation::pages::validate_tree(candidate, context)?;
    let registry = registry(source, context)?;
    let mut selected = select(source, &registry, &plan.requests, context)?;
    let removed = removed_nodes(&registry, &selected, context)?;
    let expected_registration = registration_changes(source, &registry, &removed, context)?;
    let mut registered = true;
    let mut preserved = true;
    let mut placed = true;
    let mut authorized: BTreeSet<ObjectRef> = expected_registration.keys().copied().collect();
    let original_acro =
        dict(&source.parsed().object(registry.catalog)?.value)?.get(b"AcroForm".as_slice());
    let current_acro =
        dict(&candidate.parsed().object(registry.catalog)?.value)?.get(b"AcroForm".as_slice());
    registered &= current_acro
        == if registry.acro_direct {
            expected_registration
                .get(&registry.catalog)
                .or(original_acro)
        } else {
            original_acro
        };
    if !registry.acro_direct && !expected_registration.contains_key(&registry.acro_owner) {
        registered &= objects_equal(
            source.parsed().object(registry.acro_owner)?,
            candidate.parsed().object(registry.acro_owner)?,
            source,
            context,
        )?;
    }
    for (reference, expected) in &expected_registration {
        let current = candidate.parsed().object(*reference)?;
        let actual = if *reference == registry.catalog {
            dict(&current.value)?.get(b"AcroForm".as_slice())
        } else {
            Some(&current.value)
        };
        registered &= actual == Some(expected) && current.stream.is_none();
    }
    let mut expected_pages: BTreeMap<ObjectRef, Vec<(usize, usize)>> = BTreeMap::new();
    for (field, effect) in selected.iter().enumerate() {
        for (widget, effect) in effect.widgets.iter().enumerate() {
            expected_pages
                .entry(object_ref(effect.page))
                .or_default()
                .push((field, widget));
        }
    }
    placed &= expected_pages.len() == plan.pages.len();
    let mut new_refs = BTreeSet::new();
    let allocation_floor = next_number(source, &BTreeMap::new())?;
    for (page, mut positions) in expected_pages {
        order_positions(source, page, &mut positions, &selected, context)?;
        let Some(paint) = plan.pages.get(&page) else {
            placed = false;
            continue;
        };
        authorized.extend([page, paint.prefix, paint.paint]);
        for reference in [paint.prefix, paint.paint] {
            placed &= reference.number >= allocation_floor
                && reference.generation == 0
                && !source.parsed().objects.contains_key(&reference)
                && new_refs.insert(reference);
        }
        let original = dict(&source.parsed().object(page)?.value)?;
        let actual = dict(&candidate.parsed().object(page)?.value)?;
        let mut expected_resources = resources(source, page, context)?;
        let mut xobjects = match present(&expected_resources, b"XObject") {
            Some(value) => dict(resolve(source, value, context)?)?.clone(),
            None => Dict::new(),
        };
        for (field, widget) in &positions {
            let effect = &mut selected[*field].widgets[*widget];
            effect.resource_name = unused_resource_name(&xobjects, source, context)?;
            xobjects.insert(
                effect.resource_name.clone(),
                Value::Ref(object_ref(effect.appearance)),
            );
        }
        expected_resources.insert(b"XObject".to_vec(), Value::Dict(xobjects));
        placed &= actual.get(b"Resources".as_slice()) == Some(&Value::Dict(expected_resources));
        let original_contents = contents(source, page, context)?;
        placed &= original_contents == paint.contents;
        let mut expected_contents = vec![Value::Ref(paint.prefix)];
        expected_contents.extend(original_contents.iter().copied().map(Value::Ref));
        expected_contents.push(Value::Ref(paint.paint));
        placed &= actual.get(b"Contents".as_slice()) == Some(&Value::Array(expected_contents));
        let annots = array(
            source,
            original
                .get(b"Annots".as_slice())
                .ok_or_else(|| PdfError::verification("flatten source page has no Annots"))?,
            context,
        )?;
        let expected_annots = Value::Array(
            annots
                .iter()
                .filter(
                    |value| !matches!(value, Value::Ref(reference) if removed.contains(reference)),
                )
                .cloned()
                .collect(),
        );
        registered &= actual.get(b"Annots".as_slice()) == Some(&expected_annots);
        placed &= exact_stream(candidate, paint.prefix, b"q\n", context)?;
        let program = paint_program(
            positions
                .iter()
                .map(|(field, widget)| &selected[*field].widgets[*widget]),
            source,
            context,
        )?;
        placed &= exact_stream(candidate, paint.paint, &program, context)?;
    }
    placed &= selected == plan.effects;
    // Source field/widget/AP/resources and indirect array objects stay exact.
    // Only independently derived registration dictionaries and selected page
    // slots may change here; other operations own their disjoint source checks.
    let mut protected: BTreeSet<ObjectRef> = registry
        .nodes
        .keys()
        .chain(registry.containers.iter())
        .copied()
        .collect();
    for effect in &selected {
        for widget in &effect.widgets {
            collect_closure(
                source,
                object_ref(widget.appearance),
                &mut protected,
                context,
            )?;
        }
    }
    for reference in protected {
        if authorized.contains(&reference) {
            continue;
        }
        preserved &= objects_equal(
            source.parsed().object(reference)?,
            candidate.parsed().object(reference)?,
            source,
            context,
        )?;
    }
    let authorization = (registry.acro_direct
        && expected_registration.contains_key(&registry.catalog))
        == plan.changes_catalog_acroform
        && authorized == plan.authorized
        && plan
            .replacements
            .keys()
            .all(|reference| authorized.contains(reference));
    Ok(EvidenceReport::new(CHECKS.into_iter().zip([registered, placed, preserved, authorization]).map(|(name, passed)| CheckEvidence { code: code(name), status: if passed { CheckStatus::Passed } else { CheckStatus::Failed }, detail: "Explicit terminal fields detached from registration; source APs painted with verified placement and isolated page graphics state. Detached original objects remain; no secure deletion, appearance regeneration or standards assurance.".into() }).collect()))
}

pub(crate) fn registry(
    source: &PdfDocument,
    context: &OperationContext,
) -> Result<Registry, PdfError> {
    let catalog = super::root_ref(source.parsed())?;
    let catalog_values = dict(&source.parsed().object(catalog)?.value)?;
    let acro = catalog_values
        .get(b"AcroForm".as_slice())
        .ok_or_else(|| PdfError::selection("document has no AcroForm"))?;
    let (acro_owner, acro_direct, acro) = match acro {
        Value::Ref(reference) => {
            let object = source.parsed().object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported("flatten AcroForm cannot be a stream"));
            }
            (*reference, false, dict(&object.value)?)
        }
        Value::Dict(values) => (catalog, true, values),
        _ => {
            return Err(PdfError::unsupported(
                "flatten AcroForm must be a direct or singly indirect dictionary",
            ));
        }
    };
    for key in [b"XFA".as_slice(), b"AA"] {
        if present(acro, key).is_some() {
            return Err(PdfError::unsupported(
                "flatten does not reconcile XFA or form actions",
            ));
        }
    }
    if let Some(value) = present(acro, b"NeedAppearances")
        && resolve(source, value, context)? != &Value::Bool(false)
    {
        return Err(PdfError::unsupported(
            "flatten requires current existing appearances",
        ));
    }
    if let Some(value) = present(acro, b"CO")
        && !array(source, value, context)?.is_empty()
    {
        return Err(PdfError::unsupported(
            "flatten requires empty calculation order",
        ));
    }
    // A script anywhere can refer to a selected field by name rather than by
    // object edge. Such interaction cannot be preserved by registration edits.
    for object in source.parsed().objects.values() {
        refuse_actions(&object.value, source, 0, context)?;
    }
    let mut registry = Registry {
        catalog,
        acro_owner,
        acro_direct,
        roots: Vec::new(),
        nodes: BTreeMap::new(),
        names: BTreeMap::new(),
        incoming: BTreeMap::new(),
        pages: BTreeMap::new(),
        containers: BTreeSet::new(),
        reachable: BTreeSet::new(),
        reservations: Vec::new(),
    };
    let mut roots = Vec::new();
    collect_refs(&source.parsed().trailer, &mut roots, source, 0, context)?;
    for root in roots {
        collect_closure(source, root, &mut registry.reachable, context)?;
    }
    if !acro_direct {
        add_edge(
            &mut registry,
            acro_owner,
            Edge::key(catalog, b"AcroForm"),
            context,
        )?;
    }
    let mut edge = if acro_direct {
        let mut edge = Edge::key(catalog, b"AcroForm");
        edge.path.push(Step::Key(b"Fields".to_vec()));
        edge
    } else {
        Edge::key(acro_owner, b"Fields")
    };
    let roots = owned_array(
        source,
        acro.get(b"Fields".as_slice())
            .ok_or_else(|| PdfError::selection("AcroForm has no Fields"))?,
        &mut edge,
        &mut registry,
        context,
    )?;
    for (index, value) in roots.iter().enumerate() {
        let Value::Ref(reference) = value else {
            return Err(PdfError::unsupported(
                "flatten requires indirect registered fields",
            ));
        };
        registry.roots.push(*reference);
        walk(
            source,
            *reference,
            None,
            "",
            edge.child(index),
            0,
            &mut registry,
            context,
        )?;
    }
    for page in source.page_refs()? {
        let values = dict(&source.parsed().object(page)?.value)?;
        let Some(annots) = present(values, b"Annots") else {
            continue;
        };
        let mut edge = Edge::key(page, b"Annots");
        // Annots arrays are cloned owner-locally; shared aliases are still
        // checked when they participate in a selected widget's registration.
        let annots = owned_array(source, annots, &mut edge, &mut registry, context)?;
        for (index, annotation) in annots.iter().enumerate() {
            context.charge_work(1).map_err(PdfError::from_context)?;
            if let Value::Ref(reference) = annotation
                && registry
                    .nodes
                    .get(reference)
                    .is_some_and(|node| node.widget)
            {
                add_edge(&mut registry, *reference, edge.child(index), context)?;
                reserve(
                    &mut registry.reservations,
                    std::mem::size_of::<ObjectRef>(),
                    context,
                )?;
                registry.pages.entry(*reference).or_default().push(page);
            }
        }
    }
    Ok(registry)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    source: &PdfDocument,
    reference: ObjectRef,
    parent: Option<ObjectRef>,
    prefix: &str,
    edge: Edge,
    depth: usize,
    registry: &mut Registry,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > source.parsed().limits.max_parser_depth
        || registry.nodes.len() >= source.parsed().limits.max_container_items
    {
        return Err(PdfError::limit("flatten field tree exceeds limits"));
    }
    if registry.nodes.contains_key(&reference) {
        return Err(PdfError::unsafe_rewrite(
            "flatten field tree contains a cycle or repeated registration",
        ));
    }
    let object = source.parsed().object(reference)?;
    if object.stream.is_some() {
        return Err(PdfError::unsupported(
            "flatten field node cannot be a stream",
        ));
    }
    let values = dict(&object.value)?;
    if matches!(present(values,b"Type"), Some(Value::Name(kind)) if matches!(kind.as_slice(),b"Catalog"|b"Page"|b"Pages"))
    {
        return Err(PdfError::unsafe_rewrite(
            "flatten field aliases a document role",
        ));
    }
    if match parent {
        Some(parent) => present(values, b"Parent") != Some(&Value::Ref(parent)),
        None => present(values, b"Parent").is_some(),
    } {
        return Err(PdfError::unsafe_rewrite(
            "flatten field Parent contradicts registration",
        ));
    }
    add_edge(registry, reference, edge, context)?;
    if let Some(parent) = parent {
        add_edge(registry, parent, Edge::key(reference, b"Parent"), context)?;
    }
    let widget = matches!(present(values,b"Subtype"),Some(Value::Name(kind)) if kind == b"Widget");
    let local = present(values, b"T")
        .map(|value| field_name(source, value, context))
        .transpose()?;
    let name = match local {
        Some(local) if prefix.is_empty() => local,
        Some(local) => format!("{prefix}.{local}"),
        None => prefix.to_owned(),
    };
    if name.len() > source.parsed().limits.max_token_bytes {
        return Err(PdfError::limit("flatten qualified name exceeds limit"));
    }
    reserve(
        &mut registry.reservations,
        name.len() * 2 + std::mem::size_of::<FieldNode>(),
        context,
    )?;
    if present(values, b"T").is_some() && registry.names.insert(name.clone(), reference).is_some() {
        return Err(PdfError::unsafe_rewrite(
            "flatten requires unique fully qualified field names",
        ));
    }
    let mut children = Vec::new();
    let mut child_edge = Edge::key(reference, b"Kids");
    if let Some(kids) = present(values, b"Kids") {
        if widget {
            return Err(PdfError::unsupported(
                "combined widget field cannot have Kids",
            ));
        }
        let kids = owned_array(source, kids, &mut child_edge, registry, context)?;
        for child in kids {
            let Value::Ref(reference) = child else {
                return Err(PdfError::unsupported("flatten field Kids must be indirect"));
            };
            children.push(*reference);
        }
    }
    registry.nodes.insert(
        reference,
        FieldNode {
            parent,
            children: children.clone(),
            widget,
        },
    );
    for (index, child) in children.iter().enumerate() {
        walk(
            source,
            *child,
            Some(reference),
            &name,
            child_edge.child(index),
            depth + 1,
            registry,
            context,
        )?;
    }
    Ok(())
}

fn select(
    source: &PdfDocument,
    registry: &Registry,
    requests: &[PdfFieldFlattenRequest],
    context: &OperationContext,
) -> Result<Vec<PdfFieldFlattenEffect>, PdfError> {
    let mut effects = Vec::new();
    let mut seen = BTreeSet::new();
    for request in requests {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if request.match_index != 0
            || request.field_name.is_empty()
            || request.field_name.len() > source.parsed().limits.max_token_bytes
        {
            return Err(PdfError::selection(
                "flatten requires one unique fully qualified field name",
            ));
        }
        let reference = *registry
            .names
            .get(&request.field_name)
            .ok_or_else(|| PdfError::selection("flatten field name is not registered"))?;
        if !seen.insert(reference) {
            return Err(PdfError::unsafe_rewrite("flatten requests overlap"));
        }
        let node = &registry.nodes[&reference];
        let widgets = if node.widget {
            vec![reference]
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
                                    .expect("registered child")
                                    .value,
                            )
                            .expect("registered dictionary"),
                            b"T",
                        )
                        .is_some()
                })
            {
                return Err(PdfError::unsupported(
                    "flatten selects terminal fields with all their widgets",
                ));
            }
            node.children.clone()
        };
        let kind = inherited(source, registry, reference, b"FT", context)?
            .ok_or_else(|| PdfError::unsupported("flatten field has no proven type"))?;
        if !matches!(kind,Value::Name(kind) if matches!(kind.as_slice(),b"Tx"|b"Btn"|b"Ch")) {
            return Err(PdfError::unsupported(
                "flatten supports terminal Tx/Btn/Ch fields; signature fields are excluded",
            ));
        }
        let mut current = Some(reference);
        while let Some(reference) = current {
            context.charge_work(1).map_err(PdfError::from_context)?;
            selected_policy(dict(&source.parsed().object(reference)?.value)?)?;
            current = registry.nodes[&reference].parent;
        }
        let mut widget_effects = Vec::new();
        for widget in widgets {
            let values = dict(&source.parsed().object(widget)?.value)?;
            if let Some(value) = present(values, b"BM") {
                let value = resolve(source, value, context)?;
                if !matches!(value, Value::Null)
                    && !matches!(value, Value::Name(mode) if mode == b"Normal")
                {
                    return Err(PdfError::unsupported(
                        "flatten requires the annotation's Normal blend mode",
                    ));
                }
            }
            if widget != reference
                && [b"FT".as_slice(), b"Ff", b"V", b"DV"]
                    .iter()
                    .any(|key| present(values, key).is_some())
            {
                return Err(PdfError::unsupported(
                    "flatten separate widgets cannot override terminal field values or type",
                ));
            }
            selected_policy(values)?;
            if matches!(kind, Value::Name(kind) if kind == b"Btn") {
                prove_button_state(source, registry, reference, values, context)?;
            }
            let pages = registry.pages.get(&widget).ok_or_else(|| {
                PdfError::unsafe_rewrite("flatten widget has no page registration")
            })?;
            if pages.len() != 1 || present(values, b"P") != Some(&Value::Ref(pages[0])) {
                return Err(PdfError::unsafe_rewrite(
                    "flatten widget page ownership is ambiguous",
                ));
            }
            let flags = match present(values, b"F") {
                Some(Value::Integer(4)) => 4,
                _ => {
                    return Err(PdfError::unsupported(
                        "flatten requires visible printable widgets with flags exactly Print (4)",
                    ));
                }
            };
            let rect = rectangle(resolve(
                source,
                present(values, b"Rect")
                    .ok_or_else(|| PdfError::unsupported("flatten widget has no Rect"))?,
                context,
            )?)?;
            let appearance = appearance(source, values, context)?;
            let ap = dict(&source.parsed().object(appearance)?.value)?;
            let bbox = rectangle(resolve(
                source,
                present(ap, b"BBox")
                    .ok_or_else(|| PdfError::unsupported("flatten appearance has no BBox"))?,
                context,
            )?)?;
            let matrix = appearance_matrix(source, ap, context)?;
            let placement_matrix = placement(rect, bbox, matrix)?;
            validate_opaque_appearance(source, appearance, context)?;
            widget_effects.push(PdfFlattenedWidgetEffect {
                widget: public_ref(widget),
                page: public_ref(pages[0]),
                appearance: public_ref(appearance),
                resource_name: Vec::new(),
                placement_matrix,
                annotation_flags: flags,
            });
        }
        if matches!(kind, Value::Name(kind) if kind == b"Btn") {
            prove_radio_selection(source, registry, reference, &widget_effects, context)?;
        }
        effects.push(PdfFieldFlattenEffect {
            field: public_ref(reference),
            widgets: widget_effects,
        });
    }
    Ok(effects)
}

fn removed_nodes(
    registry: &Registry,
    effects: &[PdfFieldFlattenEffect],
    context: &OperationContext,
) -> Result<BTreeSet<ObjectRef>, PdfError> {
    let mut removed = BTreeSet::new();
    for effect in effects {
        removed.insert(object_ref(effect.field));
        removed.extend(
            effect
                .widgets
                .iter()
                .map(|widget| object_ref(widget.widget)),
        );
    }
    // Only newly empty ancestors of selected fields are pruned. Their source
    // dictionaries and their historical Parent/Kids references remain stored.
    loop {
        let mut changed = false;
        for (reference, node) in &registry.nodes {
            context.charge_work(1).map_err(PdfError::from_context)?;
            if !removed.contains(reference)
                && !node.children.is_empty()
                && node.children.iter().all(|child| removed.contains(child))
            {
                removed.insert(*reference);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(removed)
}

pub(crate) fn registration_changes(
    source: &PdfDocument,
    registry: &Registry,
    removed: &BTreeSet<ObjectRef>,
    context: &OperationContext,
) -> Result<BTreeMap<ObjectRef, Value>, PdfError> {
    let mut changes = BTreeMap::new();
    let acro = if registry.acro_direct {
        dict(
            dict(&source.parsed().object(registry.catalog)?.value)?
                .get(b"AcroForm".as_slice())
                .expect("source AcroForm"),
        )?
    } else {
        dict(&source.parsed().object(registry.acro_owner)?.value)?
    };
    let roots: Vec<Value> = registry
        .roots
        .iter()
        .filter(|reference| !removed.contains(reference))
        .copied()
        .map(Value::Ref)
        .collect();
    if roots.len() != registry.roots.len() {
        let mut acro = acro.clone();
        acro.insert(b"Fields".to_vec(), Value::Array(roots));
        changes.insert(registry.acro_owner, Value::Dict(acro));
    }
    for (reference, node) in &registry.nodes {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if removed.contains(reference) || !node.children.iter().any(|child| removed.contains(child))
        {
            continue;
        }
        let mut values = dict(&source.parsed().object(*reference)?.value)?.clone();
        values.insert(
            b"Kids".to_vec(),
            Value::Array(
                node.children
                    .iter()
                    .filter(|child| !removed.contains(child))
                    .copied()
                    .map(Value::Ref)
                    .collect(),
            ),
        );
        changes.insert(*reference, Value::Dict(values));
    }
    Ok(changes)
}

fn appearance(
    source: &PdfDocument,
    widget: &Dict,
    context: &OperationContext,
) -> Result<ObjectRef, PdfError> {
    let ap = dict(resolve(
        source,
        present(widget, b"AP")
            .ok_or_else(|| PdfError::unsupported("flatten widget has no appearance"))?,
        context,
    )?)?;
    let normal = present(ap, b"N")
        .ok_or_else(|| PdfError::unsupported("flatten widget has no normal appearance"))?;
    let reference = match normal {
        Value::Ref(reference) if source.parsed().object(*reference)?.stream.is_some() => *reference,
        value => {
            let states = dict(resolve(source, value, context)?)?;
            let Some(Value::Name(state)) = present(widget, b"AS") else {
                return Err(PdfError::unsupported(
                    "state appearance requires explicit AS",
                ));
            };
            match states.get(state) {
                Some(Value::Ref(reference)) => *reference,
                _ => {
                    return Err(PdfError::unsupported(
                        "selected normal appearance is missing or not indirect",
                    ));
                }
            }
        }
    };
    let object = source.parsed().object(reference)?;
    if object.stream.is_none()
        || !matches!(present(dict(&object.value)?,b"Subtype"),Some(Value::Name(kind)) if kind==b"Form")
    {
        return Err(PdfError::unsupported(
            "flatten requires a normal Form XObject stream",
        ));
    }
    Ok(reference)
}

pub(crate) fn inherited<'a>(
    source: &'a PdfDocument,
    registry: &Registry,
    mut reference: ObjectRef,
    key: &[u8],
    context: &OperationContext,
) -> Result<Option<&'a Value>, PdfError> {
    for _ in 0..=source.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if let Some(value) = present(dict(&source.parsed().object(reference)?.value)?, key) {
            return Ok(Some(resolve(source, value, context)?));
        }
        match registry.nodes[&reference].parent {
            Some(parent) => reference = parent,
            None => return Ok(None),
        }
    }
    Err(PdfError::limit(
        "flatten inherited field depth exceeds limit",
    ))
}

pub(crate) fn selected_policy(values: &Dict) -> Result<(), PdfError> {
    for key in [
        b"A".as_slice(),
        b"AA",
        b"StructParent",
        b"StructParents",
        b"OC",
        b"MK",
        b"Lock",
        b"SV",
    ] {
        if present(values, key).is_some() {
            return Err(PdfError::unsupported(
                "flatten requires untagged widgets without actions, optional content, appearance characteristics or signing controls",
            ));
        }
    }
    Ok(())
}

fn prove_button_state(
    source: &PdfDocument,
    registry: &Registry,
    field: ObjectRef,
    widget: &Dict,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let flags = match inherited(source, registry, field, b"Ff", context)? {
        None => 0,
        Some(Value::Integer(flags)) if *flags >= 0 && *flags <= u32::MAX as i64 => *flags as u32,
        _ => return Err(PdfError::unsupported("flatten button flags are malformed")),
    };
    if flags & (1 << 16) != 0 {
        if flags & (1 << 15) != 0 {
            return Err(PdfError::unsupported(
                "flatten button cannot be both radio and push button",
            ));
        }
        return Ok(());
    }
    let ap = dict(resolve(
        source,
        present(widget, b"AP").ok_or_else(|| PdfError::unsupported("flatten button has no AP"))?,
        context,
    )?)?;
    let states = dict(resolve(
        source,
        present(ap, b"N")
            .ok_or_else(|| PdfError::unsupported("flatten button has no normal states"))?,
        context,
    )?)?;
    if !states.contains_key(b"Off".as_slice()) {
        return Err(PdfError::unsupported(
            "flatten state button requires an Off appearance",
        ));
    }
    let value = inherited(source, registry, field, b"V", context)?;
    let state = present(widget, b"AS");
    let consistent = match (value, state) {
        (None, Some(Value::Name(state))) => state == b"Off",
        (Some(Value::Name(value)), Some(Value::Name(state))) => {
            value == state || (flags & (1 << 15) != 0 && state == b"Off")
        }
        _ => false,
    };
    if !consistent {
        return Err(PdfError::unsafe_rewrite(
            "flatten button value and current appearance state disagree",
        ));
    }
    Ok(())
}

fn prove_radio_selection(
    source: &PdfDocument,
    registry: &Registry,
    field: ObjectRef,
    widgets: &[PdfFlattenedWidgetEffect],
    context: &OperationContext,
) -> Result<(), PdfError> {
    let Some(Value::Integer(flags)) = inherited(source, registry, field, b"Ff", context)? else {
        return Ok(());
    };
    if flags & (1 << 15) == 0 {
        return Ok(());
    }
    let Some(Value::Name(value)) = inherited(source, registry, field, b"V", context)? else {
        return Ok(());
    };
    if value == b"Off" {
        return Ok(());
    }
    let mut matches = 0usize;
    for widget in widgets {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if matches!(present(dict(&source.parsed().object(object_ref(widget.widget))?.value)?, b"AS"), Some(Value::Name(state)) if state == value)
        {
            matches += 1;
        }
    }
    if matches == 0 || (flags & (1 << 25) == 0 && matches != 1) {
        return Err(PdfError::unsafe_rewrite(
            "flatten radio value must select the expected number of widget appearances",
        ));
    }
    Ok(())
}

pub(crate) fn refuse_actions(
    value: &Value,
    source: &PdfDocument,
    depth: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > source.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("flatten action scan exceeds depth"));
    }
    match value {
        Value::Dict(values) => {
            let discriminator = present(values, b"S")
                .map(|value| resolve(source, value, context))
                .transpose()?;
            if matches!(discriminator,Some(Value::Name(kind)) if matches!(kind.as_slice(),b"JavaScript"|b"SubmitForm"|b"ResetForm"|b"ImportData"|b"Hide"))
                || present(values, b"JS").is_some()
            {
                return Err(PdfError::unsupported(
                    "flatten cannot preserve script or form actions",
                ));
            }
            for value in values.values() {
                refuse_actions(value, source, depth + 1, context)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                refuse_actions(value, source, depth + 1, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn owned_array<'a>(
    source: &'a PdfDocument,
    mut value: &'a Value,
    edge: &mut Edge,
    registry: &mut Registry,
    context: &OperationContext,
) -> Result<&'a [Value], PdfError> {
    for _ in 0..=source.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        match value {
            Value::Ref(reference) => {
                add_edge(registry, *reference, edge.clone(), context)?;
                registry.containers.insert(*reference);
                edge.owner = *reference;
                edge.path.clear();
                let object = source.parsed().object(*reference)?;
                if object.stream.is_some() {
                    return Err(PdfError::unsupported(
                        "flatten registration array cannot be a stream",
                    ));
                }
                value = &object.value;
            }
            Value::Array(values) if values.len() <= source.parsed().limits.max_container_items => {
                return Ok(values);
            }
            _ => {
                return Err(PdfError::unsupported(
                    "flatten registration requires bounded arrays",
                ));
            }
        }
    }
    Err(PdfError::limit(
        "flatten registration indirection exceeds depth",
    ))
}

fn add_edge(
    registry: &mut Registry,
    target: ObjectRef,
    edge: Edge,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let bytes = edge
        .path
        .iter()
        .try_fold(std::mem::size_of::<Edge>(), |total, step| {
            total.checked_add(
                std::mem::size_of::<Step>()
                    + match step {
                        Step::Key(key) => key.len(),
                        Step::Index(_) => 0,
                    },
            )
        })
        .ok_or_else(|| PdfError::limit("flatten edge size overflows"))?;
    reserve(&mut registry.reservations, bytes, context)?;
    if !registry.incoming.entry(target).or_default().insert(edge) {
        return Err(PdfError::unsafe_rewrite(
            "flatten has repeated ownership edges",
        ));
    }
    Ok(())
}

pub(crate) fn check_incoming(
    registry: &Registry,
    index: &PdfReferenceIndex,
    reference: ObjectRef,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let expected = registry
        .incoming
        .get(&reference)
        .ok_or_else(|| PdfError::unsafe_rewrite("flatten object has no proven registration"))?;
    let actual = index.incoming(index.snapshot(), public_ref(reference))?;
    let active_count = actual
        .iter()
        .filter(|record| match record.origin {
            PdfReferenceOrigin::Object(origin) => registry.reachable.contains(&object_ref(origin)),
            PdfReferenceOrigin::Trailer => true,
        })
        .count();
    if active_count != expected.len() {
        return Err(PdfError::unsafe_rewrite(
            "flatten registration has extra or missing incoming edges",
        ));
    }
    for record in actual {
        context
            .charge_work(record.path.len() as u64 + 1)
            .map_err(PdfError::from_context)?;
        let PdfReferenceOrigin::Object(owner) = record.origin else {
            return Err(PdfError::unsafe_rewrite(
                "flatten registration has trailer ownership",
            ));
        };
        // Prior flatten commits deliberately retain detached source objects.
        // Their unreachable historical references cannot re-register a field.
        if !registry.reachable.contains(&object_ref(owner)) {
            continue;
        }
        let edge = Edge {
            owner: object_ref(owner),
            path: record
                .path
                .iter()
                .map(|step| match step {
                    PdfReferencePathStep::Key(key) => Step::Key(key.clone()),
                    PdfReferencePathStep::Index(index) => Step::Index(*index),
                })
                .collect(),
        };
        if record.dangling || !expected.contains(&edge) {
            return Err(PdfError::unsafe_rewrite(
                "flatten registration has opaque incoming ownership",
            ));
        }
    }
    Ok(())
}

fn order_positions(
    source: &PdfDocument,
    page: ObjectRef,
    positions: &mut [(usize, usize)],
    effects: &[PdfFieldFlattenEffect],
    context: &OperationContext,
) -> Result<(), PdfError> {
    let page = dict(&source.parsed().object(page)?.value)?;
    let annots = array(
        source,
        present(page, b"Annots")
            .ok_or_else(|| PdfError::verification("flatten page Annots absent"))?,
        context,
    )?;
    let selected: BTreeSet<_> = positions
        .iter()
        .map(|(field, widget)| object_ref(effects[*field].widgets[*widget].widget))
        .collect();
    let mut indices = BTreeMap::new();
    let mut unselected = false;
    for (index, value) in annots.iter().enumerate() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        match value {
            Value::Ref(reference) if selected.contains(reference) => {
                if unselected {
                    return Err(PdfError::unsupported(
                        "flatten requires selected widgets before all remaining annotations to preserve paint order",
                    ));
                }
                indices.insert(*reference, index);
            }
            _ => unselected = true,
        }
    }
    positions.sort_by_key(|(field, widget)| {
        indices[&object_ref(effects[*field].widgets[*widget].widget)]
    });
    Ok(())
}

fn contents(
    source: &PdfDocument,
    page: ObjectRef,
    context: &OperationContext,
) -> Result<Vec<ObjectRef>, PdfError> {
    let values = dict(&source.parsed().object(page)?.value)?;
    let Some(value) = present(values, b"Contents") else {
        return Ok(Vec::new());
    };
    if let Value::Ref(reference) = value
        && source.parsed().object(*reference)?.stream.is_some()
    {
        return Ok(vec![*reference]);
    }
    let values = array(source, value, context)?;
    let mut references = Vec::new();
    for value in values {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Value::Ref(reference) = value else {
            return Err(PdfError::unsupported(
                "flatten Contents array must contain indirect streams",
            ));
        };
        if source.parsed().object(*reference)?.stream.is_none() {
            return Err(PdfError::unsupported(
                "flatten Contents item is not a stream",
            ));
        }
        references.push(*reference);
    }
    if references
        .len()
        .checked_add(2)
        .is_none_or(|len| len > source.parsed().limits.max_container_items)
    {
        return Err(PdfError::limit(
            "flatten Contents additions exceed container limit",
        ));
    }
    Ok(references)
}

fn resources(
    source: &PdfDocument,
    page: ObjectRef,
    context: &OperationContext,
) -> Result<Dict, PdfError> {
    context
        .charge_work(source.parsed().limits.max_parser_depth as u64 + 1)
        .map_err(PdfError::from_context)?;
    source
        .inherited_resources(page)?
        .map(|value| dict(resolve(source, value, context)?).cloned())
        .transpose()
        .map(|value| value.unwrap_or_default())
}

fn unused_resource_name(
    xobjects: &Dict,
    source: &PdfDocument,
    context: &OperationContext,
) -> Result<Vec<u8>, PdfError> {
    if xobjects.len() >= source.parsed().limits.max_container_items {
        return Err(PdfError::limit("flatten XObject count exceeds limit"));
    }
    for suffix in 0..=xobjects.len() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let name = format!("BinasFlatten{suffix}").into_bytes();
        if name.len() > source.parsed().limits.max_token_bytes {
            return Err(PdfError::limit("flatten resource name exceeds token limit"));
        }
        if !xobjects.contains_key(&name) {
            return Ok(name);
        }
    }
    unreachable!("one unused name in n+1 candidates")
}

#[derive(Default)]
struct Balance {
    graphics: usize,
    text: bool,
    marked: usize,
    compatibility: usize,
    font_selected: bool,
    saved_fonts: Vec<bool>,
}
fn operations(
    source: &PdfDocument,
    references: &[ObjectRef],
    context: &OperationContext,
    opaque_appearance: bool,
) -> Result<(), PdfError> {
    let document = source.with_cancellation(context.cancellation().clone());
    let mut budget = ParseBudget::for_document(document.parsed());
    let mut state = OperationState::new();
    let mut balance = Balance::default();
    for (part, reference) in references.iter().enumerate() {
        context.checkpoint().map_err(PdfError::from_context)?;
        let object = source.parsed().object(*reference)?;
        let encoded = object
            .stream
            .as_ref()
            .ok_or_else(|| PdfError::unsupported("flatten content is not a stream"))?;
        let decoded = decode_stream(&object.value, encoded, &source.parsed().limits, &mut budget)?;
        context
            .charge_work(decoded.len() as u64 + 1)
            .map_err(PdfError::from_context)?;
        // ISO32000-1 forbids a lexical token across Contents boundaries; PDF2
        // additionally defines separating whitespace. Require an explicit EOL
        // on nonfinal nonempty parts, making both interpretations equivalent
        // including a trailing comment, while operands may span streams.
        if part + 1 < references.len()
            && !decoded.is_empty()
            && !matches!(decoded.last(), Some(b'\r' | b'\n'))
        {
            return Err(PdfError::unsupported(
                "flatten requires an explicit line ending between Contents parts",
            ));
        }
        let parsed = read_operations_part(
            &decoded,
            part,
            &mut state,
            &source.parsed().limits,
            &mut budget,
        )?;
        for operation in parsed {
            context.charge_work(1).map_err(PdfError::from_context)?;
            let op = operation.operator.as_slice();
            if opaque_appearance
                && op == b"BMC"
                && !matches!(operation.operands.as_slice(), [crate::content::operations::ContentOperand::Name(name)] if name == b"Tx")
            {
                return Err(PdfError::unsupported(
                    "retained appearance permits only the unstructured Tx marked-content wrapper",
                ));
            }
            if opaque_appearance && op == b"Tf" {
                prove_font_resource(source, &object.value, &operation.operands, context)?;
                balance.font_selected = true;
            }
            if opaque_appearance
                && matches!(op, b"Tj" | b"TJ" | b"'" | b"\"")
                && (!balance.text || !balance.font_selected)
            {
                return Err(PdfError::unsupported(
                    "flatten appearance text requires an active text object and explicitly selected font",
                ));
            }
            if opaque_appearance
                && matches!(
                    op,
                    b"gs" | b"sh" | b"Do" | b"BI" | b"SCN" | b"scn" | b"CS" | b"cs" | b"BDC"
                )
            {
                return Err(PdfError::unsupported(
                    "flatten appearance requires opaque direct text/path paint without transparency, external graphics or optional content",
                ));
            }
            match op {
                b"q" => {
                    empty_operands(&operation.operands)?;
                    if opaque_appearance {
                        balance.saved_fonts.push(balance.font_selected);
                    }
                    balance.graphics = balance
                        .graphics
                        .checked_add(1)
                        .ok_or_else(|| PdfError::limit("flatten graphics depth overflows"))?;
                }
                b"Q" => {
                    empty_operands(&operation.operands)?;
                    balance.graphics = balance.graphics.checked_sub(1).ok_or_else(|| {
                        PdfError::unsafe_rewrite("flatten source graphics stack underflows")
                    })?;
                    if opaque_appearance {
                        balance.font_selected = balance.saved_fonts.pop().ok_or_else(|| {
                            PdfError::verification(
                                "flatten font stack diverged from graphics stack",
                            )
                        })?;
                    }
                }
                b"BT" => {
                    empty_operands(&operation.operands)?;
                    if balance.text {
                        return Err(PdfError::unsafe_rewrite("flatten source has nested BT"));
                    }
                    balance.text = true;
                }
                b"ET" => {
                    empty_operands(&operation.operands)?;
                    if !balance.text {
                        return Err(PdfError::unsafe_rewrite("flatten source ET has no BT"));
                    }
                    balance.text = false;
                }
                b"BMC" | b"BDC" => {
                    balance.marked = balance
                        .marked
                        .checked_add(1)
                        .ok_or_else(|| PdfError::limit("flatten marked-content depth overflows"))?
                }
                b"EMC" => {
                    empty_operands(&operation.operands)?;
                    balance.marked = balance.marked.checked_sub(1).ok_or_else(|| {
                        PdfError::unsafe_rewrite("flatten marked-content stack underflows")
                    })?;
                }
                b"BX" => {
                    empty_operands(&operation.operands)?;
                    balance.compatibility = balance
                        .compatibility
                        .checked_add(1)
                        .ok_or_else(|| PdfError::limit("flatten compatibility depth overflows"))?;
                }
                b"EX" => {
                    empty_operands(&operation.operands)?;
                    balance.compatibility =
                        balance.compatibility.checked_sub(1).ok_or_else(|| {
                            PdfError::unsafe_rewrite("flatten compatibility stack underflows")
                        })?;
                }
                b"w" | b"J" | b"j" | b"M" | b"d" | b"ri" | b"i" | b"gs" | b"cm" | b"m" | b"l"
                | b"c" | b"v" | b"y" | b"h" | b"re" | b"S" | b"s" | b"f" | b"F" | b"f*" | b"B"
                | b"B*" | b"b" | b"b*" | b"n" | b"W" | b"W*" | b"CS" | b"cs" | b"SC" | b"SCN"
                | b"sc" | b"scn" | b"G" | b"g" | b"RG" | b"rg" | b"K" | b"k" | b"sh" | b"Do"
                | b"BI" | b"Tc" | b"Tw" | b"Tz" | b"TL" | b"Tf" | b"Tr" | b"Ts" | b"Td" | b"TD"
                | b"Tm" | b"T*" | b"Tj" | b"TJ" | b"'" | b"\"" | b"MP" | b"DP" => {}
                _ => {
                    return Err(PdfError::unsupported(
                        "flatten cannot prove an unknown content operator's graphics effects",
                    ));
                }
            }
            if [balance.graphics, balance.marked, balance.compatibility]
                .into_iter()
                .any(|depth| depth > source.parsed().limits.max_parser_depth)
            {
                return Err(PdfError::limit("flatten content state exceeds depth limit"));
            }
        }
    }
    state.finish()?;
    if balance.graphics != 0 || balance.text || balance.marked != 0 || balance.compatibility != 0 {
        return Err(PdfError::unsafe_rewrite(
            "flatten requires balanced source graphics, text, marked-content and compatibility scopes",
        ));
    }
    Ok(())
}
fn empty_operands(operands: &[crate::content::operations::ContentOperand]) -> Result<(), PdfError> {
    if operands.is_empty() {
        Ok(())
    } else {
        Err(PdfError::unsafe_rewrite(
            "flatten balance operator has unexpected operands",
        ))
    }
}
fn prove_font_resource(
    source: &PdfDocument,
    appearance: &Value,
    operands: &[crate::content::operations::ContentOperand],
    context: &OperationContext,
) -> Result<(), PdfError> {
    use crate::content::operations::ContentOperand;
    let [ContentOperand::Name(name), ContentOperand::Number(size)] = operands else {
        return Err(PdfError::unsupported(
            "flatten appearance Tf requires an explicit font name and size",
        ));
    };
    if !size.is_finite() {
        return Err(PdfError::unsupported(
            "flatten appearance font size is nonfinite",
        ));
    }
    let resources = dict(resolve(
        source,
        present(dict(appearance)?, b"Resources")
            .ok_or_else(|| PdfError::unsupported("flatten appearance has no explicit Resources"))?,
        context,
    )?)?;
    let fonts = dict(resolve(
        source,
        present(resources, b"Font").ok_or_else(|| {
            PdfError::unsupported("flatten appearance has no explicit Font resources")
        })?,
        context,
    )?)?;
    let font = dict(resolve(
        source,
        fonts.get(name).ok_or_else(|| {
            PdfError::unsupported("flatten appearance Tf names a missing font resource")
        })?,
        context,
    )?)?;
    if !matches!(present(font, b"Subtype"), Some(Value::Name(kind)) if matches!(kind.as_slice(), b"Type0" | b"Type1" | b"MMType1" | b"TrueType"))
    {
        return Err(PdfError::unsupported(
            "flatten appearance font type is not proven for ordinary text paint",
        ));
    }
    Ok(())
}

fn validate_content_balance(
    source: &PdfDocument,
    references: &[ObjectRef],
    context: &OperationContext,
) -> Result<(), PdfError> {
    operations(source, references, context, false)
}

pub(super) fn validate_opaque_appearance(
    source: &PdfDocument,
    appearance: ObjectRef,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let mut closure = BTreeSet::new();
    collect_closure(source, appearance, &mut closure, context)?;
    for reference in closure {
        let object = source.parsed().object(reference)?;
        refuse_transparency(&object.value, source, 0, context)?;
    }
    let values = dict(&source.parsed().object(appearance)?.value)?;
    // Annotation AP defaults differ from ordinary Forms for transparency.
    // This slice proves opaque painting; it does not infer equivalent grouping.
    if present(values, b"Resources").is_none() {
        return Err(PdfError::unsupported(
            "flatten appearance requires explicit Resources",
        ));
    }
    operations(source, &[appearance], context, true)
}
fn refuse_transparency(
    value: &Value,
    source: &PdfDocument,
    depth: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > source.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("flatten appearance scan exceeds depth"));
    }
    match value {
        Value::Dict(values) => {
            for key in [
                b"ExtGState".as_slice(),
                b"Group",
                b"SMask",
                b"Mask",
                b"OC",
                b"StructParent",
                b"StructParents",
                b"Ref",
                b"OPI",
            ] {
                if present(values, key).is_some() {
                    return Err(PdfError::unsupported(
                        "flatten appearance closure has transparency, tags, optional content or external rendering semantics",
                    ));
                }
            }
            if matches!(present(values,b"Subtype"),Some(Value::Name(kind)) if kind==b"Type3") {
                return Err(PdfError::unsupported(
                    "flatten does not prove Type3 glyph graphics",
                ));
            }
            if matches!(present(values,b"Type"),Some(Value::Name(kind)) if matches!(kind.as_slice(),b"Page"|b"Pages"|b"Catalog"))
                || present(values, b"FT").is_some()
            {
                return Err(PdfError::unsupported(
                    "flatten appearance closure aliases page or field roles",
                ));
            }
            for value in values.values() {
                refuse_transparency(value, source, depth + 1, context)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                refuse_transparency(value, source, depth + 1, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn collect_closure(
    source: &PdfDocument,
    start: ObjectRef,
    seen: &mut BTreeSet<ObjectRef>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let mut pending = vec![start];
    while let Some(reference) = pending.pop() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if !seen.insert(reference) {
            continue;
        }
        if seen.len() > source.parsed().limits.max_objects {
            return Err(PdfError::limit(
                "flatten appearance closure exceeds object limit",
            ));
        }
        collect_refs(
            &source.parsed().object(reference)?.value,
            &mut pending,
            source,
            0,
            context,
        )?;
    }
    Ok(())
}
fn collect_refs(
    value: &Value,
    pending: &mut Vec<ObjectRef>,
    source: &PdfDocument,
    depth: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > source.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("flatten closure value depth exceeds limit"));
    }
    match value {
        Value::Ref(reference) => pending.push(*reference),
        Value::Array(values) => {
            for value in values {
                collect_refs(value, pending, source, depth + 1, context)?;
            }
        }
        Value::Dict(values) => {
            for value in values.values() {
                collect_refs(value, pending, source, depth + 1, context)?;
            }
        }
        _ => {}
    }
    if pending.len() > source.parsed().limits.max_container_items {
        return Err(PdfError::limit("flatten closure frontier exceeds limit"));
    }
    Ok(())
}

pub(crate) fn rectangle(value: &Value) -> Result<[f64; 4], PdfError> {
    let values = numbers::<4>(value)?;
    if values[2] <= values[0]
        || values[3] <= values[1]
        || !(values[2] - values[0]).is_finite()
        || !(values[3] - values[1]).is_finite()
    {
        return Err(PdfError::unsupported(
            "flatten rectangle must have finite positive extents",
        ));
    }
    Ok(values)
}
fn numbers<const N: usize>(value: &Value) -> Result<[f64; N], PdfError> {
    let Value::Array(values) = value else {
        return Err(PdfError::unsupported(
            "flatten geometry must be a numeric array",
        ));
    };
    if values.len() != N {
        return Err(PdfError::unsupported(
            "flatten geometry array has wrong length",
        ));
    }
    let mut numbers = [0.; N];
    for (output, value) in numbers.iter_mut().zip(values) {
        *output = match value {
            Value::Integer(value) => *value as f64,
            Value::Real(value) => *value,
            _ => {
                return Err(PdfError::unsupported(
                    "flatten geometry contains non-number",
                ));
            }
        };
    }
    if numbers.iter().any(|value| !value.is_finite()) {
        return Err(PdfError::unsupported(
            "flatten geometry contains nonfinite number",
        ));
    }
    Ok(numbers)
}
pub(crate) fn appearance_matrix(
    source: &PdfDocument,
    values: &Dict,
    context: &OperationContext,
) -> Result<[f64; 6], PdfError> {
    present(values, b"Matrix")
        .map(|value| numbers::<6>(resolve(source, value, context)?))
        .transpose()
        .map(|value| value.unwrap_or([1., 0., 0., 1., 0., 0.]))
}
pub(crate) fn placement(
    rect: [f64; 4],
    bbox: [f64; 4],
    matrix: [f64; 6],
) -> Result<[f64; 6], PdfError> {
    let determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if !determinant.is_finite()
        || determinant == 0.
        || matrix.iter().any(|value| !value.is_finite())
    {
        return Err(PdfError::unsupported(
            "flatten appearance Matrix is singular or nonfinite",
        ));
    }
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for x in [bbox[0], bbox[2]] {
        for y in [bbox[1], bbox[3]] {
            let x2 = matrix[0] * x + matrix[2] * y + matrix[4];
            let y2 = matrix[1] * x + matrix[3] * y + matrix[5];
            if !x2.is_finite() || !y2.is_finite() {
                return Err(PdfError::unsupported(
                    "flatten transformed appearance bounds overflow",
                ));
            }
            bounds[0] = bounds[0].min(x2);
            bounds[1] = bounds[1].min(y2);
            bounds[2] = bounds[2].max(x2);
            bounds[3] = bounds[3].max(y2);
        }
    }
    let sx = (rect[2] - rect[0]) / (bounds[2] - bounds[0]);
    let sy = (rect[3] - rect[1]) / (bounds[3] - bounds[1]);
    let placement = [
        sx,
        0.,
        0.,
        sy,
        rect[0] - sx * bounds[0],
        rect[1] - sy * bounds[1],
    ];
    if placement.iter().any(|value| !value.is_finite())
        || sx <= 0.
        || sy <= 0.
        || !((bounds[2] - bounds[0]) * (bounds[3] - bounds[1])).is_finite()
    {
        return Err(PdfError::unsupported(
            "flatten placement is nonfinite or degenerate",
        ));
    }
    Ok(placement)
}

fn paint_program<'a>(
    effects: impl Iterator<Item = &'a PdfFlattenedWidgetEffect>,
    source: &PdfDocument,
    context: &OperationContext,
) -> Result<Vec<u8>, PdfError> {
    let mut program = String::from("\nn\nQ\n");
    for effect in effects {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let numbers: Vec<String> = effect
            .placement_matrix
            .iter()
            .map(|value| value.to_string())
            .collect();
        program.push_str("q\n");
        program.push_str(&numbers.join(" "));
        program.push_str(" cm\n/");
        program.push_str(
            std::str::from_utf8(&effect.resource_name).map_err(|_| {
                PdfError::verification("flatten generated resource name is not ASCII")
            })?,
        );
        program.push_str(" Do\nQ\n");
        if program.len() > source.parsed().limits.max_stream_bytes {
            return Err(PdfError::limit("flatten paint stream exceeds limit"));
        }
    }
    Ok(program.into_bytes())
}

pub(crate) fn next_number(
    source: &PdfDocument,
    staged: &BTreeMap<ObjectRef, IndirectObject>,
) -> Result<u32, PdfError> {
    let highest = source
        .parsed()
        .objects
        .keys()
        .chain(staged.keys())
        .map(|reference| reference.number)
        .max()
        .unwrap_or(0);
    let after = highest
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("flatten object number overflows"))?;
    let size = u32::try_from(source.parsed().source_size.unwrap_or(0))
        .map_err(|_| PdfError::limit("flatten source Size exceeds object numbering"))?;
    Ok(after.max(size))
}
pub(crate) fn allocate(
    next: &mut u32,
    source: &PdfDocument,
    staged: &BTreeMap<ObjectRef, IndirectObject>,
    extra: usize,
) -> Result<ObjectRef, PdfError> {
    let staged_new = staged
        .keys()
        .filter(|reference| !source.parsed().objects.contains_key(reference))
        .count();
    if source
        .parsed()
        .objects
        .len()
        .checked_add(staged_new)
        .and_then(|count| count.checked_add(extra))
        .is_none_or(|count| count > source.parsed().limits.max_objects)
        || u64::from(*next) >= source.parsed().limits.max_xref_entries as u64
    {
        return Err(PdfError::limit("flatten object allocation exceeds limits"));
    }
    let reference = ObjectRef {
        number: *next,
        generation: 0,
    };
    *next = next
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("flatten object numbering overflows"))?;
    Ok(reference)
}
fn stream_object(bytes: Vec<u8>, context: &OperationContext) -> Result<IndirectObject, PdfError> {
    let reservation = context
        .reserve(bytes.len() as u64)
        .map_err(PdfError::from_context)?;
    Ok(IndirectObject {
        value: Value::Dict(BTreeMap::from([(
            b"Length".to_vec(),
            Value::Integer(bytes.len() as i64),
        )])),
        stream: Some(StreamPayload::from_reserved(bytes, vec![reservation])?),
        stream_offset: 0,
        offset: 0,
    })
}
fn exact_stream(
    document: &PdfDocument,
    reference: ObjectRef,
    bytes: &[u8],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context
        .charge_work(bytes.len() as u64 + 1)
        .map_err(PdfError::from_context)?;
    let object = document.parsed().object(reference)?;
    Ok(object.stream.as_deref() == Some(bytes)
        && object.value
            == Value::Dict(BTreeMap::from([(
                b"Length".to_vec(),
                Value::Integer(bytes.len() as i64),
            )])))
}
fn composed_object(
    source: &PdfDocument,
    staged: &BTreeMap<ObjectRef, IndirectObject>,
    reference: ObjectRef,
    reservations: &mut Vec<Reservation>,
    context: &OperationContext,
) -> Result<IndirectObject, PdfError> {
    let object = staged
        .get(&reference)
        .unwrap_or(source.parsed().object(reference)?);
    if object.stream.is_some() {
        return Err(PdfError::unsupported(
            "flatten dictionary role cannot be a stream",
        ));
    }
    reserve_value(&object.value, source, reservations, context)?;
    Ok(object.clone())
}
pub(crate) fn objects_equal(
    before: &IndirectObject,
    after: &IndirectObject,
    source: &PdfDocument,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context
        .charge_work(
            value_cost(&before.value, source, 0, context)? as u64
                + before.stream.as_ref().map_or(0, |stream| stream.len()) as u64,
        )
        .map_err(PdfError::from_context)?;
    Ok(before.value == after.value && before.stream.as_deref() == after.stream.as_deref())
}
fn field_name(
    source: &PdfDocument,
    value: &Value,
    context: &OperationContext,
) -> Result<String, PdfError> {
    let Value::String(bytes) = resolve(source, value, context)? else {
        return Err(PdfError::unsupported(
            "flatten field name must be a text string",
        ));
    };
    context
        .charge_work(bytes.len() as u64)
        .map_err(PdfError::from_context)?;
    let name = if let Some(bytes) = bytes.strip_prefix(&[0xfe, 0xff]) {
        if bytes.len() % 2 != 0 {
            return Err(PdfError::unsupported("flatten name has truncated UTF16"));
        }
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .map_err(|_| PdfError::unsupported("flatten name has invalid UTF16"))?
    } else if bytes.is_ascii() {
        String::from_utf8(bytes.clone()).expect("ASCII")
    } else {
        return Err(PdfError::unsupported(
            "flatten field name encoding is unsupported without loss",
        ));
    };
    if name.is_empty() || name.contains('.') || name.len() > source.parsed().limits.max_token_bytes
    {
        return Err(PdfError::unsupported(
            "flatten partial name is empty, ambiguous or oversized",
        ));
    }
    Ok(name)
}
pub(crate) fn dict(value: &Value) -> Result<&Dict, PdfError> {
    match value {
        Value::Dict(values) => Ok(values),
        _ => Err(PdfError::unsupported("flatten requires a dictionary")),
    }
}
pub(crate) fn present<'a>(values: &'a Dict, key: &[u8]) -> Option<&'a Value> {
    values
        .get(key)
        .filter(|value| !matches!(value, Value::Null))
}
pub(crate) fn resolve<'a>(
    source: &'a PdfDocument,
    mut value: &'a Value,
    context: &OperationContext,
) -> Result<&'a Value, PdfError> {
    for _ in 0..=source.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        match value {
            Value::Ref(reference) => {
                let object = source.parsed().object(*reference)?;
                if object.stream.is_some() {
                    return Err(PdfError::unsupported(
                        "flatten scalar value cannot be a stream",
                    ));
                }
                value = &object.value;
            }
            _ => return Ok(value),
        }
    }
    Err(PdfError::limit("flatten value indirection exceeds depth"))
}
fn array<'a>(
    source: &'a PdfDocument,
    value: &'a Value,
    context: &OperationContext,
) -> Result<&'a [Value], PdfError> {
    match resolve(source, value, context)? {
        Value::Array(values) if values.len() <= source.parsed().limits.max_container_items => {
            Ok(values)
        }
        _ => Err(PdfError::unsupported("flatten requires a bounded array")),
    }
}
pub(crate) fn public_ref(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}
pub(crate) fn object_ref(reference: PdfReferenceTarget) -> ObjectRef {
    ObjectRef {
        number: reference.number,
        generation: reference.generation,
    }
}
fn bound(source: &PdfDocument, plan: &FlattenPlan) -> Result<(), PdfError> {
    if plan.snapshot.source() != source.source_id()
        || !source.parsed().objects.shares_snapshot_with(&plan.store)
    {
        Err(PdfError::selection(
            "flatten plan source/object-store mismatch",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn reserve(
    reservations: &mut Vec<Reservation>,
    bytes: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    reservations.push(
        context
            .reserve(bytes as u64)
            .map_err(PdfError::from_context)?,
    );
    Ok(())
}
pub(crate) fn reserve_value(
    value: &Value,
    source: &PdfDocument,
    reservations: &mut Vec<Reservation>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let bytes = value_cost(value, source, 0, context)?
        .checked_mul(2)
        .ok_or_else(|| PdfError::limit("flatten clone workspace overflows"))?;
    reserve(reservations, bytes, context)
}
pub(crate) fn value_cost(
    value: &Value,
    source: &PdfDocument,
    depth: usize,
    context: &OperationContext,
) -> Result<usize, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > source.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("flatten copied value depth exceeds limit"));
    }
    let mut total = std::mem::size_of::<Value>();
    match value {
        Value::Name(bytes) | Value::String(bytes) => {
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| PdfError::limit("flatten copied scalar size overflows"))?
        }
        Value::Array(values) => {
            if values.len() > source.parsed().limits.max_container_items {
                return Err(PdfError::limit(
                    "flatten copied array exceeds container limit",
                ));
            }
            for value in values {
                total = total
                    .checked_add(value_cost(value, source, depth + 1, context)?)
                    .ok_or_else(|| PdfError::limit("flatten array clone size overflows"))?;
            }
        }
        Value::Dict(values) => {
            if values.len() > source.parsed().limits.max_container_items {
                return Err(PdfError::limit(
                    "flatten copied dictionary exceeds container limit",
                ));
            }
            for (key, value) in values {
                let child = value_cost(value, source, depth + 1, context)?;
                total = total
                    .checked_add(key.len())
                    .and_then(|total| total.checked_add(child))
                    .ok_or_else(|| PdfError::limit("flatten dictionary clone size overflows"))?;
            }
        }
        _ => {}
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget, RevisionId};

    fn fixture() -> PdfDocument {
        PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/targeted-flatten.pdf"),
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
    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn request(name: &str) -> PdfFieldFlattenRequest {
        PdfFieldFlattenRequest {
            field_name: name.into(),
            match_index: 0,
        }
    }
    fn planned(source: &PdfDocument, ctx: &OperationContext) -> Result<FlattenPlan, PdfError> {
        let index = PdfReferenceIndex::build(source, ctx)?;
        plan(
            source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &index,
            &[request("group.answer")],
            &BTreeMap::new(),
            ctx,
        )
    }
    fn staged(source: &PdfDocument, plan: &FlattenPlan, ctx: &OperationContext) -> PdfDocument {
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
        let object = parsed.objects.get_mut(&reference(number)).unwrap();
        let Value::Dict(values) = &mut object.value else {
            panic!("fixture dictionary")
        };
        change(values);
        source.with_parsed(parsed)
    }
    fn rejected(
        source: &PdfDocument,
        candidate: &PdfDocument,
        plan: &FlattenPlan,
        ctx: &OperationContext,
    ) {
        if let Ok(report) = verify(source, candidate, plan, ctx) {
            assert!(report.validate(&required_checks()).is_err());
        }
    }

    #[test]
    fn shared_matrix_appearance_is_owner_local_and_source_objects_survive() {
        let source = fixture();
        let ctx = context();
        let plan = planned(&source, &ctx).unwrap();
        assert_eq!(plan.effects[0].field, public_ref(reference(8)));
        assert_eq!(
            plan.effects[0].widgets[0].placement_matrix,
            [2., 0., 0., 1., 16., 143.]
        );
        assert_eq!(
            plan.effects[0].widgets[1].placement_matrix,
            [1., 0., 0., 2., 58., 116.]
        );
        assert!(
            plan.effects[0]
                .widgets
                .iter()
                .all(|widget| widget.resource_name == b"BinasFlatten1")
        );
        let candidate = staged(&source, &plan, &ctx);
        verify(&source, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        for number in [8, 9, 10, 11, 12, 13, 14, 30] {
            assert!(
                objects_equal(
                    source.parsed().object(reference(number)).unwrap(),
                    candidate.parsed().object(reference(number)).unwrap(),
                    &source,
                    &ctx
                )
                .unwrap()
            );
        }
    }

    #[test]
    fn independent_checks_detect_registration_resource_and_paint_corruption() {
        let source = fixture();
        let ctx = context();
        let plan = planned(&source, &ctx).unwrap();
        let candidate = staged(&source, &plan, &ctx);
        let wrong = changed(&candidate, 7, |values| {
            values.insert(b"Kids".to_vec(), Value::Array(Vec::new()));
        });
        rejected(&source, &wrong, &plan, &ctx);
        let wrong = changed(&candidate, 3, |values| {
            values.insert(b"Resources".to_vec(), Value::Dict(Dict::new()));
        });
        rejected(&source, &wrong, &plan, &ctx);
        let mut parsed = candidate.parsed().clone();
        parsed
            .objects
            .get_mut(&plan.pages[&reference(3)].paint)
            .unwrap()
            .stream = Some(b"Q\n".to_vec().into());
        let wrong = candidate.with_parsed(parsed);
        rejected(&source, &wrong, &plan, &ctx);
    }

    #[test]
    fn arbitrary_plan_objects_cannot_authorize_themselves() {
        let source = fixture();
        let ctx = context();
        let mut plan = planned(&source, &ctx).unwrap();
        let invented = reference(200);
        plan.authorized.insert(invented);
        plan.replacements.insert(
            invented,
            IndirectObject {
                value: Value::Dict(Dict::new()),
                stream: None,
                stream_offset: 0,
                offset: 0,
            },
        );
        let candidate = staged(&source, &plan, &ctx);
        rejected(&source, &candidate, &plan, &ctx);
    }

    #[test]
    fn reachable_opaque_widget_ownership_is_refused() {
        let source = changed(&fixture(), 1, |values| {
            values.insert(b"PrivateWidget".to_vec(), Value::Ref(reference(9)));
        });
        assert!(planned(&source, &context()).is_err());
    }

    #[test]
    fn malformed_matrix_visibility_and_transparency_are_refused() {
        let source = fixture();
        let wrong = changed(&source, 14, |values| {
            values.insert(
                b"Matrix".to_vec(),
                Value::Array([1., 1., 1., 1., 0., 0.].map(Value::Real).to_vec()),
            );
        });
        assert!(planned(&wrong, &context()).is_err());
        for flags in [0, 2, 12, 20, 36, 260] {
            let wrong = changed(&source, 9, |values| {
                values.insert(b"F".to_vec(), Value::Integer(flags));
            });
            assert!(planned(&wrong, &context()).is_err());
        }
        let wrong = changed(&source, 14, |values| {
            values.insert(b"Group".to_vec(), Value::Dict(Dict::new()));
        });
        assert!(planned(&wrong, &context()).is_err());
    }

    #[test]
    fn source_content_balance_and_original_stack_order_are_required() {
        let source = fixture();
        let page = contents(&source, reference(3), &context()).unwrap();
        for bytes in [b"Q\n".as_slice(), b"BT\n", b"/Artifact BMC\n", b"BX\n"] {
            let mut parsed = source.parsed().clone();
            let object = parsed.objects.get_mut(&page[0]).unwrap();
            object.stream = Some(bytes.to_vec().into());
            object.value = Value::Dict(Dict::from([(
                b"Length".to_vec(),
                Value::Integer(bytes.len() as i64),
            )]));
            assert!(planned(&source.with_parsed(parsed), &context()).is_err());
        }
        let wrong = changed(&source, 3, |values| {
            let Value::Array(annots) = values.get_mut(b"Annots".as_slice()).unwrap() else {
                panic!("fixture Annots")
            };
            annots.swap(0, 1);
        });
        assert!(planned(&wrong, &context()).is_err());
    }

    #[test]
    fn source_branch_cancellation_and_live_work_limits_do_not_publish() {
        let source = fixture();
        let ctx = context();
        let plan = planned(&source, &ctx).unwrap();
        let wrong = changed(&source, 8, |values| {
            values.insert(b"Private".to_vec(), Value::Integer(7));
        });
        assert!(apply(&wrong, &plan, &mut BTreeMap::new(), &ctx).is_err());
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert!(planned(&source, &cancelled).is_err());
        for (live, work) in [(1, 64 * 1024 * 1024), (32 * 1024 * 1024, 1)] {
            let ctx = OperationContext::new(
                ResourceBudget::new(BudgetLimits {
                    max_live_bytes: live,
                    max_work_units: work,
                }),
                CancellationToken::new(),
            );
            assert!(planned(&source, &ctx).is_err());
        }
        assert_eq!(source.parsed().objects.changes().len(), 0);
    }

    #[test]
    fn placement_preserves_reflection_and_small_finite_scale() {
        assert_eq!(
            placement(
                [10., 20., 30., 60.],
                [0., 0., 10., 20.],
                [-1., 0., 0., 1., 3., 4.]
            )
            .unwrap(),
            [2., 0., 0., 2., 24., 12.]
        );
        let result = placement(
            [0., 0., 1e-20, 1e-20],
            [0., 0., 1., 1.],
            [1., 0., 0., 1., 0., 0.],
        )
        .unwrap();
        assert_eq!(result[0], 1e-20);
    }

    #[test]
    fn appearance_font_lookup_cannot_fall_back_to_annotation_defaults() {
        let source = fixture();
        let missing = changed(&source, 14, |values| {
            values.insert(b"Resources".to_vec(), Value::Dict(Dict::new()));
        });
        assert!(planned(&missing, &context()).is_err());
        let wrong = changed(&source, 14, |values| {
            values.insert(
                b"Resources".to_vec(),
                Value::Dict(Dict::from([(
                    b"Font".to_vec(),
                    Value::Dict(Dict::from([(b"Other".to_vec(), Value::Ref(reference(19)))])),
                )])),
            );
        });
        assert!(planned(&wrong, &context()).is_err());
        assert!(planned(&source, &context()).is_ok());
    }

    #[test]
    fn radio_selection_requires_a_current_selected_widget_and_honors_unison() {
        let mut source = changed(&fixture(), 8, |values| {
            values.insert(b"FT".to_vec(), Value::Name(b"Btn".to_vec()));
            values.insert(b"Ff".to_vec(), Value::Integer(1 << 15));
            values.insert(b"V".to_vec(), Value::Name(b"Yes".to_vec()));
        });
        for number in [9, 10] {
            source = changed(&source, number, |values| {
                values.insert(b"AS".to_vec(), Value::Name(b"Off".to_vec()));
                values.insert(
                    b"AP".to_vec(),
                    Value::Dict(Dict::from([(
                        b"N".to_vec(),
                        Value::Dict(Dict::from([
                            (b"Off".to_vec(), Value::Ref(reference(14))),
                            (b"Yes".to_vec(), Value::Ref(reference(14))),
                        ])),
                    )])),
                );
            });
        }
        assert!(planned(&source, &context()).is_err());
        source = changed(&source, 9, |values| {
            values.insert(b"AS".to_vec(), Value::Name(b"Yes".to_vec()));
        });
        assert!(planned(&source, &context()).is_ok());
        source = changed(&source, 10, |values| {
            values.insert(b"AS".to_vec(), Value::Name(b"Yes".to_vec()));
        });
        assert!(planned(&source, &context()).is_err());
        source = changed(&source, 8, |values| {
            values.insert(b"Ff".to_vec(), Value::Integer((1 << 15) | (1 << 25)));
        });
        assert!(planned(&source, &context()).is_ok());
    }

    #[test]
    fn indirect_form_actions_outside_selected_fields_are_refused() {
        let source = fixture();
        let mut parsed = source.parsed().clone();
        parsed.objects.insert(
            reference(200),
            IndirectObject {
                value: Value::Name(b"ResetForm".to_vec()),
                stream: None,
                stream_offset: 0,
                offset: 0,
            },
        );
        let source = source.with_parsed(parsed);
        let source = changed(&source, 1, |values| {
            values.insert(
                b"OpenAction".to_vec(),
                Value::Dict(Dict::from([
                    (b"S".to_vec(), Value::Ref(reference(200))),
                    (
                        b"Fields".to_vec(),
                        Value::Array(vec![Value::String(b"group.answer".to_vec())]),
                    ),
                ])),
            );
        });
        assert!(planned(&source, &context()).is_err());
    }

    #[test]
    fn annotation_blend_mode_is_resolved_and_requires_normal_paint() {
        let source = fixture();
        let wrong = changed(&source, 9, |values| {
            values.insert(b"BM".to_vec(), Value::Name(b"Multiply".to_vec()));
        });
        assert!(planned(&wrong, &context()).is_err());
        let mut parsed = source.parsed().clone();
        parsed.objects.insert(
            reference(200),
            IndirectObject {
                value: Value::Name(b"Multiply".to_vec()),
                stream: None,
                stream_offset: 0,
                offset: 0,
            },
        );
        let indirect = changed(&source.with_parsed(parsed), 9, |values| {
            values.insert(b"BM".to_vec(), Value::Ref(reference(200)));
        });
        assert!(planned(&indirect, &context()).is_err());
        for value in [Value::Null, Value::Name(b"Normal".to_vec())] {
            let allowed = changed(&source, 9, |values| {
                values.insert(b"BM".to_vec(), value);
            });
            assert!(planned(&allowed, &context()).is_ok());
        }
    }

    #[test]
    fn appearance_text_font_state_is_explicit_and_restored_by_q() {
        let source = fixture();
        for (bytes, allowed) in [
            (b"BT (Unknown font) Tj ET\n".as_slice(), false),
            (
                b"q /F1 9 Tf Q BT (Restored missing font) Tj ET\n".as_slice(),
                false,
            ),
            (b"/F1 9 Tf q Q BT (Explicit font) Tj ET\n".as_slice(), true),
        ] {
            let mut parsed = source.parsed().clone();
            let object = parsed.objects.get_mut(&reference(14)).unwrap();
            let Value::Dict(values) = &mut object.value else {
                panic!("appearance dictionary")
            };
            values.remove(b"Filter".as_slice());
            values.insert(b"Length".to_vec(), Value::Integer(bytes.len() as i64));
            object.stream = Some(bytes.to_vec().into());
            assert_eq!(
                planned(&source.with_parsed(parsed), &context()).is_ok(),
                allowed
            );
        }
    }

    #[test]
    fn fresh_paint_identity_must_respect_source_allocation_floor() {
        let source = fixture();
        let ctx = context();
        let mut plan = planned(&source, &ctx).unwrap();
        let page = reference(3);
        let original = plan.pages[&page].prefix;
        let invalid = reference(0);
        let prefix = plan.replacements.remove(&original).unwrap();
        plan.replacements.insert(invalid, prefix);
        plan.authorized.remove(&original);
        plan.authorized.insert(invalid);
        plan.pages.get_mut(&page).unwrap().prefix = invalid;
        let Value::Dict(values) = &mut plan.replacements.get_mut(&page).unwrap().value else {
            panic!("page")
        };
        let Value::Array(contents) = values.get_mut(b"Contents".as_slice()).unwrap() else {
            panic!("Contents")
        };
        contents[0] = Value::Ref(invalid);
        let candidate = staged(&source, &plan, &ctx);
        rejected(&source, &candidate, &plan, &ctx);
    }
}
