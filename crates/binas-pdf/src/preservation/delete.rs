//! Logical removal of one page occurrence and its widget registration. Original
//! objects remain stored verbatim unless a retained relationship needs changing;
//! this operation is not secure erasure. Local Fit destinations require an
//! explicit retained-page remap. Unknown active references to removed roles fail.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SnapshotId,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};
use serde::{Deserialize, Serialize};

use crate::{
    PdfDocument, PdfError, PdfReferenceIndex, PdfReferencePathStep, PdfReferenceTarget,
    forms::flatten::{self, dict, present, public_ref, resolve},
    object::{IndirectObject, ObjectRef, Value},
    store::ObjectStore,
};

#[cfg(test)]
type Dict = BTreeMap<Vec<u8>, Value>;
type Changes = BTreeMap<ObjectRef, IndirectObject>;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PdfExcludedPageTargetPolicy {
    Reject,
    RemapToPage { page_index: usize },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PdfPageDeleteRequest {
    pub page_index: usize,
    pub excluded_targets: PdfExcludedPageTargetPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfPageDestinationRemap {
    pub object: PdfReferenceTarget,
    /// Source-qualified syntax path to the changed page reference.
    pub path: Vec<PdfReferencePathStep>,
    pub from: PdfReferenceTarget,
    pub to: PdfReferenceTarget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfPageDeleteEffect {
    pub page: PdfReferenceTarget,
    pub page_index: usize,
    pub remaining_pages: Vec<PdfReferenceTarget>,
    pub removed_widgets: Vec<PdfReferenceTarget>,
    pub unregistered_fields: Vec<PdfReferenceTarget>,
    pub remapped_destinations: Vec<PdfPageDestinationRemap>,
}

#[derive(Debug)]
pub(crate) struct DeletePlan {
    snapshot: SnapshotId,
    store: ObjectStore,
    request: PdfPageDeleteRequest,
    effect: PdfPageDeleteEffect,
    replacements: Changes,
    expected_order: Vec<ObjectRef>,
    source_indices: Vec<usize>,
    catalog_keys: BTreeSet<Vec<u8>>,
    _reservations: Vec<Reservation>,
}

impl DeletePlan {
    pub(crate) fn effects(&self) -> &PdfPageDeleteEffect {
        &self.effect
    }
    pub(crate) fn changed_refs(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.replacements.keys().copied()
    }
    pub(crate) fn expected_order(&self) -> &[ObjectRef] {
        &self.expected_order
    }
    pub(crate) fn source_indices(&self) -> &[usize] {
        &self.source_indices
    }
    pub(crate) fn allows_catalog_key(&self, key: &[u8]) -> bool {
        self.catalog_keys.contains(key)
    }
    pub(crate) fn catalog_keys(&self) -> impl Iterator<Item = &[u8]> {
        self.catalog_keys.iter().map(Vec::as_slice)
    }
}

struct Requested {
    effect: PdfPageDeleteEffect,
    changes: Changes,
    order: Vec<ObjectRef>,
    indices: Vec<usize>,
    catalog: ObjectRef,
    catalog_keys: BTreeSet<Vec<u8>>,
    excluded: BTreeSet<ObjectRef>,
    reservations: Vec<Reservation>,
}

pub(crate) fn plan(
    source: &PdfDocument,
    snapshot: SnapshotId,
    index: &PdfReferenceIndex,
    request: &PdfPageDeleteRequest,
    staged: &Changes,
    context: &OperationContext,
) -> Result<DeletePlan, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if snapshot.source() != source.source_id() || index.snapshot() != snapshot {
        return Err(PdfError::selection("delete source/revision mismatch"));
    }
    let fresh = PdfReferenceIndex::build(source, context)?;
    work(context, fresh.records().len())?;
    if fresh.records() != index.records() {
        return Err(PdfError::selection(
            "delete index belongs to another object-store branch",
        ));
    }
    let requested = reconstruct(source, request, context)?;
    for reference in requested.changes.keys() {
        work(context, 1)?;
        if staged.contains_key(reference) {
            return Err(PdfError::unsafe_rewrite(
                "delete overlaps another staged object",
            ));
        }
    }
    Ok(DeletePlan {
        snapshot,
        store: source.parsed().objects.clone(),
        request: request.clone(),
        effect: requested.effect,
        replacements: requested.changes,
        expected_order: requested.order,
        source_indices: requested.indices,
        catalog_keys: requested.catalog_keys,
        _reservations: requested.reservations,
    })
}

pub(crate) fn apply(
    source: &PdfDocument,
    plan: &DeletePlan,
    changes: &mut Changes,
    context: &OperationContext,
) -> Result<(), PdfError> {
    bound(source, plan)?;
    for reference in plan.replacements.keys() {
        work(context, 1)?;
        if changes.contains_key(reference) {
            return Err(PdfError::unsafe_rewrite("delete overlaps staged objects"));
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

const CHECKS: [&str; 4] = [
    "page_mapping",
    "registration_and_destinations",
    "source_preservation",
    "excluded_roles_unreachable",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.delete", name).expect("static delete check")
}
pub(crate) fn required_checks() -> RequiredChecks {
    RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
        .expect("static delete requirements")
}

pub(crate) fn verify(
    source: &PdfDocument,
    candidate: &PdfDocument,
    plan: &DeletePlan,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    bound(source, plan)?;
    if candidate.source_id() != source.source_id() {
        return Err(PdfError::selection("delete candidate source mismatch"));
    }
    // Re-derive every authorized value and semantic effect from the immutable
    // source and request. Neither the retained replacement map nor its effects
    // are accepted as proof of what deletion was permitted to change.
    let expected = reconstruct(source, &plan.request, context)?;
    let tree_ok = candidate.page_refs()? == expected.order
        && plan.expected_order == expected.order
        && plan.source_indices == expected.indices;
    super::pages::validate_tree(candidate, context)?;
    let effects_ok = plan.effect == expected.effect
        && plan.catalog_keys == expected.catalog_keys
        && plan.replacements.keys().eq(expected.changes.keys());
    let mut labels = BTreeSet::new();
    super::labels::visit_tree_objects(source, context, |reference| {
        labels.insert(reference);
        Ok(())
    })?;
    work(
        context,
        flatten::value_cost(&candidate.parsed().trailer, candidate, 0, context)?,
    )?;
    let mut preserved = source.parsed().trailer == candidate.parsed().trailer
        && source
            .parsed()
            .objects
            .keys()
            .eq(candidate.parsed().objects.keys());
    for (reference, before) in &source.parsed().objects {
        work(context, 1)?;
        let wanted = expected.changes.get(reference).unwrap_or(before);
        let actual = candidate.parsed().object(*reference)?;
        if *reference == expected.catalog {
            let a = dict(&wanted.value)?;
            let b = dict(&actual.value)?;
            work(
                context,
                flatten::value_cost(&actual.value, source, 0, context)?,
            )?;
            preserved &= a
                .iter()
                .filter(|(key, _)| key.as_slice() != b"PageLabels")
                .eq(b.iter().filter(|(key, _)| key.as_slice() != b"PageLabels"))
                && actual.stream.is_none();
        } else if !labels.contains(reference) {
            preserved &= flatten::objects_equal(wanted, actual, source, context)?;
        }
    }
    let active = active_objects(candidate, &BTreeMap::new(), context)?;
    let excluded_ok = expected.excluded.is_disjoint(&active);
    Ok(EvidenceReport::new(CHECKS.iter().zip([tree_ok, effects_ok, preserved, excluded_ok]).map(|(name, passed)| CheckEvidence {
        code: code(name), status: if passed { CheckStatus::Passed } else { CheckStatus::Failed }, detail: format!("Source-derived logical page deletion: {name}. Original stored objects retained; no secure erasure or tag reconciliation. Labels checked separately."),
    }).collect()))
}

fn reconstruct(
    source: &PdfDocument,
    request: &PdfPageDeleteRequest,
    context: &OperationContext,
) -> Result<Requested, PdfError> {
    crate::writer::refuse_security_boundaries_with_context(source.parsed(), Some(context))?;
    super::pages::validate_tree(source, context)?;
    // Precharge copied dictionary/path/set workspace. Encoded stream payloads
    // remain shared and are never decoded or included in these logical copies.
    let mut cost = source
        .parsed()
        .objects
        .len()
        .checked_mul(256)
        .ok_or_else(|| PdfError::limit("delete workspace overflows"))?;
    cost = cost
        .checked_add(
            flatten::value_cost(&source.parsed().trailer, source, 0, context)?
                .checked_mul(6)
                .ok_or_else(|| PdfError::limit("delete workspace overflows"))?,
        )
        .ok_or_else(|| PdfError::limit("delete workspace overflows"))?;
    for object in source.parsed().objects.values() {
        cost = cost
            .checked_add(
                flatten::value_cost(&object.value, source, 0, context)?
                    .checked_mul(6)
                    .ok_or_else(|| PdfError::limit("delete workspace overflows"))?,
            )
            .ok_or_else(|| PdfError::limit("delete workspace overflows"))?;
    }
    let mut reservations = Vec::new();
    flatten::reserve(&mut reservations, cost, context)?;
    let pages = source.page_refs()?;
    let page = *pages
        .get(request.page_index)
        .ok_or_else(|| PdfError::selection("delete page index is out of range"))?;
    if pages.len() <= 1 {
        return Err(PdfError::unsupported(
            "delete cannot produce a zero-page document",
        ));
    }
    let target = match request.excluded_targets {
        PdfExcludedPageTargetPolicy::Reject => None,
        PdfExcludedPageTargetPolicy::RemapToPage { page_index } => {
            let target = *pages.get(page_index).ok_or_else(|| {
                PdfError::selection("delete destination page index is out of range")
            })?;
            if target == page {
                return Err(PdfError::selection("delete remap target must be retained"));
            }
            Some(target)
        }
    };
    let catalog = reference(present(dict(&source.parsed().trailer)?, b"Root"))?;
    let catalog_values = dict(&source.parsed().object(catalog)?.value)?;
    if present(catalog_values, b"StructTreeRoot").is_some() {
        return Err(PdfError::unsupported(
            "delete does not reconcile tagged structure trees",
        ));
    }
    let indices: Vec<_> = (0..pages.len())
        .filter(|index| *index != request.page_index)
        .collect();
    let order: Vec<_> = indices.iter().map(|index| pages[*index]).collect();
    let mut changes = BTreeMap::new();
    let mut excluded = BTreeSet::from([page]);
    let root = reference(present(catalog_values, b"Pages"))?;
    prune_tree(source, page, root, &mut changes, &mut excluded, context)?;
    let mut removed_widgets = BTreeSet::new();
    for annotation in annotations(source, page, context)? {
        excluded.insert(annotation);
        let values = dict(&source.parsed().object(annotation)?.value)?;
        if let Some(owner) = present(values, b"P")
            && owner != &Value::Ref(page)
        {
            return Err(PdfError::unsafe_rewrite(
                "delete annotation P contradicts selected page",
            ));
        }
        if matches!(present(values, b"Subtype"), Some(Value::Name(kind)) if kind == b"Widget") {
            removed_widgets.insert(annotation);
        }
    }
    let mut unregistered = BTreeSet::new();
    let mut catalog_keys = BTreeSet::new();
    if present(catalog_values, b"AcroForm").is_some() {
        let registry = flatten::registry(source, context)?;
        for widget in &removed_widgets {
            if !registry.nodes.get(widget).is_some_and(|node| node.widget)
                || registry
                    .pages
                    .get(widget)
                    .is_none_or(|owners| owners.as_slice() != [page])
            {
                return Err(PdfError::unsafe_rewrite(
                    "delete widget registration is missing or shared across pages",
                ));
            }
        }
        let mut removed = removed_widgets.clone();
        loop {
            let mut progress = false;
            for (reference, node) in &registry.nodes {
                work(context, 1)?;
                if !removed.contains(reference)
                    && !node.children.is_empty()
                    && node.children.iter().all(|child| removed.contains(child))
                {
                    removed.insert(*reference);
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        for (reference, node) in &registry.nodes {
            if !removed.contains(reference)
                && node.children.iter().any(|child| removed.contains(child))
                && node.children.iter().any(|child| {
                    let child_values = dict(
                        &source
                            .parsed()
                            .object(*child)
                            .expect("registered child")
                            .value,
                    )
                    .expect("registered dictionary");
                    registry.nodes[child].widget
                        && present(child_values, b"T").is_none()
                        && present(child_values, b"FT").is_none()
                })
            {
                let kind = flatten::inherited(source, &registry, *reference, b"FT", context)?;
                if !matches!(kind, Some(Value::Name(kind)) if kind == b"Tx") {
                    return Err(PdfError::unsupported(
                        "delete partial widget removal requires a text field",
                    ));
                }
            }
        }
        for reference in &removed {
            let values = dict(&source.parsed().object(*reference)?.value)?;
            if !registry.nodes[reference].widget
                || present(values, b"T").is_some()
                || present(values, b"FT").is_some()
            {
                unregistered.insert(*reference);
            }
        }
        for (reference, value) in
            flatten::registration_changes(source, &registry, &removed, context)?
        {
            let value = if registry.acro_direct && reference == catalog {
                let mut values = catalog_values.clone();
                values.insert(b"AcroForm".to_vec(), value);
                catalog_keys.insert(b"AcroForm".to_vec());
                Value::Dict(values)
            } else {
                value
            };
            replace(source, &mut changes, reference, value)?;
        }
        excluded.extend(removed);
    } else if !removed_widgets.is_empty() {
        return Err(PdfError::unsupported(
            "delete page contains unregistered widgets",
        ));
    }
    let mut remaps = Vec::new();
    remap(
        source,
        catalog,
        vec![key(b"OpenAction")],
        page,
        target,
        &mut changes,
        &mut remaps,
        context,
    )?;
    // Navigation identities are discovered through actual catalog and surviving
    // page roles. A key merely named Dest inside opaque data is never rewritten.
    for retained in &order {
        for annotation in annotations(source, *retained, context)? {
            let values = dict(&source.parsed().object(annotation)?.value)?;
            if matches!(present(values, b"Subtype"), Some(Value::Name(kind)) if kind == b"Link") {
                remap(
                    source,
                    annotation,
                    vec![key(b"Dest")],
                    page,
                    target,
                    &mut changes,
                    &mut remaps,
                    context,
                )?;
            }
        }
    }
    if let Some(value) = present(catalog_values, b"Outlines") {
        let outline = reference(Some(value))?;
        let values = dict(&source.parsed().object(outline)?.value)?;
        let mut queue = Vec::new();
        if let Some(first) = present(values, b"First") {
            queue.push(reference(Some(first))?);
        }
        let mut seen = BTreeSet::new();
        while let Some(item) = queue.pop() {
            work(context, 1)?;
            if !seen.insert(item) {
                return Err(PdfError::unsafe_rewrite(
                    "delete outline traversal repeats an item",
                ));
            }
            let values = dict(&source.parsed().object(item)?.value)?;
            remap(
                source,
                item,
                vec![key(b"Dest")],
                page,
                target,
                &mut changes,
                &mut remaps,
                context,
            )?;
            for child in [b"First".as_slice(), b"Next"] {
                if let Some(value) = present(values, child) {
                    queue.push(reference(Some(value))?);
                }
            }
        }
    }
    let mut names = Navigator {
        source,
        page,
        target,
        changes: &mut changes,
        remaps: &mut remaps,
        seen: BTreeSet::new(),
        context,
    };
    if let Some(value) = present(catalog_values, b"Names") {
        let (owner, path, value) = names.locate(catalog, vec![key(b"Names")], value)?;
        if let Some(dests) = present(dict(value)?, b"Dests") {
            let mut path = path;
            path.push(key(b"Dests"));
            names.tree(owner, path, dests, 0)?;
        }
    }
    if let Some(value) = present(catalog_values, b"Dests") {
        let (owner, path, value) = names.locate(catalog, vec![key(b"Dests")], value)?;
        for name in dict(value)?.keys() {
            let mut path = path.clone();
            path.push(PdfReferencePathStep::Key(name.clone()));
            names.destination(owner, path)?;
        }
    }
    // Exact changed catalog keys are computed from source values, not requested
    // or caller-supplied permission lists. Labels belong to a separate checker.
    if let Some(changed) = changes.get(&catalog) {
        for (key, value) in dict(&changed.value)? {
            if catalog_values.get(key) != Some(value) {
                catalog_keys.insert(key.clone());
            }
        }
    }
    let active = active_objects(source, &changes, context)?;
    if !excluded.is_disjoint(&active) {
        return Err(PdfError::unsupported(
            "delete leaves an excluded page, tree node, annotation or field reachable through an unsupported reference",
        ));
    }
    for reference in &active {
        reject_actions(
            &changes
                .get(reference)
                .unwrap_or(source.parsed().object(*reference)?)
                .value,
            source,
            *reference == catalog,
            0,
            context,
        )?;
    }
    Ok(Requested {
        effect: PdfPageDeleteEffect {
            page: public_ref(page),
            page_index: request.page_index,
            remaining_pages: order.iter().copied().map(public_ref).collect(),
            removed_widgets: removed_widgets.into_iter().map(public_ref).collect(),
            unregistered_fields: unregistered.into_iter().map(public_ref).collect(),
            remapped_destinations: remaps,
        },
        changes,
        order,
        indices,
        catalog,
        catalog_keys,
        excluded,
        reservations,
    })
}

fn prune_tree(
    source: &PdfDocument,
    page: ObjectRef,
    root: ObjectRef,
    changes: &mut Changes,
    excluded: &mut BTreeSet<ObjectRef>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let mut child = page;
    let mut current = reference(present(
        dict(&source.parsed().object(page)?.value)?,
        b"Parent",
    ))?;
    let mut seen = BTreeSet::new();
    let mut unlink = true;
    loop {
        work(context, 1)?;
        if !seen.insert(current) || seen.len() > source.parsed().limits.max_parser_depth {
            return Err(PdfError::unsafe_rewrite(
                "delete page ancestors are cyclic or too deep",
            ));
        }
        let before = dict(&source.parsed().object(current)?.value)?;
        let Value::Array(kids) = resolve(
            source,
            present(before, b"Kids")
                .ok_or_else(|| PdfError::unsupported("delete Pages has no Kids"))?,
            context,
        )?
        else {
            return Err(PdfError::unsupported("delete requires page Kids arrays"));
        };
        let Value::Integer(count) = present(before, b"Count")
            .ok_or_else(|| PdfError::unsupported("delete Pages has no Count"))?
        else {
            return Err(PdfError::unsupported("delete requires direct page Count"));
        };
        let mut values = before.clone();
        if unlink {
            let retained: Vec<_> = kids
                .iter()
                .filter(|value| **value != Value::Ref(child))
                .cloned()
                .collect();
            if retained.len() + 1 != kids.len() {
                return Err(PdfError::unsafe_rewrite(
                    "delete page child is not uniquely owned",
                ));
            }
            unlink = retained.is_empty();
            values.insert(b"Kids".to_vec(), Value::Array(retained));
        }
        if *count <= 0 {
            return Err(PdfError::unsafe_rewrite("delete invalid page count"));
        }
        values.insert(b"Count".to_vec(), Value::Integer(count - 1));
        if unlink && current != root {
            excluded.insert(current);
        } else {
            replace(source, changes, current, Value::Dict(values))?;
        }
        if current == root {
            if unlink {
                return Err(PdfError::unsupported("delete cannot remove page-tree root"));
            }
            break;
        }
        child = current;
        current = reference(present(before, b"Parent"))?;
    }
    Ok(())
}

fn annotations(
    source: &PdfDocument,
    page: ObjectRef,
    context: &OperationContext,
) -> Result<Vec<ObjectRef>, PdfError> {
    let values = dict(&source.parsed().object(page)?.value)?;
    if present(values, b"StructParents").is_some() || present(values, b"StructParent").is_some() {
        return Err(PdfError::unsupported(
            "delete does not remap page tagging associations",
        ));
    }
    let Some(value) = present(values, b"Annots") else {
        return Ok(Vec::new());
    };
    let Value::Array(values) = resolve(source, value, context)? else {
        return Err(PdfError::unsupported("delete requires annotation arrays"));
    };
    let mut seen = BTreeSet::new();
    for value in values {
        work(context, 1)?;
        if !seen.insert(reference(Some(value))?) {
            return Err(PdfError::unsafe_rewrite(
                "delete annotation is repeated on a page",
            ));
        }
    }
    Ok(seen.into_iter().collect())
}

struct Navigator<'a> {
    source: &'a PdfDocument,
    page: ObjectRef,
    target: Option<ObjectRef>,
    changes: &'a mut Changes,
    remaps: &'a mut Vec<PdfPageDestinationRemap>,
    seen: BTreeSet<ObjectRef>,
    context: &'a OperationContext,
}
impl<'a> Navigator<'a> {
    fn locate(
        &mut self,
        mut owner: ObjectRef,
        mut path: Vec<PdfReferencePathStep>,
        mut value: &'a Value,
    ) -> Result<(ObjectRef, Vec<PdfReferencePathStep>, &'a Value), PdfError> {
        for _ in 0..=self.source.parsed().limits.max_parser_depth {
            work(self.context, 1)?;
            let Value::Ref(reference) = value else {
                return Ok((owner, path, value));
            };
            if !self.seen.insert(*reference) {
                return Err(PdfError::unsafe_rewrite(
                    "delete navigation containers are cyclic or shared",
                ));
            }
            owner = *reference;
            path.clear();
            let object = self.source.parsed().object(owner)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "delete navigation container cannot be a stream",
                ));
            }
            value = &object.value;
        }
        Err(PdfError::limit(
            "delete navigation indirection exceeds depth",
        ))
    }
    fn tree(
        &mut self,
        owner: ObjectRef,
        path: Vec<PdfReferencePathStep>,
        value: &'a Value,
        depth: usize,
    ) -> Result<(), PdfError> {
        if depth > self.source.parsed().limits.max_parser_depth {
            return Err(PdfError::limit("delete name tree exceeds depth"));
        }
        let (owner, path, value) = self.locate(owner, path, value)?;
        let values = dict(value)?;
        if let Some(kids) = present(values, b"Kids") {
            let mut child_path = path.clone();
            child_path.push(key(b"Kids"));
            let (owner, child_path, kids) = self.locate(owner, child_path, kids)?;
            let Value::Array(kids) = kids else {
                return Err(PdfError::unsupported("delete name Kids must be array"));
            };
            for (index, child) in kids.iter().enumerate() {
                let mut path = child_path.clone();
                path.push(PdfReferencePathStep::Index(index));
                self.tree(owner, path, child, depth + 1)?;
            }
        }
        if let Some(names) = present(values, b"Names") {
            let mut names_path = path;
            names_path.push(key(b"Names"));
            let (owner, names_path, names) = self.locate(owner, names_path, names)?;
            let Value::Array(names) = names else {
                return Err(PdfError::unsupported("delete name Names must be array"));
            };
            if names.len() % 2 != 0 {
                return Err(PdfError::unsupported(
                    "delete name tree has unpaired entries",
                ));
            }
            for index in (1..names.len()).step_by(2) {
                if !matches!(names[index - 1], Value::String(_)) {
                    return Err(PdfError::unsupported(
                        "delete destination name must be a string",
                    ));
                }
                let mut path = names_path.clone();
                path.push(PdfReferencePathStep::Index(index));
                self.destination(owner, path)?;
            }
        }
        Ok(())
    }
    fn destination(
        &mut self,
        owner: ObjectRef,
        mut path: Vec<PdfReferencePathStep>,
    ) -> Result<(), PdfError> {
        if matches!(
            at(&self.source.parsed().object(owner)?.value, &path),
            Some(Value::Dict(_))
        ) {
            path.push(key(b"D"));
        }
        remap(
            self.source,
            owner,
            path,
            self.page,
            self.target,
            self.changes,
            self.remaps,
            self.context,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn remap(
    source: &PdfDocument,
    owner: ObjectRef,
    mut path: Vec<PdfReferencePathStep>,
    page: ObjectRef,
    target: Option<ObjectRef>,
    changes: &mut Changes,
    effects: &mut Vec<PdfPageDestinationRemap>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    work(context, path.len() + 1)?;
    let Some(Value::Array(values)) = at(&source.parsed().object(owner)?.value, &path) else {
        return Ok(());
    };
    if values.first() != Some(&Value::Ref(page)) {
        return Ok(());
    }
    if values.len() != 2 || values[1] != Value::Name(b"Fit".to_vec()) {
        return Err(PdfError::unsupported(
            "delete only remaps direct local Fit destinations",
        ));
    }
    let target = target.ok_or_else(|| {
        PdfError::unsupported("delete would exclude a navigation target; explicit remap required")
    })?;
    if source.parsed().object(owner)?.stream.is_some() {
        return Err(PdfError::unsupported(
            "delete navigation dictionary cannot be a stream",
        ));
    }
    let mut object = changes
        .get(&owner)
        .unwrap_or(source.parsed().object(owner)?)
        .clone();
    path.push(PdfReferencePathStep::Index(0));
    *at_mut(&mut object.value, &path)
        .ok_or_else(|| PdfError::unsafe_rewrite("delete navigation roles overlap"))? =
        Value::Ref(target);
    object.offset = 0;
    object.stream_offset = 0;
    changes.insert(owner, object);
    effects.push(PdfPageDestinationRemap {
        object: public_ref(owner),
        path,
        from: public_ref(page),
        to: public_ref(target),
    });
    Ok(())
}

fn active_objects(
    source: &PdfDocument,
    changes: &Changes,
    context: &OperationContext,
) -> Result<BTreeSet<ObjectRef>, PdfError> {
    let mut queue = Vec::new();
    refs(&source.parsed().trailer, &mut queue, source, 0, context)?;
    let mut active = BTreeSet::new();
    while let Some(reference) = queue.pop() {
        work(context, 1)?;
        if active.insert(reference) {
            refs(
                &changes
                    .get(&reference)
                    .unwrap_or(source.parsed().object(reference)?)
                    .value,
                &mut queue,
                source,
                0,
                context,
            )?;
        }
    }
    Ok(active)
}
fn refs(
    value: &Value,
    refs_out: &mut Vec<ObjectRef>,
    source: &PdfDocument,
    depth: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    work(context, 1)?;
    if depth > source.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("delete reference scan exceeds depth"));
    }
    match value {
        Value::Ref(reference) => refs_out.push(*reference),
        Value::Array(values) => {
            for value in values {
                refs(value, refs_out, source, depth + 1, context)?;
            }
        }
        Value::Dict(values) => {
            for value in values.values() {
                refs(value, refs_out, source, depth + 1, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn reject_actions(
    value: &Value,
    source: &PdfDocument,
    catalog_root: bool,
    depth: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    work(context, 1)?;
    if depth > source.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("delete action scan exceeds depth"));
    }
    match value {
        Value::Dict(values) => {
            for key in [
                b"A".as_slice(),
                b"AA",
                b"JS",
                b"XFA",
                b"StructParent",
                b"StructParents",
            ] {
                if present(values, key).is_some() {
                    return Err(PdfError::unsupported(
                        "delete cannot reconcile actions, XFA or tagged associations",
                    ));
                }
            }
            if let Some(value) = present(values, b"OpenAction")
                && (!catalog_root
                    || !matches!(value, Value::Array(values) if values.len() == 2 && matches!(values[0], Value::Ref(_)) && values[1] == Value::Name(b"Fit".to_vec())))
            {
                return Err(PdfError::unsupported(
                    "delete only preserves the catalog's direct local Fit OpenAction",
                ));
            }
            for value in values.values() {
                reject_actions(value, source, false, depth + 1, context)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_actions(value, source, false, depth + 1, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn replace(
    source: &PdfDocument,
    changes: &mut Changes,
    reference: ObjectRef,
    value: Value,
) -> Result<(), PdfError> {
    let original = source.parsed().object(reference)?;
    if original.stream.is_some() || changes.contains_key(&reference) {
        return Err(PdfError::unsafe_rewrite(
            "delete document roles overlap or use stream dictionaries",
        ));
    }
    let mut object = original.clone();
    object.value = value;
    object.offset = 0;
    object.stream_offset = 0;
    changes.insert(reference, object);
    Ok(())
}
fn key(value: &[u8]) -> PdfReferencePathStep {
    PdfReferencePathStep::Key(value.to_vec())
}
fn reference(value: Option<&Value>) -> Result<ObjectRef, PdfError> {
    match value {
        Some(Value::Ref(reference)) => Ok(*reference),
        _ => Err(PdfError::unsupported(
            "delete requires an indirect object reference",
        )),
    }
}
fn at<'a>(mut value: &'a Value, path: &[PdfReferencePathStep]) -> Option<&'a Value> {
    for step in path {
        value = match (value, step) {
            (Value::Dict(values), PdfReferencePathStep::Key(key)) => values.get(key)?,
            (Value::Array(values), PdfReferencePathStep::Index(index)) => values.get(*index)?,
            _ => return None,
        };
    }
    Some(value)
}
fn at_mut<'a>(mut value: &'a mut Value, path: &[PdfReferencePathStep]) -> Option<&'a mut Value> {
    for step in path {
        value = match (value, step) {
            (Value::Dict(values), PdfReferencePathStep::Key(key)) => values.get_mut(key)?,
            (Value::Array(values), PdfReferencePathStep::Index(index)) => values.get_mut(*index)?,
            _ => return None,
        };
    }
    Some(value)
}
fn work(context: &OperationContext, amount: usize) -> Result<(), PdfError> {
    context
        .charge_work(amount as u64)
        .map_err(PdfError::from_context)
}
fn bound(source: &PdfDocument, plan: &DeletePlan) -> Result<(), PdfError> {
    if plan.snapshot.source() != source.source_id()
        || !source.parsed().objects.shares_snapshot_with(&plan.store)
    {
        Err(PdfError::selection(
            "delete plan source/object-store mismatch",
        ))
    } else {
        Ok(())
    }
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
    fn r(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn request() -> PdfPageDeleteRequest {
        PdfPageDeleteRequest {
            page_index: 1,
            excluded_targets: PdfExcludedPageTargetPolicy::RemapToPage { page_index: 0 },
        }
    }
    fn try_plan(
        source: &PdfDocument,
        request: &PdfPageDeleteRequest,
        context: &OperationContext,
    ) -> Result<DeletePlan, PdfError> {
        plan(
            source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &PdfReferenceIndex::build(source, context)?,
            request,
            &BTreeMap::new(),
            context,
        )
    }
    fn staged(source: &PdfDocument, plan: &DeletePlan, context: &OperationContext) -> PdfDocument {
        let mut changes = BTreeMap::new();
        apply(source, plan, &mut changes, context).unwrap();
        let mut parsed = source.parsed().clone();
        for (reference, object) in changes {
            parsed.objects.insert(reference, object);
        }
        source.with_parsed(parsed)
    }
    fn changed(source: &PdfDocument, number: u32, change: impl FnOnce(&mut Dict)) -> PdfDocument {
        let mut parsed = source.parsed().clone();
        let Value::Dict(values) = &mut parsed.objects.get_mut(&r(number)).unwrap().value else {
            panic!("test dictionary")
        };
        change(values);
        source.with_parsed(parsed)
    }
    fn good(source: &PdfDocument, plan: &DeletePlan, context: &OperationContext) -> PdfDocument {
        let candidate = staged(source, plan, context);
        verify(source, &candidate, plan, context)
            .unwrap()
            .validate(&required_checks())
            .unwrap();
        candidate
    }
    fn rejected(
        source: &PdfDocument,
        candidate: &PdfDocument,
        plan: &DeletePlan,
        context: &OperationContext,
    ) {
        if let Ok(report) = verify(source, candidate, plan, context) {
            assert!(report.validate(&required_checks()).is_err());
        }
    }

    #[test]
    fn deletion_reconciles_partial_widgets_and_three_classified_targets() {
        let source = fixture();
        let ctx = context();
        let plan = try_plan(&source, &request(), &ctx).unwrap();
        let candidate = good(&source, &plan, &ctx);
        assert_eq!(
            plan.effect.removed_widgets,
            [10, 12, 13].map(|n| public_ref(r(n)))
        );
        assert_eq!(
            plan.effect.unregistered_fields,
            [12, 13].map(|n| public_ref(r(n)))
        );
        assert_eq!(
            plan.effect
                .remapped_destinations
                .iter()
                .map(|effect| effect.object.number)
                .collect::<Vec<_>>(),
            [18, 26, 27]
        );
        assert_eq!(
            plan.changed_refs()
                .map(|reference| reference.number)
                .collect::<Vec<_>>(),
            [2, 8, 18, 23, 26, 27]
        );
        assert_eq!(plan.source_indices(), [0]);
        assert_eq!(plan.expected_order(), [r(3)]);
        assert_eq!(
            source.parsed().objects.len(),
            candidate.parsed().objects.len()
        );
        for number in [
            3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 14, 19, 22, 28, 29, 30, 32, 33, 34,
        ] {
            assert!(
                flatten::objects_equal(
                    source.parsed().object(r(number)).unwrap(),
                    candidate.parsed().object(r(number)).unwrap(),
                    &source,
                    &ctx
                )
                .unwrap()
            );
        }
        let active = active_objects(&candidate, &BTreeMap::new(), &ctx).unwrap();
        assert!(
            [4, 6, 10, 12, 13, 15, 16, 17]
                .into_iter()
                .all(|n| !active.contains(&r(n)))
        );
    }

    #[test]
    fn verification_rejects_self_consistent_unauthorized_values_and_effects() {
        let source = fixture();
        let ctx = context();
        let mut plan = try_plan(&source, &request(), &ctx).unwrap();
        let candidate = good(&source, &plan, &ctx);
        for (number, key, value) in [
            (
                8,
                b"V".as_slice(),
                Value::String(b"lost user value".to_vec()),
            ),
            (23, b"Fields".as_slice(), Value::Array(Vec::new())),
            (
                18,
                b"Dest".as_slice(),
                Value::Array(vec![Value::Ref(r(4)), Value::Name(b"Fit".to_vec())]),
            ),
            (2, b"Count".as_slice(), Value::Integer(2)),
        ] {
            let corrupt = changed(&candidate, number, |values| {
                values.insert(key.to_vec(), value.clone());
            });
            plan.replacements.insert(
                r(number),
                corrupt.parsed().object(r(number)).unwrap().clone(),
            );
            rejected(&source, &corrupt, &plan, &ctx);
            plan = try_plan(&source, &request(), &ctx).unwrap();
        }
        let corrupt = changed(&candidate, 34, |values| {
            values.clear();
        });
        rejected(&source, &corrupt, &plan, &ctx);
        plan.effect.remapped_destinations.clear();
        rejected(&source, &candidate, &plan, &ctx);
        let mut plan = try_plan(&source, &request(), &ctx).unwrap();
        plan.catalog_keys.insert(b"Private".to_vec());
        rejected(&source, &candidate, &plan, &ctx);
    }

    #[test]
    fn unknown_active_edges_cannot_keep_removed_roles_alive() {
        for removed in [4, 10, 12] {
            let source = changed(&fixture(), 34, |values| {
                values.insert(b"OpaqueTarget".to_vec(), Value::Ref(r(removed)));
            });
            assert!(try_plan(&source, &request(), &context()).is_err());
        }
        let source = changed(&fixture(), 34, |values| {
            values.insert(
                b"Dest".to_vec(),
                Value::Array(vec![Value::Ref(r(4)), Value::Name(b"Fit".to_vec())]),
            );
        });
        assert!(try_plan(&source, &request(), &context()).is_err());
        // A retained historic object is not an active owner. Retained logical
        // revisions may legitimately contain such detached original references.
        let source = fixture();
        let mut parsed = source.parsed().clone();
        let mut detached = parsed.object(r(34)).unwrap().clone();
        detached.value = Value::Dict(BTreeMap::from([(
            b"OpaqueTarget".to_vec(),
            Value::Ref(r(4)),
        )]));
        parsed.objects.insert(r(35), detached);
        let source = source.with_parsed(parsed);
        let ctx = context();
        let plan = try_plan(&source, &request(), &ctx).unwrap();
        good(&source, &plan, &ctx);
    }

    #[test]
    fn empty_page_ancestors_are_detached_without_reparenting_survivors() {
        let source = fixture();
        let mut parsed = source.parsed().clone();
        let mut parent = parsed.object(r(2)).unwrap().clone();
        parent.value = Value::Dict(BTreeMap::from([
            (b"Type".to_vec(), Value::Name(b"Pages".to_vec())),
            (b"Parent".to_vec(), Value::Ref(r(2))),
            (b"Kids".to_vec(), Value::Array(vec![Value::Ref(r(4))])),
            (b"Count".to_vec(), Value::Integer(1)),
            (
                b"Private".to_vec(),
                Value::String(b"retained historical node".to_vec()),
            ),
        ]));
        parsed.objects.insert(r(35), parent);
        let source = source.with_parsed(parsed);
        let source = changed(&source, 4, |values| {
            values.insert(b"Parent".to_vec(), Value::Ref(r(35)));
        });
        let source = changed(&source, 2, |values| {
            values.insert(
                b"Kids".to_vec(),
                Value::Array(vec![Value::Ref(r(3)), Value::Ref(r(35))]),
            );
        });
        let ctx = context();
        let plan = try_plan(&source, &request(), &ctx).unwrap();
        let candidate = good(&source, &plan, &ctx);
        assert!(!plan.replacements.contains_key(&r(35)));
        assert!(
            flatten::objects_equal(
                source.parsed().object(r(35)).unwrap(),
                candidate.parsed().object(r(35)).unwrap(),
                &source,
                &ctx
            )
            .unwrap()
        );
        assert!(
            !active_objects(&candidate, &BTreeMap::new(), &ctx)
                .unwrap()
                .contains(&r(35))
        );
        let opaque = changed(&source, 34, |values| {
            values.insert(b"PagesNode".to_vec(), Value::Ref(r(35)));
        });
        assert!(try_plan(&opaque, &request(), &context()).is_err());
    }

    #[test]
    fn direct_catalog_form_and_destination_containers_compose_exact_keys() {
        let source = fixture();
        let acro = source.parsed().object(r(23)).unwrap().value.clone();
        let names = source.parsed().object(r(27)).unwrap().value.clone();
        let source = changed(&source, 1, |values| {
            values.insert(b"AcroForm".to_vec(), acro);
            values.insert(b"Names".to_vec(), names);
        });
        let ctx = context();
        let plan = try_plan(&source, &request(), &ctx).unwrap();
        let candidate = good(&source, &plan, &ctx);
        assert_eq!(
            plan.catalog_keys().collect::<Vec<_>>(),
            [b"AcroForm".as_slice(), b"Names"]
        );
        assert!(!plan.allows_catalog_key(b"PageLabels"));
        for number in [23, 27] {
            assert!(
                flatten::objects_equal(
                    source.parsed().object(r(number)).unwrap(),
                    candidate.parsed().object(r(number)).unwrap(),
                    &source,
                    &ctx
                )
                .unwrap()
            );
        }
        let corrupt = changed(&candidate, 1, |values| {
            values.remove(b"Private".as_slice());
            values.insert(b"Undeclared".to_vec(), Value::Bool(true));
        });
        rejected(&source, &corrupt, &plan, &ctx);
    }

    #[test]
    fn bounds_ownership_and_unsupported_destination_forms_fail_closed() {
        let source = fixture();
        for request in [
            PdfPageDeleteRequest {
                page_index: 2,
                ..request()
            },
            PdfPageDeleteRequest {
                excluded_targets: PdfExcludedPageTargetPolicy::RemapToPage { page_index: 1 },
                ..request()
            },
            PdfPageDeleteRequest {
                excluded_targets: PdfExcludedPageTargetPolicy::RemapToPage { page_index: 2 },
                ..request()
            },
            PdfPageDeleteRequest {
                excluded_targets: PdfExcludedPageTargetPolicy::Reject,
                ..request()
            },
        ] {
            assert!(try_plan(&source, &request, &context()).is_err());
        }
        let bad = changed(&source, 10, |values| {
            values.insert(b"P".to_vec(), Value::Ref(r(3)));
        });
        assert!(try_plan(&bad, &request(), &context()).is_err());
        let shared = changed(&source, 3, |values| {
            let Value::Array(annots) = values.get_mut(b"Annots".as_slice()).unwrap() else {
                panic!()
            };
            annots.push(Value::Ref(r(10)));
        });
        assert!(try_plan(&shared, &request(), &context()).is_err());
        let bad = changed(&source, 8, |values| {
            values.insert(b"FT".to_vec(), Value::Name(b"Btn".to_vec()));
        });
        assert!(try_plan(&bad, &request(), &context()).is_err());
        let bad = changed(&source, 18, |values| {
            values.insert(
                b"Dest".to_vec(),
                Value::Array(vec![
                    Value::Ref(r(4)),
                    Value::Name(b"XYZ".to_vec()),
                    Value::Null,
                    Value::Null,
                    Value::Null,
                ]),
            );
        });
        assert!(try_plan(&bad, &request(), &context()).is_err());
        let bad = changed(&source, 26, |values| {
            values.insert(b"Next".to_vec(), Value::Ref(r(26)));
        });
        assert!(try_plan(&bad, &request(), &context()).is_err());
        let request_first = PdfPageDeleteRequest {
            page_index: 0,
            excluded_targets: PdfExcludedPageTargetPolicy::Reject,
        };
        let ctx = context();
        let plan = try_plan(&source, &request_first, &ctx).unwrap();
        let candidate = good(&source, &plan, &ctx);
        assert!(plan.effect.remapped_destinations.is_empty());
        assert!(try_plan(&candidate, &request_first, &context()).is_err());
    }

    #[test]
    fn stale_store_staged_conflicts_cancellation_and_quotas_leave_no_changes() {
        let source = fixture();
        let ctx = context();
        let plan = try_plan(&source, &request(), &ctx).unwrap();
        let altered = changed(&source, 34, |values| {
            values.insert(b"Unrelated".to_vec(), Value::Bool(true));
        });
        assert!(apply(&altered, &plan, &mut BTreeMap::new(), &ctx).is_err());
        let mut conflict = BTreeMap::from([(r(8), source.parsed().object(r(8)).unwrap().clone())]);
        assert!(apply(&source, &plan, &mut conflict, &ctx).is_err());
        assert_eq!(conflict.len(), 1);
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
            let budget = ResourceBudget::new(limits);
            let limited = OperationContext::new(budget.clone(), CancellationToken::new());
            assert!(try_plan(&source, &request(), &limited).is_err());
            assert_eq!(budget.snapshot().live_bytes, 0);
            let spent = budget.snapshot().work_units;
            assert!(spent > 0);
            assert!(try_plan(&source, &request(), &limited).is_err());
            assert!(budget.snapshot().work_units >= spent);
        }
    }
}
