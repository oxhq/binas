//! Retained checkbox state selection using existing normal appearances.
//! No appearance generation, scripting, field renaming or tag repair occurs.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SnapshotId,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};

use super::CheckboxFieldMutationRequest;
use crate::{
    PdfDocument, PdfError, PdfReference, PdfReferenceIndex, PdfReferenceOrigin,
    PdfReferencePathStep, PdfReferenceTarget,
    object::{IndirectObject, ObjectRef, Value},
    store::ObjectStore,
};

type Dict = BTreeMap<Vec<u8>, Value>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfCheckboxEffect {
    pub field: PdfReferenceTarget,
    pub widget: PdfReferenceTarget,
    pub page: PdfReferenceTarget,
    pub checked: bool,
    /// Effective field value, including a proven inherited value.
    pub before_value: Option<Vec<u8>>,
    pub before_appearance_state: Vec<u8>,
    pub after_state: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct CheckboxPlan {
    snapshot: SnapshotId,
    store: ObjectStore,
    requests: Vec<CheckboxFieldMutationRequest>,
    effects: Vec<PdfCheckboxEffect>,
    replacements: BTreeMap<ObjectRef, IndirectObject>,
    protected: BTreeSet<ObjectRef>,
    _reservations: Vec<Reservation>,
}

impl CheckboxPlan {
    pub(crate) fn effects(&self) -> &[PdfCheckboxEffect] {
        &self.effects
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Step {
    Key(Vec<u8>),
    Index(usize),
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Edge {
    origin: ObjectRef,
    path: Vec<Step>,
}

impl Edge {
    fn key(origin: ObjectRef, key: &[u8]) -> Self {
        Self {
            origin,
            path: vec![Step::Key(key.to_vec())],
        }
    }
    fn child(&self, index: usize) -> Self {
        let mut result = self.clone();
        result.path.push(Step::Index(index));
        result
    }
}

#[derive(Debug)]
struct Node {
    parent: Option<ObjectRef>,
    children: Vec<ObjectRef>,
    widget: bool,
}

#[derive(Debug, Default)]
struct Tree {
    names: BTreeMap<String, ObjectRef>,
    nodes: BTreeMap<ObjectRef, Node>,
    incoming: BTreeMap<ObjectRef, BTreeSet<Edge>>,
    pages: BTreeMap<ObjectRef, Vec<ObjectRef>>,
    protected: BTreeSet<ObjectRef>,
    reservations: Vec<Reservation>,
}

pub(crate) fn plan(
    source: &PdfDocument,
    snapshot: SnapshotId,
    index: &PdfReferenceIndex,
    requests: &[CheckboxFieldMutationRequest],
    context: &OperationContext,
) -> Result<CheckboxPlan, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if snapshot.source() != source.source_id() || snapshot != index.snapshot() {
        return Err(PdfError::selection(
            "checkbox index source/revision mismatch",
        ));
    }
    if requests.is_empty() || requests.len() > source.parsed().limits.max_container_items {
        return Err(PdfError::limit(
            "checkbox request count is empty or exceeds limit",
        ));
    }
    for request in requests {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if request.field_name.is_empty()
            || request.field_name.len() > source.parsed().limits.max_token_bytes
        {
            return Err(PdfError::limit(
                "checkbox field name is empty or exceeds token limit",
            ));
        }
        if request.match_index != 0 {
            return Err(PdfError::selection(
                "retained checkbox requires one unique field name",
            ));
        }
    }
    crate::writer::refuse_security_boundaries_with_context(source.parsed(), Some(context))?;
    crate::preservation::pages::validate_tree(source, context)?;
    // The index currently lacks store-branch identity. Compare a fresh bounded
    // census rather than trusting an index from the same source/revision branch.
    let fresh = PdfReferenceIndex::build(source, context)?;
    context
        .charge_work(fresh.records().len() as u64)
        .map_err(PdfError::from_context)?;
    if fresh.records() != index.records() {
        return Err(PdfError::selection(
            "checkbox index describes a different object-store branch",
        ));
    }
    let mut tree = tree(source, context)?;
    let mut effects = Vec::new();
    let mut seen = BTreeSet::new();
    for request in requests {
        let effect = prove(source, &mut tree, &fresh, request, context)?;
        if !seen.insert(object_ref(effect.field)) {
            return Err(PdfError::unsafe_rewrite(
                "checkbox requests overlap on one field",
            ));
        }
        reserve(&mut tree.reservations, effect_cost(&effect)?, context)?;
        effects.push(effect);
    }
    let mut replacements = BTreeMap::new();
    for effect in &effects {
        for (reference, key) in [
            (object_ref(effect.field), b"V".as_slice()),
            (object_ref(effect.widget), b"AS".as_slice()),
        ] {
            if let std::collections::btree_map::Entry::Vacant(entry) = replacements.entry(reference)
            {
                let original = source.parsed().object(reference)?;
                let cost = value_cost(&original.value, 0, source, context)?
                    .checked_add(std::mem::size_of::<IndirectObject>())
                    .and_then(|cost| cost.checked_mul(2))
                    .ok_or_else(|| PdfError::limit("checkbox clone workspace overflows"))?;
                // Prepay the plan dictionary and its one staged clone. This is
                // plan-lifetime logical accounting, not session/process memory.
                reserve(&mut tree.reservations, cost, context)?;
                entry.insert(original.clone());
            }
            reserve(
                &mut tree.reservations,
                (key.len() + effect.after_state.len() + std::mem::size_of::<Value>()) * 2,
                context,
            )?;
            let Value::Dict(values) = &mut replacements
                .get_mut(&reference)
                .expect("inserted checkbox object")
                .value
            else {
                unreachable!("proved dictionary")
            };
            values.insert(key.to_vec(), Value::Name(effect.after_state.clone()));
        }
    }
    for request in requests {
        reserve(
            &mut tree.reservations,
            request.field_name.len() + std::mem::size_of::<CheckboxFieldMutationRequest>(),
            context,
        )?;
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(CheckboxPlan {
        snapshot,
        store: source.parsed().objects.clone(),
        requests: requests.to_vec(),
        effects,
        replacements,
        protected: tree.protected,
        _reservations: tree.reservations,
    })
}

pub(crate) fn apply(
    source: &PdfDocument,
    plan: &CheckboxPlan,
    changes: &mut BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    bound(source, plan)?;
    for (reference, object) in &plan.replacements {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if changes.contains_key(reference) {
            return Err(PdfError::unsafe_rewrite(
                "checkbox conflicts with another object mutation",
            ));
        }
        let cost = value_cost(&object.value, 0, source, context)?;
        context
            .charge_work(cost as u64)
            .map_err(PdfError::from_context)?;
    }
    for (reference, object) in &plan.replacements {
        context.checkpoint().map_err(PdfError::from_context)?;
        changes.insert(*reference, object.clone());
    }
    context.checkpoint().map_err(PdfError::from_context)
}

const CHECKS: [&str; 3] = [
    "requested_states",
    "registration_and_ownership",
    "source_keys_and_appearances",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.checkbox", name).expect("static checkbox check")
}
pub(crate) fn required_checks() -> RequiredChecks {
    RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
        .expect("static checkbox obligations")
}

pub(crate) fn verify(
    source: &PdfDocument,
    candidate: &PdfDocument,
    plan: &CheckboxPlan,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    bound(source, plan)?;
    if candidate.source_id() != source.source_id() {
        return Err(PdfError::selection("checkbox candidate source mismatch"));
    }
    crate::preservation::pages::validate_tree(candidate, context)?;
    let index = PdfReferenceIndex::build(candidate, context)?;
    let mut current = tree(candidate, context)?;
    let mut states = true;
    let mut ownership = true;
    for (request, expected) in plan.requests.iter().zip(&plan.effects) {
        let actual = prove(candidate, &mut current, &index, request, context)?;
        states &= actual.before_value.as_deref() == Some(expected.after_state.as_slice())
            && actual.before_appearance_state == expected.after_state
            && actual.after_state == expected.after_state;
        ownership &= actual.field == expected.field
            && actual.widget == expected.widget
            && actual.page == expected.page;
    }
    // prove extends the protected set with each selected appearance closure.
    ownership &= current.protected == plan.protected;
    type ExpectedStates<'a> = (Option<&'a [u8]>, Option<&'a [u8]>);
    let maximum_records = plan
        .effects
        .len()
        .checked_mul(2)
        .ok_or_else(|| PdfError::limit("checkbox verification state count overflows"))?;
    let state_bytes = maximum_records
        .checked_mul(std::mem::size_of::<(ObjectRef, ExpectedStates<'_>)>())
        .ok_or_else(|| PdfError::limit("checkbox verification state workspace overflows"))?;
    let _state_reservation = context
        .reserve(state_bytes as u64)
        .map_err(PdfError::from_context)?;
    let mut expected_states: BTreeMap<ObjectRef, ExpectedStates<'_>> = BTreeMap::new();
    for effect in &plan.effects {
        for (reference, field) in [
            (object_ref(effect.field), true),
            (object_ref(effect.widget), false),
        ] {
            context
                .charge_work(u64::from(usize::BITS - expected_states.len().leading_zeros()) + 1)
                .map_err(PdfError::from_context)?;
            let entry = expected_states.entry(reference).or_default();
            let state = if field { &mut entry.0 } else { &mut entry.1 };
            if state.replace(effect.after_state.as_slice()).is_some() {
                return Err(PdfError::verification(
                    "checkbox evidence has overlapping expected roles",
                ));
            }
        }
    }
    let mut preserved = true;
    let catalog = super::root_ref(source.parsed())?;
    for reference in &plan.protected {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let before = source.parsed().object(*reference)?;
        let after = candidate.parsed().object(*reference)?;
        context
            .charge_work(u64::from(usize::BITS - expected_states.len().leading_zeros()) + 1)
            .map_err(PdfError::from_context)?;
        let selected = expected_states.get(reference);
        if *reference == catalog {
            // A composed reorder may update direct PageLabels on the catalog.
            // Checkbox evidence owns its exact AcroForm association (including
            // a direct form dictionary); the transaction's other source checks
            // own unrelated catalog keys and declared label changes.
            let before = dict(&before.value)?.get(b"AcroForm".as_slice());
            let after = dict(&after.value)?.get(b"AcroForm".as_slice());
            if let Some(value) = before {
                let cost = value_cost(value, 0, source, context)?;
                context
                    .charge_work(cost as u64)
                    .map_err(PdfError::from_context)?;
            }
            preserved &= before == after;
        } else if selected.is_none() {
            let cost = value_cost(&before.value, 0, source, context)?;
            context
                .charge_work(cost as u64)
                .map_err(PdfError::from_context)?;
            preserved &= before.value == after.value;
        } else {
            let before = dict(&before.value)?;
            let after = dict(&after.value)?;
            let expected_state = |key: &[u8]| {
                selected.and_then(|states| match key {
                    b"V" => states.0,
                    b"AS" => states.1,
                    _ => None,
                })
            };
            for (key, value) in before {
                context
                    .charge_work(key.len() as u64 + 1)
                    .map_err(PdfError::from_context)?;
                if let Some(state) = expected_state(key) {
                    preserved &=
                        matches!(after.get(key), Some(Value::Name(actual)) if actual == state);
                } else {
                    let cost = value_cost(value, 0, source, context)?;
                    context
                        .charge_work(cost as u64)
                        .map_err(PdfError::from_context)?;
                    preserved &= after.get(key) == Some(value);
                }
            }
            for (key, value) in after {
                context
                    .charge_work(key.len() as u64 + 1)
                    .map_err(PdfError::from_context)?;
                if let Some(state) = expected_state(key) {
                    preserved &= matches!(value, Value::Name(actual) if actual == state);
                } else {
                    preserved &= before.get(key) == Some(value);
                }
            }
        }
        context
            .charge_work(before.stream.as_ref().map_or(0, |stream| stream.len()) as u64)
            .map_err(PdfError::from_context)?;
        preserved &= before.stream.as_deref() == after.stream.as_deref();
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(EvidenceReport::new(CHECKS.into_iter().zip([states, ownership, preserved]).map(|(name, passed)| CheckEvidence {
        code: code(name), status: if passed { CheckStatus::Passed } else { CheckStatus::Failed },
        detail: "Selected existing checkbox V/AS, unique form/page ownership and exact source registration/appearance values; no scripting, regeneration, tag repair or conformance assurance".into(),
    }).collect()))
}

fn tree(document: &PdfDocument, context: &OperationContext) -> Result<Tree, PdfError> {
    let mut tree = Tree::default();
    for object in document.parsed().objects.values() {
        calculation(&object.value, document, 0, context)?;
    }
    let root = super::root_ref(document.parsed())?;
    protect(&mut tree, root, context)?;
    let catalog = dict(&document.parsed().object(root)?.value)?;
    let acro = catalog
        .get(b"AcroForm".as_slice())
        .ok_or_else(|| PdfError::selection("document has no AcroForm"))?;
    let mut acro_location = Edge::key(root, b"AcroForm");
    let acro = resolve_at(document, acro, &mut acro_location, &mut tree, context)?;
    let acro = dict(acro)?;
    if present(acro, b"XFA").is_some() {
        return Err(PdfError::unsupported(
            "retained checkbox does not update XFA",
        ));
    }
    if let Some(value) = present(acro, b"NeedAppearances")
        && !matches!(resolve(document, value, context)?, Value::Bool(false))
    {
        return Err(PdfError::unsupported(
            "checkbox requires existing appearances without regeneration",
        ));
    }
    if let Some(value) = present(acro, b"CO")
        && !matches!(resolve(document, value, context)?, Value::Array(values) if values.is_empty())
    {
        return Err(PdfError::unsupported(
            "checkbox calculation order requires a calculation policy",
        ));
    }
    let fields = acro
        .get(b"Fields".as_slice())
        .ok_or_else(|| PdfError::selection("AcroForm has no Fields"))?;
    let mut location = acro_location;
    location.path.push(Step::Key(b"Fields".to_vec()));
    let fields = array_at(document, fields, &mut location, &mut tree, context)?;
    for (index, field) in fields.iter().enumerate() {
        let Value::Ref(reference) = field else {
            return Err(PdfError::unsupported(
                "checkbox registration requires indirect fields",
            ));
        };
        walk(
            document,
            *reference,
            None,
            "",
            location.child(index),
            0,
            &mut tree,
            context,
        )?;
    }
    context
        .charge_work(document.parsed().objects.len() as u64)
        .map_err(PdfError::from_context)?;
    for page in document.page_refs()? {
        let page_keys = dict(&document.parsed().object(page)?.value)?;
        let Some(annots) = present(page_keys, b"Annots") else {
            continue;
        };
        let mut location = Edge::key(page, b"Annots");
        let annots = array_at(document, annots, &mut location, &mut tree, context)?;
        for (index, annotation) in annots.iter().enumerate() {
            context.charge_work(1).map_err(PdfError::from_context)?;
            let Value::Ref(reference) = annotation else {
                continue;
            };
            if tree.nodes.get(reference).is_some_and(|node| node.widget) {
                add_edge(&mut tree, *reference, location.child(index), context)?;
                reserve(
                    &mut tree.reservations,
                    std::mem::size_of::<ObjectRef>(),
                    context,
                )?;
                tree.pages.entry(*reference).or_default().push(page);
            }
        }
    }
    Ok(tree)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    document: &PdfDocument,
    reference: ObjectRef,
    parent: Option<ObjectRef>,
    prefix: &str,
    edge: Edge,
    depth: usize,
    tree: &mut Tree,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > document.parsed().limits.max_parser_depth
        || tree.nodes.len() >= document.parsed().limits.max_container_items
    {
        return Err(PdfError::limit("checkbox field tree exceeds limits"));
    }
    if tree.nodes.contains_key(&reference) {
        return Err(PdfError::unsafe_rewrite(
            "checkbox field registration has a cycle or repeated node",
        ));
    }
    let object = document.parsed().object(reference)?;
    if object.stream.is_some() {
        return Err(PdfError::unsupported(
            "checkbox field node cannot be a stream",
        ));
    }
    let values = dict(&object.value)?;
    if matches!(values.get(b"Type".as_slice()), Some(Value::Name(kind)) if matches!(kind.as_slice(), b"Page" | b"Pages" | b"Catalog"))
    {
        return Err(PdfError::unsafe_rewrite(
            "checkbox field aliases a document/page role",
        ));
    }
    let actual_parent = present(values, b"Parent");
    if match parent {
        Some(parent) => actual_parent != Some(&Value::Ref(parent)),
        None => actual_parent.is_some(),
    } {
        return Err(PdfError::unsafe_rewrite(
            "checkbox field Parent contradicts registration",
        ));
    }
    protect(tree, reference, context)?;
    add_edge(tree, reference, edge, context)?;
    if let Some(parent) = parent {
        add_edge(tree, parent, Edge::key(reference, b"Parent"), context)?;
    }
    let widget =
        matches!(values.get(b"Subtype".as_slice()), Some(Value::Name(kind)) if kind == b"Widget");
    let local = present(values, b"T")
        .map(|value| strict_name(value, document, &mut tree.reservations, context))
        .transpose()?;
    let name_len = prefix
        .len()
        .checked_add(local.as_ref().map_or(0, String::len))
        .and_then(|len| len.checked_add(usize::from(!prefix.is_empty() && local.is_some())))
        .ok_or_else(|| PdfError::limit("checkbox full name length overflows"))?;
    if name_len > document.parsed().limits.max_token_bytes {
        return Err(PdfError::limit("checkbox full name exceeds token limit"));
    }
    reserve(
        &mut tree.reservations,
        name_len
            .checked_mul(2)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Node>()))
            .ok_or_else(|| PdfError::limit("checkbox field name workspace overflows"))?,
        context,
    )?;
    let name = match local {
        Some(local) if !prefix.is_empty() => format!("{prefix}.{local}"),
        Some(local) => local,
        None => prefix.to_owned(),
    };
    if values
        .get(b"T".as_slice())
        .is_some_and(|value| !matches!(value, Value::Null))
        && tree.names.insert(name.clone(), reference).is_some()
    {
        return Err(PdfError::unsafe_rewrite(
            "duplicate fully qualified form field names are unsupported",
        ));
    }
    let mut children = Vec::new();
    if let Some(kids) = present(values, b"Kids") {
        if widget {
            return Err(PdfError::unsupported(
                "combined checkbox widget cannot have Kids",
            ));
        }
        let mut location = Edge::key(reference, b"Kids");
        let kids = array_at(document, kids, &mut location, tree, context)?;
        reserve(
            &mut tree.reservations,
            kids.len()
                .checked_mul(std::mem::size_of::<ObjectRef>() * 2)
                .ok_or_else(|| PdfError::limit("checkbox Kids workspace overflows"))?,
            context,
        )?;
        for child in kids {
            let Value::Ref(child) = child else {
                return Err(PdfError::unsupported(
                    "checkbox field Kids must be indirect",
                ));
            };
            children.push(*child);
        }
        tree.nodes.insert(
            reference,
            Node {
                parent,
                children: children.clone(),
                widget,
            },
        );
        for (index, child) in children.iter().enumerate() {
            walk(
                document,
                *child,
                Some(reference),
                &name,
                location.child(index),
                depth + 1,
                tree,
                context,
            )?;
        }
    } else {
        tree.nodes.insert(
            reference,
            Node {
                parent,
                children,
                widget,
            },
        );
    }
    Ok(())
}

fn prove(
    document: &PdfDocument,
    tree: &mut Tree,
    index: &PdfReferenceIndex,
    request: &CheckboxFieldMutationRequest,
    context: &OperationContext,
) -> Result<PdfCheckboxEffect, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    let field = *tree
        .names
        .get(&request.field_name)
        .ok_or_else(|| PdfError::selection("checkbox field name not found"))?;
    let node = &tree.nodes[&field];
    let widget = if node.widget {
        if !node.children.is_empty() {
            return Err(PdfError::unsupported(
                "combined checkbox widget has children",
            ));
        }
        field
    } else {
        let [widget] = node.children.as_slice() else {
            return Err(PdfError::unsupported(
                "checkbox terminal field requires exactly one widget",
            ));
        };
        if !tree.nodes[widget].widget || !tree.nodes[widget].children.is_empty() {
            return Err(PdfError::unsupported(
                "selected checkbox is not a terminal single-widget field",
            ));
        }
        let values = dict(&document.parsed().object(*widget)?.value)?;
        if [b"T".as_slice(), b"FT", b"Ff", b"V", b"DV"]
            .iter()
            .any(|key| present(values, key).is_some())
        {
            return Err(PdfError::unsupported(
                "separate checkbox widget also owns field properties",
            ));
        }
        *widget
    };
    if !matches!(inherited(document, tree, field, b"FT", context)?, Some(Value::Name(kind)) if kind == b"Btn")
    {
        return Err(PdfError::unsupported(
            "selected field is not a checkbox button",
        ));
    }
    let flags = match inherited(document, tree, field, b"Ff", context)? {
        None => 0,
        Some(Value::Integer(flags)) => u32::try_from(*flags)
            .map_err(|_| PdfError::unsupported("invalid checkbox field flags"))?,
        Some(_) => {
            return Err(PdfError::unsupported(
                "checkbox field flags must be an integer",
            ));
        }
    };
    if flags & !6 != 0 {
        return Err(PdfError::unsupported(
            "checkbox ReadOnly, radio, pushbutton or other unsupported flags",
        ));
    }
    let mut path = Some(field);
    while let Some(reference) = path {
        selected_policy(document, reference, context)?;
        check_incoming(tree, index, reference, context)?;
        path = tree.nodes[&reference].parent;
    }
    if widget != field {
        selected_policy(document, widget, context)?;
        check_incoming(tree, index, widget, context)?;
    }
    let [page] = tree.pages.get(&widget).map_or(&[][..], Vec::as_slice) else {
        return Err(PdfError::unsafe_rewrite(
            "checkbox widget must occur in exactly one page Annots slot",
        ));
    };
    let page = *page;
    let values = dict(&document.parsed().object(widget)?.value)?;
    if present(values, b"P").is_some_and(|value| value != &Value::Ref(page)) {
        return Err(PdfError::unsafe_rewrite(
            "checkbox widget P contradicts page ownership",
        ));
    }
    if let Some(value) = present(values, b"F") {
        let Value::Integer(flags) = value else {
            return Err(PdfError::unsupported(
                "checkbox annotation flags must be an integer",
            ));
        };
        if *flags < 0 || flags & (64 | 128 | 512) != 0 {
            return Err(PdfError::unsupported(
                "checkbox annotation is read-only or locked",
            ));
        }
    }
    let rect = values
        .get(b"Rect".as_slice())
        .ok_or_else(|| PdfError::unsupported("checkbox widget has no Rect"))?;
    valid_rectangle(resolve(document, rect, context)?)?;
    let ap = values
        .get(b"AP".as_slice())
        .ok_or_else(|| PdfError::unsupported("checkbox widget has no existing appearance"))?;
    appearance_closure(document, ap, tree, context)?;
    let mut ap_location = Edge::key(widget, b"AP");
    let ap_dict = dict(resolve_at(document, ap, &mut ap_location, tree, context)?)?;
    let normal = ap_dict
        .get(b"N".as_slice())
        .ok_or_else(|| PdfError::unsupported("checkbox appearance has no normal states"))?;
    let normal = dict(resolve(document, normal, context)?)?;
    if normal.len() != 2 || !normal.contains_key(b"Off".as_slice()) {
        return Err(PdfError::unsupported(
            "checkbox requires Off and one on appearance",
        ));
    }
    let state_bytes = normal
        .keys()
        .try_fold(0usize, |bytes, key| bytes.checked_add(key.len()))
        .ok_or_else(|| PdfError::limit("checkbox state names exceed size limit"))?;
    let state_workspace = document
        .parsed()
        .limits
        .max_parser_depth
        .checked_mul(std::mem::size_of::<ObjectRef>() * 2)
        .and_then(|bytes| bytes.checked_add(state_bytes))
        .ok_or_else(|| PdfError::limit("checkbox state workspace overflows"))?;
    reserve(&mut tree.reservations, state_workspace, context)?;
    context
        .charge_work(state_bytes as u64 + 2)
        .map_err(PdfError::from_context)?;
    let on = super::button_widget_on_state(document.parsed(), values, widget)?;
    for appearance in normal.values() {
        let Value::Ref(reference) = appearance else {
            unreachable!("parent appearance validator requires indirect states")
        };
        let dictionary = dict(&document.parsed().object(*reference)?.value)?;
        let bbox = dictionary
            .get(b"BBox".as_slice())
            .ok_or_else(|| PdfError::unsupported("checkbox appearance lacks BBox"))?;
        let bbox = valid_rectangle(resolve(document, bbox, context)?)?;
        valid_appearance_matrix(document, dictionary, bbox, context)?;
    }
    reserve(
        &mut tree.reservations,
        on.len()
            .checked_mul(4)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<PdfCheckboxEffect>()))
            .ok_or_else(|| PdfError::limit("checkbox state workspace overflows"))?,
        context,
    )?;
    let before_value = match inherited(document, tree, field, b"V", context)? {
        None => None,
        Some(Value::Name(value)) if value == b"Off" || value == &on => Some(value.clone()),
        Some(_) => {
            return Err(PdfError::unsafe_rewrite(
                "checkbox field V has no proven appearance state",
            ));
        }
    };
    let Some(Value::Name(before_as)) = present(values, b"AS") else {
        return Err(PdfError::unsafe_rewrite(
            "checkbox requires an explicit existing AS name",
        ));
    };
    if before_as != before_value.as_deref().unwrap_or(b"Off") {
        return Err(PdfError::unsafe_rewrite("checkbox V and AS disagree"));
    }
    Ok(PdfCheckboxEffect {
        field: public_ref(field),
        widget: public_ref(widget),
        page: public_ref(page),
        checked: request.checked,
        before_value,
        before_appearance_state: before_as.clone(),
        after_state: if request.checked { on } else { b"Off".to_vec() },
    })
}

fn inherited<'a>(
    document: &'a PdfDocument,
    tree: &Tree,
    mut field: ObjectRef,
    key: &[u8],
    context: &OperationContext,
) -> Result<Option<&'a Value>, PdfError> {
    for _ in 0..=document.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if let Some(value) = present(dict(&document.parsed().object(field)?.value)?, key) {
            let value = resolve(document, value, context)?;
            if !matches!(value, Value::Null) {
                return Ok(Some(value));
            }
        }
        match tree.nodes[&field].parent {
            Some(parent) => field = parent,
            None => return Ok(None),
        }
    }
    Err(PdfError::limit(
        "checkbox field inheritance depth exceeds limit",
    ))
}

fn selected_policy(
    document: &PdfDocument,
    reference: ObjectRef,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let values = dict(&document.parsed().object(reference)?.value)?;
    if present(values, b"A").is_some() {
        return Err(PdfError::unsupported(
            "selected checkbox path has an action",
        ));
    }
    if let Some(actions) = present(values, b"AA")
        && !matches!(resolve(document, actions, context)?, Value::Dict(values) if values.is_empty())
    {
        return Err(PdfError::unsupported(
            "selected checkbox path has additional actions",
        ));
    }
    if [b"StructParent".as_slice(), b"StructParents"]
        .iter()
        .any(|key| present(values, key).is_some())
    {
        return Err(PdfError::unsupported(
            "tagged checkbox widgets require an association handler",
        ));
    }
    Ok(())
}

fn calculation(
    value: &Value,
    document: &PdfDocument,
    depth: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > document.parsed().limits.max_parser_depth {
        return Err(PdfError::limit(
            "checkbox calculation scan exceeds depth limit",
        ));
    }
    match value {
        Value::Dict(values) => {
            if let Some(actions) = present(values, b"AA") {
                let actions = resolve(document, actions, context)?;
                if !matches!(actions, Value::Null) {
                    let actions = dict(actions)?;
                    if present(actions, b"C").is_some() {
                        return Err(PdfError::unsupported(
                            "document contains a calculation action",
                        ));
                    }
                }
            }
            for value in values.values() {
                calculation(value, document, depth + 1, context)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                calculation(value, document, depth + 1, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn appearance_closure(
    document: &PdfDocument,
    value: &Value,
    tree: &mut Tree,
    context: &OperationContext,
) -> Result<(), PdfError> {
    fn walk(
        document: &PdfDocument,
        value: &Value,
        tree: &mut Tree,
        seen: &mut BTreeSet<ObjectRef>,
        depth: usize,
        context: &OperationContext,
    ) -> Result<(), PdfError> {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if depth > document.parsed().limits.max_parser_depth
            || seen.len() > document.parsed().limits.max_container_items
        {
            return Err(PdfError::limit(
                "checkbox appearance closure exceeds limits",
            ));
        }
        match value {
            Value::Ref(reference) if !seen.contains(reference) => {
                if tree.nodes.contains_key(reference) {
                    return Err(PdfError::unsupported(
                        "checkbox appearance references the field tree",
                    ));
                }
                reserve(
                    &mut tree.reservations,
                    std::mem::size_of::<ObjectRef>(),
                    context,
                )?;
                seen.insert(*reference);
                protect(tree, *reference, context)?;
                walk(
                    document,
                    &document.parsed().object(*reference)?.value,
                    tree,
                    seen,
                    depth + 1,
                    context,
                )?;
            }
            Value::Dict(values) => {
                if [b"StructParent".as_slice(), b"StructParents"]
                    .iter()
                    .any(|key| present(values, key).is_some())
                {
                    return Err(PdfError::unsupported(
                        "tagged checkbox appearances are not handled",
                    ));
                }
                for value in values.values() {
                    walk(document, value, tree, seen, depth + 1, context)?;
                }
            }
            Value::Array(values) => {
                for value in values {
                    walk(document, value, tree, seen, depth + 1, context)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    walk(document, value, tree, &mut BTreeSet::new(), 0, context)
}

fn check_incoming(
    tree: &mut Tree,
    index: &PdfReferenceIndex,
    reference: ObjectRef,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let actual = index.incoming(index.snapshot(), public_ref(reference))?;
    let expected = tree
        .incoming
        .get(&reference)
        .ok_or_else(|| PdfError::unsafe_rewrite("checkbox has no registration edge"))?;
    if actual.len() != expected.len() {
        return Err(PdfError::unsafe_rewrite(
            "checkbox has extra or missing ownership references",
        ));
    }
    for actual in actual {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let (edge, _scratch) = edge_from(actual, context)?;
        if actual.dangling || !expected.contains(&edge) {
            return Err(PdfError::unsafe_rewrite(
                "checkbox has an unknown incoming reference",
            ));
        }
    }
    Ok(())
}

fn edge_from(
    reference: &PdfReference,
    context: &OperationContext,
) -> Result<(Edge, Reservation), PdfError> {
    let PdfReferenceOrigin::Object(origin) = reference.origin else {
        return Err(PdfError::unsafe_rewrite(
            "checkbox has a trailer ownership reference",
        ));
    };
    let bytes = reference
        .path
        .iter()
        .try_fold(std::mem::size_of::<Edge>(), |total, step| {
            total.checked_add(
                std::mem::size_of::<Step>()
                    + match step {
                        PdfReferencePathStep::Key(key) => key.len(),
                        PdfReferencePathStep::Index(_) => 0,
                    },
            )
        })
        .ok_or_else(|| PdfError::limit("checkbox edge workspace overflows"))?;
    context
        .charge_work(bytes as u64)
        .map_err(PdfError::from_context)?;
    let reservation = context
        .reserve(bytes as u64)
        .map_err(PdfError::from_context)?;
    Ok((
        Edge {
            origin: object_ref(origin),
            path: reference
                .path
                .iter()
                .map(|step| match step {
                    PdfReferencePathStep::Key(key) => Step::Key(key.clone()),
                    PdfReferencePathStep::Index(index) => Step::Index(*index),
                })
                .collect(),
        },
        reservation,
    ))
}

fn add_edge(
    tree: &mut Tree,
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
        .ok_or_else(|| PdfError::limit("checkbox ownership edge size overflows"))?;
    reserve(&mut tree.reservations, bytes, context)?;
    if !tree.incoming.entry(target).or_default().insert(edge) {
        return Err(PdfError::unsafe_rewrite(
            "checkbox has repeated ownership edges",
        ));
    }
    Ok(())
}

fn protect(
    tree: &mut Tree,
    reference: ObjectRef,
    context: &OperationContext,
) -> Result<(), PdfError> {
    if !tree.protected.contains(&reference) {
        reserve(
            &mut tree.reservations,
            std::mem::size_of::<ObjectRef>(),
            context,
        )?;
        tree.protected.insert(reference);
    }
    Ok(())
}

fn array_at<'a>(
    document: &'a PdfDocument,
    value: &'a Value,
    edge: &mut Edge,
    tree: &mut Tree,
    context: &OperationContext,
) -> Result<&'a [Value], PdfError> {
    let value = resolve_at(document, value, edge, tree, context)?;
    let Value::Array(values) = value else {
        return Err(PdfError::unsupported(
            "checkbox registration requires an array",
        ));
    };
    if values.len() > document.parsed().limits.max_container_items {
        return Err(PdfError::limit("checkbox registration array exceeds limit"));
    }
    Ok(values)
}

fn resolve_at<'a>(
    document: &'a PdfDocument,
    mut value: &'a Value,
    edge: &mut Edge,
    tree: &mut Tree,
    context: &OperationContext,
) -> Result<&'a Value, PdfError> {
    for _ in 0..=document.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        match value {
            Value::Ref(reference) => {
                protect(tree, *reference, context)?;
                edge.origin = *reference;
                edge.path.clear();
                let object = document.parsed().object(*reference)?;
                if object.stream.is_some() {
                    return Err(PdfError::unsupported(
                        "checkbox structural value cannot be a stream",
                    ));
                }
                value = &object.value;
            }
            _ => return Ok(value),
        }
    }
    Err(PdfError::limit("checkbox value indirection exceeds limit"))
}

fn resolve<'a>(
    document: &'a PdfDocument,
    mut value: &'a Value,
    context: &OperationContext,
) -> Result<&'a Value, PdfError> {
    for _ in 0..=document.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        match value {
            Value::Ref(reference) => {
                let object = document.parsed().object(*reference)?;
                if object.stream.is_some() {
                    return Err(PdfError::unsupported(
                        "checkbox scalar value cannot be a stream",
                    ));
                }
                value = &object.value;
            }
            _ => return Ok(value),
        }
    }
    Err(PdfError::limit("checkbox scalar indirection exceeds limit"))
}

fn strict_name(
    value: &Value,
    document: &PdfDocument,
    reservations: &mut Vec<Reservation>,
    context: &OperationContext,
) -> Result<String, PdfError> {
    let Value::String(bytes) = resolve(document, value, context)? else {
        return Err(PdfError::unsupported(
            "checkbox field name must be a text string",
        ));
    };
    if bytes.len() > document.parsed().limits.max_token_bytes {
        return Err(PdfError::limit("checkbox partial name exceeds token limit"));
    }
    context
        .charge_work(bytes.len() as u64)
        .map_err(PdfError::from_context)?;
    reserve(
        reservations,
        bytes
            .len()
            .checked_mul(3)
            .ok_or_else(|| PdfError::limit("checkbox decoded name workspace overflows"))?,
        context,
    )?;
    let name = if let Some(bytes) = bytes.strip_prefix(&[0xfe, 0xff]) {
        if !bytes.len().is_multiple_of(2) {
            return Err(PdfError::unsupported(
                "checkbox name contains truncated UTF-16",
            ));
        }
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .map_err(|_| PdfError::unsupported("checkbox name contains invalid UTF-16"))?
    } else if bytes.is_ascii() {
        String::from_utf8(bytes.clone()).expect("ASCII")
    } else {
        return Err(PdfError::unsupported(
            "checkbox name encoding is not supported without loss",
        ));
    };
    if name.is_empty() || name.contains('.') {
        return Err(PdfError::unsupported(
            "checkbox partial name is empty or contains a separator",
        ));
    }
    Ok(name)
}

fn valid_rectangle(value: &Value) -> Result<[f64; 4], PdfError> {
    let Value::Array(values) = value else {
        return Err(PdfError::unsupported("checkbox rectangle must be an array"));
    };
    if values.len() != 4 {
        return Err(PdfError::unsupported(
            "checkbox rectangle needs four numbers",
        ));
    }
    let mut rectangle = [0.0; 4];
    for (number, value) in rectangle.iter_mut().zip(values) {
        *number = match value {
            Value::Integer(value) => *value as f64,
            Value::Real(value) => *value,
            _ => {
                return Err(PdfError::unsupported(
                    "checkbox rectangle contains a non-number",
                ));
            }
        };
    }
    if rectangle.iter().any(|number| !number.is_finite())
        || !(rectangle[2] - rectangle[0]).is_finite()
        || !(rectangle[3] - rectangle[1]).is_finite()
        || rectangle[2] <= rectangle[0]
        || rectangle[3] <= rectangle[1]
    {
        return Err(PdfError::unsupported(
            "checkbox rectangle is not finite and positive",
        ));
    }
    Ok(rectangle)
}

fn valid_appearance_matrix(
    document: &PdfDocument,
    dictionary: &Dict,
    bbox: [f64; 4],
    context: &OperationContext,
) -> Result<(), PdfError> {
    let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    if let Some(value) = present(dictionary, b"Matrix") {
        let Value::Array(values) = resolve(document, value, context)? else {
            return Err(PdfError::unsupported(
                "checkbox appearance Matrix must be an array",
            ));
        };
        if values.len() != 6 {
            return Err(PdfError::unsupported(
                "checkbox appearance Matrix needs six numbers",
            ));
        }
        for (number, value) in matrix.iter_mut().zip(values) {
            *number = match value {
                Value::Integer(value) => *value as f64,
                Value::Real(value) => *value,
                _ => {
                    return Err(PdfError::unsupported(
                        "checkbox appearance Matrix contains a non-number",
                    ));
                }
            };
        }
    }
    context.charge_work(12).map_err(PdfError::from_context)?;
    let determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
    if matrix.iter().any(|value| !value.is_finite())
        || !determinant.is_finite()
        || determinant == 0.0
    {
        return Err(PdfError::unsupported(
            "checkbox appearance Matrix is non-finite or singular",
        ));
    }
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for x in [bbox[0], bbox[2]] {
        for y in [bbox[1], bbox[3]] {
            let point = [
                matrix[0] * x + matrix[2] * y + matrix[4],
                matrix[1] * x + matrix[3] * y + matrix[5],
            ];
            if point.iter().any(|number| !number.is_finite()) {
                return Err(PdfError::unsupported(
                    "checkbox transformed appearance bounds overflow",
                ));
            }
            for axis in 0..2 {
                minimum[axis] = minimum[axis].min(point[axis]);
                maximum[axis] = maximum[axis].max(point[axis]);
            }
        }
    }
    for axis in 0..2 {
        let extent = maximum[axis] - minimum[axis];
        if !extent.is_finite() || extent <= 0.0 {
            return Err(PdfError::unsupported(
                "checkbox transformed appearance has empty or overflowing bounds",
            ));
        }
    }
    Ok(())
}

fn dict(value: &Value) -> Result<&Dict, PdfError> {
    match value {
        Value::Dict(values) => Ok(values),
        _ => Err(PdfError::unsupported("checkbox requires a dictionary")),
    }
}
fn present<'a>(values: &'a Dict, key: &[u8]) -> Option<&'a Value> {
    values
        .get(key)
        .filter(|value| !matches!(value, Value::Null))
}
fn public_ref(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}
fn object_ref(reference: PdfReferenceTarget) -> ObjectRef {
    ObjectRef {
        number: reference.number,
        generation: reference.generation,
    }
}
fn bound(source: &PdfDocument, plan: &CheckboxPlan) -> Result<(), PdfError> {
    if plan.snapshot.source() != source.source_id()
        || !source.parsed().objects.shares_snapshot_with(&plan.store)
    {
        return Err(PdfError::selection(
            "checkbox plan source/object-store mismatch",
        ));
    }
    Ok(())
}
fn reserve(
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
fn effect_cost(effect: &PdfCheckboxEffect) -> Result<usize, PdfError> {
    std::mem::size_of::<PdfCheckboxEffect>()
        .checked_add(effect.before_value.as_ref().map_or(0, Vec::len))
        .and_then(|bytes| bytes.checked_add(effect.before_appearance_state.len()))
        .and_then(|bytes| bytes.checked_add(effect.after_state.len()))
        .ok_or_else(|| PdfError::limit("checkbox effect size overflows"))
}
fn value_cost(
    value: &Value,
    depth: usize,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<usize, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > document.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("checkbox copied value depth exceeds limit"));
    }
    let mut cost = std::mem::size_of::<Value>();
    match value {
        Value::Name(bytes) | Value::String(bytes) => {
            cost = cost
                .checked_add(bytes.len())
                .ok_or_else(|| PdfError::limit("checkbox value size overflows"))?
        }
        Value::Array(values) => {
            for value in values {
                cost = cost
                    .checked_add(value_cost(value, depth + 1, document, context)?)
                    .ok_or_else(|| PdfError::limit("checkbox array size overflows"))?;
            }
        }
        Value::Dict(values) => {
            for (key, value) in values {
                let child = value_cost(value, depth + 1, document, context)?;
                cost = cost
                    .checked_add(key.len())
                    .and_then(|cost| cost.checked_add(child))
                    .ok_or_else(|| PdfError::limit("checkbox dictionary size overflows"))?;
            }
        }
        _ => {}
    }
    Ok(cost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfErrorCode, PdfRevision, PdfRevisionRequest};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget, RevisionId};

    fn fixture() -> PdfDocument {
        PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/checkbox-states.pdf"),
                OpenOptions::default(),
            )
            .unwrap()
    }
    fn context() -> OperationContext {
        limited(32 * 1024 * 1024, 64 * 1024 * 1024)
    }
    fn limited(memory: u64, work: u64) -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: memory,
                max_work_units: work,
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
    fn request(name: &str, checked: bool) -> CheckboxFieldMutationRequest {
        CheckboxFieldMutationRequest {
            field_name: name.into(),
            checked,
            match_index: 0,
        }
    }
    fn planned(
        document: &PdfDocument,
        requests: &[CheckboxFieldMutationRequest],
        ctx: &OperationContext,
    ) -> Result<CheckboxPlan, PdfError> {
        let index = PdfReferenceIndex::build(document, ctx)?;
        plan(document, index.snapshot(), &index, requests, ctx)
    }
    fn changed(document: &PdfDocument, number: u32, change: impl FnOnce(&mut Dict)) -> PdfDocument {
        let mut parsed = document.parsed().clone();
        let Value::Dict(values) = &mut parsed.objects.get_mut(&reference(number)).unwrap().value
        else {
            panic!("fixture dictionary");
        };
        change(values);
        document.with_parsed(parsed)
    }
    fn staged(document: &PdfDocument, plan: &CheckboxPlan, ctx: &OperationContext) -> PdfDocument {
        let mut replacements = BTreeMap::new();
        apply(document, plan, &mut replacements, ctx).unwrap();
        let mut parsed = document.parsed().clone();
        for (reference, object) in replacements {
            parsed.objects.insert(reference, object);
        }
        document.with_parsed(parsed)
    }
    fn rejected(
        source: &PdfDocument,
        candidate: &PdfDocument,
        plan: &CheckboxPlan,
        ctx: &OperationContext,
    ) {
        // Invalid ownership/state can fail before evidence construction.
        if let Ok(report) = verify(source, candidate, plan, ctx) {
            assert!(report.validate(&required_checks()).is_err());
        }
    }

    #[test]
    fn checkbox_combined_and_separate_states_preserve_existing_appearances() {
        let original = fixture();
        let ctx = context();
        let plan = planned(
            &original,
            &[
                request("accept", true),
                request("preferences.subscribe", false),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(plan.effects.len(), 2);
        assert_eq!(plan.effects[0].field, public_ref(reference(5)));
        assert_eq!(plan.effects[0].widget, public_ref(reference(5)));
        assert_eq!(plan.effects[0].page, public_ref(reference(3)));
        assert_eq!(plan.effects[1].field, public_ref(reference(7)));
        assert_eq!(plan.effects[1].widget, public_ref(reference(8)));
        assert_eq!(plan.effects[1].page, public_ref(reference(4)));
        assert_eq!(
            plan.effects[0].before_value.as_deref(),
            Some(b"Off".as_slice())
        );
        assert_eq!(
            plan.effects[1].before_value.as_deref(),
            Some(b"Yes".as_slice())
        );
        let candidate = staged(&original, &plan, &ctx);
        verify(&original, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        for number in [11, 12] {
            assert_eq!(
                original.parsed().object(reference(number)).unwrap().stream,
                candidate.parsed().object(reference(number)).unwrap().stream
            );
        }
        assert_eq!(
            dict(&original.parsed().object(reference(5)).unwrap().value)
                .unwrap()
                .get(b"V".as_slice()),
            Some(&Value::Name(b"Off".to_vec()))
        );
    }

    #[test]
    fn checkbox_inherited_values_are_materialized_only_on_selected_terminal() {
        let original = changed(&fixture(), 7, |values| {
            values.remove(b"FT".as_slice());
            values.remove(b"V".as_slice());
        });
        let original = changed(&original, 6, |values| {
            values.insert(b"FT".to_vec(), Value::Name(b"Btn".to_vec()));
            values.insert(b"Ff".to_vec(), Value::Integer(6));
            values.insert(b"V".to_vec(), Value::Name(b"Yes".to_vec()));
        });
        let ctx = context();
        let plan = planned(&original, &[request("preferences.subscribe", false)], &ctx).unwrap();
        assert_eq!(
            plan.effects[0].before_value.as_deref(),
            Some(b"Yes".as_slice())
        );
        let candidate = staged(&original, &plan, &ctx);
        verify(&original, &candidate, &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        assert_eq!(
            original.parsed().object(reference(6)).unwrap().value,
            candidate.parsed().object(reference(6)).unwrap().value
        );

        // A missing field value is a proven Off state only when AS says Off.
        let absent = changed(&fixture(), 5, |values| {
            values.remove(b"V".as_slice());
        });
        let plan = planned(&absent, &[request("accept", false)], &ctx).unwrap();
        assert_eq!(plan.effects[0].before_value, None);
        verify(&absent, &staged(&absent, &plan, &ctx), &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
    }

    #[test]
    fn checkbox_ambiguous_registration_and_unknown_incoming_edges_refuse() {
        let original = fixture();
        let cases = [
            changed(&original, 6, |values| {
                values.insert(b"T".to_vec(), Value::String(b"accept".to_vec()));
            }),
            changed(&original, 3, |values| {
                let Value::Array(annots) = values.get_mut(b"Annots".as_slice()).unwrap() else {
                    panic!()
                };
                annots.push(Value::Ref(reference(5)));
            }),
            changed(&original, 1, |values| {
                values.insert(b"UnknownOwner".to_vec(), Value::Ref(reference(5)));
            }),
            changed(&original, 5, |values| {
                values.insert(b"P".to_vec(), Value::Ref(reference(4)));
            }),
            changed(&original, 15, |values| {
                let Value::Array(fields) = values.get_mut(b"Fields".as_slice()).unwrap() else {
                    panic!()
                };
                fields.push(Value::Ref(reference(5)));
            }),
        ];
        for source in cases {
            assert!(planned(&source, &[request("accept", true)], &context()).is_err());
        }
        assert!(
            planned(
                &original,
                &[request("accept", true), request("accept", false)],
                &context()
            )
            .is_err()
        );
    }

    #[test]
    fn checkbox_unproven_states_actions_flags_and_tag_associations_refuse() {
        let original = fixture();
        for (key, value) in [
            (b"AS".as_slice(), Value::Name(b"Yes".to_vec())),
            (b"AS", Value::Null),
            (b"V", Value::Name(b"Unknown".to_vec())),
            (b"Ff", Value::Integer(1)),
            (b"Ff", Value::Integer(32768)),
            (b"F", Value::Integer(128)),
            (b"A", Value::Dict(BTreeMap::new())),
            (b"StructParent", Value::Integer(0)),
            (
                b"AP",
                Value::Dict(BTreeMap::from([(
                    b"N".to_vec(),
                    Value::Dict(BTreeMap::from([(
                        b"Off".to_vec(),
                        Value::Ref(reference(11)),
                    )])),
                )])),
            ),
        ] {
            let source = changed(&original, 5, |values| {
                values.insert(key.to_vec(), value);
            });
            assert!(
                planned(&source, &[request("accept", true)], &context()).is_err(),
                "accepted {key:?}"
            );
        }
        for (key, value) in [
            (b"NeedAppearances".as_slice(), Value::Bool(true)),
            (b"XFA", Value::Array(Vec::new())),
            (b"CO", Value::Array(vec![Value::Ref(reference(7))])),
        ] {
            let source = changed(&original, 15, |values| {
                values.insert(key.to_vec(), value);
            });
            assert!(planned(&source, &[request("accept", true)], &context()).is_err());
        }
        let unrelated_calculation = changed(&original, 23, |values| {
            values.insert(
                b"AA".to_vec(),
                Value::Dict(BTreeMap::from([(
                    b"C".to_vec(),
                    Value::Dict(BTreeMap::new()),
                )])),
            );
        });
        assert!(
            planned(
                &unrelated_calculation,
                &[request("accept", true)],
                &context()
            )
            .is_err()
        );
    }

    #[test]
    fn checkbox_source_intent_verifier_rejects_self_consistent_replacement_corruption() {
        let original = fixture();
        let ctx = context();
        let mut plan = planned(&original, &[request("accept", true)], &ctx).unwrap();
        let Value::Dict(values) = &mut plan.replacements.get_mut(&reference(5)).unwrap().value
        else {
            panic!()
        };
        values.remove(b"Rect".as_slice());
        rejected(&original, &staged(&original, &plan, &ctx), &plan, &ctx);

        let mut plan = planned(&original, &[request("accept", true)], &ctx).unwrap();
        let Value::Dict(values) = &mut plan.replacements.get_mut(&reference(5)).unwrap().value
        else {
            panic!()
        };
        values.insert(b"V".to_vec(), Value::Name(b"Off".to_vec()));
        values.insert(b"AS".to_vec(), Value::Name(b"Off".to_vec()));
        rejected(&original, &staged(&original, &plan, &ctx), &plan, &ctx);

        let plan = planned(&original, &[request("accept", true)], &ctx).unwrap();
        let candidate = staged(&original, &plan, &ctx);
        let mut corrupted = candidate.parsed().clone();
        corrupted.objects.get_mut(&reference(12)).unwrap().stream = Some(b"q Q".to_vec().into());
        rejected(&original, &candidate.with_parsed(corrupted), &plan, &ctx);

        // A same-role key loss passes state/ownership reads, so it must fail
        // the independent source-key obligation, even if staged bytes agree.
        let corrupted = changed(&candidate, 5, |values| {
            values.remove(b"TU".as_slice());
            values.insert(b"PrivateAddition".to_vec(), Value::Integer(1));
        });
        let report = verify(&original, &corrupted, &plan, &ctx).unwrap();
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code == code("source_keys_and_appearances")
                    && check.status == CheckStatus::Failed)
        );
    }

    #[test]
    fn checkbox_index_revision_and_divergent_store_bindings_are_enforced() {
        let original = fixture();
        let ctx = context();
        let index = PdfReferenceIndex::build(&original, &ctx).unwrap();
        let requests = [request("accept", true)];
        assert!(
            plan(
                &original,
                SnapshotId::new(original.source_id(), RevisionId::new(1)),
                &index,
                &requests,
                &ctx
            )
            .is_err()
        );
        let branch = changed(&original, 1, |values| {
            values.insert(b"ExtraReference".to_vec(), Value::Ref(reference(5)));
        });
        assert!(plan(&branch, index.snapshot(), &index, &requests, &ctx).is_err());
        let plan = plan(&original, index.snapshot(), &index, &requests, &ctx).unwrap();
        assert!(apply(&branch, &plan, &mut BTreeMap::new(), &ctx).is_err());
        let branch = changed(&original, 23, |values| {
            values.insert(b"Contents".to_vec(), Value::String(b"branch".to_vec()));
        });
        assert!(apply(&branch, &plan, &mut BTreeMap::new(), &ctx).is_err());
        let mut conflict = BTreeMap::from([(
            reference(5),
            original.parsed().object(reference(5)).unwrap().clone(),
        )]);
        assert!(apply(&original, &plan, &mut conflict, &ctx).is_err());
        assert_eq!(conflict.len(), 1);
    }

    #[test]
    fn checkbox_cancellation_and_cumulative_budget_failure_release_plan_workspace() {
        let original = fixture();
        let requests = [request("accept", true)];
        let index = PdfReferenceIndex::build(&original, &context()).unwrap();
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert_eq!(
            plan(&original, index.snapshot(), &index, &requests, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        assert_eq!(cancelled.budget().snapshot().live_bytes, 0);
        assert_eq!(cancelled.budget().snapshot().work_units, 0);
        for ctx in [limited(0, 10_000_000), limited(10_000_000, 0)] {
            assert_eq!(
                plan(&original, index.snapshot(), &index, &requests, &ctx)
                    .unwrap_err()
                    .code,
                PdfErrorCode::ResourceLimit
            );
            assert_eq!(ctx.budget().snapshot().live_bytes, 0);
        }
        let ctx = context();
        let first = plan(&original, index.snapshot(), &index, &requests, &ctx).unwrap();
        let spent = ctx.budget().snapshot().work_units;
        assert!(ctx.budget().snapshot().live_bytes > 0);
        drop(first);
        assert_eq!(ctx.budget().snapshot().live_bytes, 0);
        assert_eq!(ctx.budget().snapshot().work_units, spent);
        let first = plan(&original, index.snapshot(), &index, &requests, &ctx).unwrap();
        assert!(ctx.budget().snapshot().work_units > spent);
        let mut changes = BTreeMap::new();
        assert_eq!(
            apply(&original, &first, &mut changes, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        assert!(changes.is_empty());
        let candidate = staged(&original, &first, &ctx);
        assert_eq!(
            verify(&original, &candidate, &first, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
    }

    #[test]
    fn checkbox_composes_with_reorder_and_direct_catalog_page_labels() {
        let original = fixture();
        let labels = original
            .parsed()
            .object(reference(24))
            .unwrap()
            .value
            .clone();
        let original = changed(&original, 1, |values| {
            values.insert(b"PageLabels".to_vec(), labels);
        });
        let mut revision = PdfRevision::new(original);
        let ctx = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    checkboxes: vec![request("accept", true)],
                    reorder_pages: Some(vec![1, 0]),
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        revision.commit(&plan, &ctx).unwrap();
        assert_eq!(revision.snapshot().revision(), RevisionId::new(1));
    }

    #[test]
    fn checkbox_existing_appearance_matrix_must_preserve_finite_area() {
        let original = fixture();
        for matrix in [
            vec![0.0; 6],
            vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0],
            vec![1.0, 0.0, 0.0, 1.0],
            vec![f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0],
            vec![1e308, 0.0, 0.0, 1.0, 0.0, 0.0],
        ] {
            let source = changed(&original, 12, |values| {
                values.insert(
                    b"Matrix".to_vec(),
                    Value::Array(matrix.into_iter().map(Value::Real).collect()),
                );
            });
            assert!(planned(&source, &[request("accept", true)], &context()).is_err());
        }
        let rotated = changed(&original, 12, |values| {
            values.insert(
                b"Matrix".to_vec(),
                Value::Array(
                    [0, 1, -1, 0, 20, 0]
                        .into_iter()
                        .map(Value::Integer)
                        .collect(),
                ),
            );
        });
        let ctx = context();
        let plan = planned(&rotated, &[request("accept", true)], &ctx).unwrap();
        verify(&rotated, &staged(&rotated, &plan, &ctx), &plan, &ctx)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
    }
}
