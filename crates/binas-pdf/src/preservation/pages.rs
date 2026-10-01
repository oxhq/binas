//! Stable-identity reorder of a complete single-document page permutation.
//!
//! Intermediate Pages nodes keep their position, child-slot topology and Counts.
//! Leaf references move between slots; moved leaves receive the matching Parent
//! and their ORIGINAL effective inheritable attributes. Catalog, trailer, page
//! resources, annotations, forms and tag identities are not rebuilt. Logical tag
//! order is not physical page order and is deliberately unchanged here.
//!
//! Opaque values remain byte/value-identical in untouched objects; their unknown
//! semantics, rendering and conformance are not validated. This helper neither
//! deletes nor duplicates/imports pages and does not create byte provenance.

use crate::{
    PdfDocument, PdfError,
    object::{IndirectObject, ObjectRef, Value},
    store::ObjectStore,
};
use binas_core::{
    OperationContext, Reservation, SourceId,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub(crate) struct PageReorderPlan {
    pub(crate) replacements: BTreeMap<ObjectRef, IndirectObject>,
    pub(crate) expected_order: Vec<ObjectRef>,
    source: SourceId,
    store: ObjectStore,
    // Computed from source-relative leaf changes before replacements are moved
    // into the enclosing transaction. Same-parent leaves have no permissions.
    leaf_keys: BTreeMap<ObjectRef, u8>,
    // Logical copied values and planned order records. Allocation overhead,
    // spare capacities and shared source/stream payloads are not measured.
    _reservations: Vec<Reservation>,
}

impl PageReorderPlan {
    pub(crate) fn allows_key(&self, page: ObjectRef, key: &[u8]) -> bool {
        self.leaf_keys
            .get(&page)
            .is_some_and(|keys| *keys & leaf_key_bit(key) != 0)
    }
}

const LEAF_KEYS: [&[u8]; 5] = [b"Parent", b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

fn leaf_key_bit(key: &[u8]) -> u8 {
    match key {
        b"Parent" => 1,
        b"Resources" => 2,
        b"MediaBox" => 4,
        b"CropBox" => 8,
        b"Rotate" => 16,
        _ => 0,
    }
}

#[derive(Clone, Copy, Default)]
struct Inherited<'a> {
    resources: Option<&'a Value>,
    media: Option<&'a Value>,
    crop: Option<&'a Value>,
    rotate: Option<&'a Value>,
}
impl<'a> Inherited<'a> {
    fn with_dictionary(mut self, values: &'a BTreeMap<Vec<u8>, Value>) -> Self {
        self.resources = values
            .get(b"Resources".as_slice())
            .filter(|value| !matches!(value, Value::Null))
            .or(self.resources);
        self.media = values
            .get(b"MediaBox".as_slice())
            .filter(|value| !matches!(value, Value::Null))
            .or(self.media);
        self.crop = values
            .get(b"CropBox".as_slice())
            .filter(|value| !matches!(value, Value::Null))
            .or(self.crop);
        self.rotate = values
            .get(b"Rotate".as_slice())
            .filter(|value| !matches!(value, Value::Null))
            .or(self.rotate);
        self
    }
}
struct Slot<'a> {
    page: ObjectRef,
    parent: ObjectRef,
    index: usize,
    inherited: Inherited<'a>,
}
struct Census<'a> {
    slots: Vec<Slot<'a>>,
    nodes: BTreeSet<ObjectRef>,
    _reservations: Vec<Reservation>,
}

pub(crate) fn plan_reorder(
    document: &PdfDocument,
    requested: &[ObjectRef],
    context: &OperationContext,
) -> Result<PageReorderPlan, PdfError> {
    let census = census(document, context)?;
    if requested.len() != census.slots.len() {
        return Err(PdfError::selection(
            "reorder requires every original page exactly once",
        ));
    }
    let expected_set: BTreeSet<_> = census.slots.iter().map(|slot| slot.page).collect();
    let mut seen = BTreeSet::new();
    for page in requested {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if !expected_set.contains(page) || !seen.insert(*page) {
            return Err(PdfError::selection(
                "reorder contains a missing, foreign-generation or duplicate page",
            ));
        }
    }
    let order_bytes = requested
        .len()
        .checked_mul(std::mem::size_of::<ObjectRef>())
        .ok_or_else(|| PdfError::limit("reorder order size overflows"))?;
    let mut reservations = vec![
        context
            .reserve(order_bytes as u64)
            .map_err(PdfError::from_context)?,
    ];
    let by_page: BTreeMap<_, _> = census.slots.iter().map(|slot| (slot.page, slot)).collect();
    let mut replacements = BTreeMap::new();
    for (destination, requested_page) in census.slots.iter().zip(requested) {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if destination.page == *requested_page {
            continue;
        }
        if let std::collections::btree_map::Entry::Vacant(entry) =
            replacements.entry(destination.parent)
        {
            entry.insert(clone_object(
                document.parsed().object(destination.parent)?,
                context,
                &mut reservations,
            )?);
        }
        let parent = replacements
            .get_mut(&destination.parent)
            .expect("inserted parent");
        let Value::Dict(dictionary) = &mut parent.value else {
            unreachable!("validated page tree dictionary")
        };
        let Some(Value::Array(kids)) = dictionary.get_mut(b"Kids".as_slice()) else {
            unreachable!("validated Kids")
        };
        kids[destination.index] = Value::Ref(*requested_page);
        let original = by_page[requested_page];
        if original.parent != destination.parent {
            let mut leaf = clone_object(
                document.parsed().object(*requested_page)?,
                context,
                &mut reservations,
            )?;
            let Value::Dict(dictionary) = &mut leaf.value else {
                unreachable!("validated page dictionary")
            };
            dictionary.insert(b"Parent".to_vec(), Value::Ref(destination.parent));
            let empty_resources = Value::Dict(BTreeMap::new());
            let zero_rotation = Value::Integer(0);
            let media = original.inherited.media.expect("validated MediaBox");
            for (key, value) in [
                (
                    b"Resources".as_slice(),
                    original.inherited.resources.unwrap_or(&empty_resources),
                ),
                (b"MediaBox".as_slice(), media),
                (
                    b"CropBox".as_slice(),
                    original.inherited.crop.unwrap_or(media),
                ),
                (
                    b"Rotate".as_slice(),
                    original.inherited.rotate.unwrap_or(&zero_rotation),
                ),
            ] {
                if dictionary
                    .get(key)
                    .is_none_or(|value| matches!(value, Value::Null))
                {
                    let cost = value_cost(value, context)?
                        .checked_add(key.len())
                        .ok_or_else(|| PdfError::limit("reorder inherited value size overflows"))?;
                    reservations.push(
                        context
                            .reserve(cost as u64)
                            .map_err(PdfError::from_context)?,
                    );
                    dictionary.insert(key.to_vec(), value.clone());
                }
            }
            replacements.insert(*requested_page, leaf);
        }
    }
    let mut leaf_keys = BTreeMap::new();
    for slot in &census.slots {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Some(replacement) = replacements.get(&slot.page) else {
            continue;
        };
        let before = dictionary(&document.parsed().object(slot.page)?.value)?;
        let after = dictionary(&replacement.value)?;
        let mut changed = 0;
        for key in LEAF_KEYS {
            for value in [before.get(key), after.get(key)].into_iter().flatten() {
                let cost = value_cost(value, context)?;
                context
                    .charge_work(cost as u64)
                    .map_err(PdfError::from_context)?;
            }
            if before.get(key) != after.get(key) {
                changed |= leaf_key_bit(key);
            }
        }
        if changed != 0 {
            reservations.push(
                context
                    .reserve(std::mem::size_of::<(ObjectRef, u8)>() as u64)
                    .map_err(PdfError::from_context)?,
            );
            leaf_keys.insert(slot.page, changed);
        }
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(PageReorderPlan {
        replacements,
        expected_order: requested.to_vec(),
        source: document.source_id(),
        store: document.parsed().objects.clone(),
        leaf_keys,
        _reservations: reservations,
    })
}

pub(crate) fn required_checks() -> RequiredChecks {
    RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
        .expect("static reorder checks")
}
const CHECKS: [&str; 3] = [
    "stable_page_order",
    "page_tree_values",
    "effective_inheritance",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.reorder", name).expect("static reorder code")
}

/// Checks page-tree-only effects so other independently checked text, annotation
/// and page-label edits can compose in one transaction. The enclosing transaction
/// must check all other requested/untouched objects, catalog and trailer.
pub(crate) fn verify_reorder(
    original: &PdfDocument,
    candidate: &PdfDocument,
    plan: &PageReorderPlan,
    expected_objects: &BTreeMap<ObjectRef, IndirectObject>,
    geometry: Option<&super::geometry::GeometryPlan>,
    flatten: Option<&crate::forms::flatten::FlattenPlan>,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    if original.source_id() != plan.source
        || candidate.source_id() != plan.source
        || !original.parsed().objects.shares_snapshot_with(&plan.store)
    {
        return Err(PdfError::selection(
            "reorder plan source/object snapshot mismatch",
        ));
    }
    let before = census(original, context)?;
    let after = census(candidate, context)?;
    let order = after
        .slots
        .iter()
        .map(|slot| slot.page)
        .eq(plan.expected_order.iter().copied())
        && before.nodes == after.nodes;
    let mut values =
        source_tree_values(original, candidate, &before, &plan.expected_order, context)?;
    for reference in &before.nodes {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let expected = match expected_objects.get(reference) {
            Some(value) => value,
            None => original.parsed().object(*reference)?,
        };
        let actual = candidate.parsed().object(*reference)?;
        values &= expected.value == actual.value
            && expected.stream.as_deref() == actual.stream.as_deref();
    }
    let by_page: BTreeMap<_, _> = before
        .slots
        .iter()
        .map(|slot| (slot.page, slot.inherited))
        .collect();
    let mut inherited = true;
    for page in &after.slots {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Some(old) = by_page.get(&page.page) else {
            inherited = false;
            continue;
        };
        let resources_transformed =
            flatten.is_some_and(|plan| plan.allows_page_key(page.page, b"Resources"));
        inherited &= if let Some(expected) = geometry.and_then(|plan| plan.expected_for(page.page))
        {
            // Geometry comes from the independent source/request intent, never
            // from staged dictionaries. Declared owner-local flatten resources
            // are independently checked by its required source/effect evidence.
            let empty = Value::Dict(BTreeMap::new());
            (resources_transformed
                || resolved(original, old.resources.unwrap_or(&empty), context)?
                    == resolved(
                        candidate,
                        page.inherited.resources.unwrap_or(&empty),
                        context,
                    )?)
                && super::geometry::read_for(candidate, page.page, context)? == *expected
        } else {
            equal_inherited(
                original,
                *old,
                candidate,
                page.inherited,
                resources_transformed,
                context,
            )?
        };
    }
    Ok(EvidenceReport::new(CHECKS.into_iter().zip([order, values, inherited]).map(|(name, passed)| CheckEvidence { code: code(name), status: if passed { CheckStatus::Passed } else { CheckStatus::Failed }, detail: "Stable identities and exact page-tree/inherited values; logical tag order unchanged; opaque meaning/rendering/conformance not evaluated".into() }).collect()))
}

/// Reorder changes only source leaf slots inside Pages/Kids arrays. The
/// intermediate tree, Counts and every other Pages dictionary entry come from
/// the source, independently of the dictionaries emitted by the write planner.
fn source_tree_values(
    original: &PdfDocument,
    candidate: &PdfDocument,
    before: &Census<'_>,
    requested: &[ObjectRef],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    if before.slots.len() != requested.len() {
        return Ok(false);
    }
    let workspace = before
        .slots
        .len()
        .checked_mul(std::mem::size_of::<((ObjectRef, usize), ObjectRef)>())
        .ok_or_else(|| PdfError::limit("reorder verification slot workspace overflows"))?;
    let _reservation = context
        .reserve(workspace as u64)
        .map_err(PdfError::from_context)?;
    let mut expected_slots = BTreeMap::new();
    for (slot, page) in before.slots.iter().zip(requested) {
        context.charge_work(1).map_err(PdfError::from_context)?;
        expected_slots.insert((slot.parent, slot.index), *page);
    }
    for reference in &before.nodes {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let source = original.parsed().object(*reference)?;
        let source_keys = dictionary(&source.value)?;
        if !matches!(source_keys.get(b"Type".as_slice()), Some(Value::Name(kind)) if kind == b"Pages")
        {
            continue;
        }
        let actual = candidate.parsed().object(*reference)?;
        let actual_keys = dictionary(&actual.value)?;
        if source_keys.len() != actual_keys.len()
            || source.stream.as_deref() != actual.stream.as_deref()
        {
            return Ok(false);
        }
        for (key, source_value) in source_keys {
            context
                .charge_work(key.len() as u64 + 1)
                .map_err(PdfError::from_context)?;
            let Some(actual_value) = actual_keys.get(key) else {
                return Ok(false);
            };
            if key != b"Kids" {
                let cost = value_cost(source_value, context)?;
                context
                    .charge_work(cost as u64)
                    .map_err(PdfError::from_context)?;
                if source_value != actual_value {
                    return Ok(false);
                }
                continue;
            }
            let (Value::Array(source_kids), Value::Array(actual_kids)) =
                (source_value, actual_value)
            else {
                return Ok(false);
            };
            if source_kids.len() != actual_kids.len() {
                return Ok(false);
            }
            for (index, (source_child, actual_child)) in
                source_kids.iter().zip(actual_kids).enumerate()
            {
                context.charge_work(1).map_err(PdfError::from_context)?;
                match expected_slots.get(&(*reference, index)) {
                    Some(page) if actual_child == &Value::Ref(*page) => {}
                    None if actual_child == source_child => {}
                    _ => return Ok(false),
                }
            }
        }
    }
    Ok(true)
}

/// Shared retained-operation gate: Parent links, subtree Counts, unique leaf
/// occurrences and effective inherited values must agree with the page tree.
pub(crate) fn validate_tree(
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<(), PdfError> {
    census(document, context).map(|_| ())
}

/// Visits only validated source Page/Pages identities. Catalog resources and
/// other reachable objects are deliberately outside the page-tree role.
pub(crate) fn visit_tree_objects(
    document: &PdfDocument,
    context: &OperationContext,
    mut visit: impl FnMut(ObjectRef) -> Result<(), PdfError>,
) -> Result<(), PdfError> {
    let tree = census(document, context)?;
    for reference in &tree.nodes {
        context.charge_work(1).map_err(PdfError::from_context)?;
        visit(*reference)?;
    }
    Ok(())
}

fn census<'a>(
    document: &'a PdfDocument,
    context: &OperationContext,
) -> Result<Census<'a>, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    let trailer = dictionary(&document.parsed().trailer)?;
    let root = reference(trailer.get(b"Root".as_slice()), "trailer Root")?;
    let catalog = dictionary(&document.parsed().object(root)?.value)?;
    let pages = reference(catalog.get(b"Pages".as_slice()), "catalog Pages")?;
    let mut result = Census {
        slots: Vec::new(),
        nodes: BTreeSet::new(),
        _reservations: Vec::new(),
    };
    walk(
        document,
        pages,
        None,
        0,
        0,
        Inherited::default(),
        &mut result,
        context,
    )?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn walk<'a>(
    document: &'a PdfDocument,
    current: ObjectRef,
    parent: Option<ObjectRef>,
    index: usize,
    depth: usize,
    inherited: Inherited<'a>,
    result: &mut Census<'a>,
    context: &OperationContext,
) -> Result<usize, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    let limits = &document.parsed().limits;
    if depth > limits.max_parser_depth || result.nodes.len() >= limits.max_container_items {
        return Err(PdfError::limit(
            "reorder page tree exceeds depth/visit limits",
        ));
    }
    if result.nodes.contains(&current) {
        return Err(PdfError::unsafe_rewrite(
            "reorder page tree contains cycle or repeated node",
        ));
    }
    let object = document.parsed().object(current)?;
    if object.stream.is_some() {
        return Err(PdfError::unsafe_rewrite(
            "reorder page tree node cannot be a stream",
        ));
    }
    let values = dictionary(&object.value)?;
    match parent {
        Some(parent) if values.get(b"Parent".as_slice()) != Some(&Value::Ref(parent)) => {
            return Err(PdfError::unsafe_rewrite(
                "reorder page Parent contradicts tree containment",
            ));
        }
        None if values.contains_key(b"Parent".as_slice()) => {
            return Err(PdfError::unsafe_rewrite("root Pages cannot have a Parent"));
        }
        _ => {}
    }
    result._reservations.push(
        context
            .reserve(std::mem::size_of::<ObjectRef>() as u64)
            .map_err(PdfError::from_context)?,
    );
    result.nodes.insert(current);
    let inherited = inherited.with_dictionary(values);
    match values.get(b"Type".as_slice()) {
        Some(Value::Name(name)) if name == b"Page" => {
            let parent = parent.ok_or_else(|| {
                PdfError::unsafe_rewrite("catalog Pages must reference a Pages node")
            })?;
            if result.slots.len() >= limits.max_pages {
                return Err(PdfError::limit("reorder page count exceeds max_pages"));
            }
            validate_inherited(document, inherited, context)?;
            result._reservations.push(
                context
                    .reserve(std::mem::size_of::<Slot<'_>>() as u64)
                    .map_err(PdfError::from_context)?,
            );
            result.slots.push(Slot {
                page: current,
                parent,
                index,
                inherited,
            });
            Ok(1)
        }
        Some(Value::Name(name)) if name == b"Pages" => {
            let Some(Value::Array(kids)) = values.get(b"Kids".as_slice()) else {
                return Err(PdfError::unsafe_rewrite(
                    "reorder Pages requires direct Kids array",
                ));
            };
            if kids.len() > limits.max_container_items {
                return Err(PdfError::limit("reorder Kids exceeds container limit"));
            }
            let Some(Value::Integer(expected)) = values.get(b"Count".as_slice()) else {
                return Err(PdfError::unsafe_rewrite(
                    "reorder Pages requires direct Count",
                ));
            };
            let mut actual = 0usize;
            for (index, kid) in kids.iter().enumerate() {
                let child = reference(Some(kid), "page-tree child")?;
                actual = actual
                    .checked_add(walk(
                        document,
                        child,
                        Some(current),
                        index,
                        depth + 1,
                        inherited,
                        result,
                        context,
                    )?)
                    .ok_or_else(|| PdfError::limit("reorder subtree Count overflows"))?;
            }
            if usize::try_from(*expected).ok() != Some(actual) {
                return Err(PdfError::unsafe_rewrite(
                    "reorder Pages Count does not match descendant pages",
                ));
            }
            Ok(actual)
        }
        _ => Err(PdfError::unsafe_rewrite(
            "reorder tree requires Page or Pages Type",
        )),
    }
}

fn dictionary(value: &Value) -> Result<&BTreeMap<Vec<u8>, Value>, PdfError> {
    if let Value::Dict(values) = value {
        Ok(values)
    } else {
        Err(PdfError::unsafe_rewrite(
            "reorder requires direct dictionary",
        ))
    }
}
fn reference(value: Option<&Value>, label: &str) -> Result<ObjectRef, PdfError> {
    if let Some(Value::Ref(reference)) = value {
        Ok(*reference)
    } else {
        Err(PdfError::unsafe_rewrite(format!(
            "reorder {label} must be an indirect reference"
        )))
    }
}

fn resolved<'a>(
    document: &'a PdfDocument,
    mut value: &'a Value,
    context: &OperationContext,
) -> Result<&'a Value, PdfError> {
    for _ in 0..=document.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if let Value::Ref(reference) = value {
            value = &document.parsed().object(*reference)?.value;
        } else {
            return Ok(value);
        }
    }
    Err(PdfError::limit(
        "reorder inherited reference cycle/depth exceeds limit",
    ))
}
fn validate_inherited(
    document: &PdfDocument,
    inherited: Inherited<'_>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let media = inherited
        .media
        .ok_or_else(|| PdfError::unsafe_rewrite("reorder page lacks effective MediaBox"))?;
    for value in [Some(media), inherited.crop].into_iter().flatten() {
        let Value::Array(values) = resolved(document, value, context)? else {
            return Err(PdfError::unsafe_rewrite("reorder page box must be array"));
        };
        if values.len() != 4
            || values
                .iter()
                .any(|value| !matches!(value, Value::Integer(_) | Value::Real(_)))
            || values
                .iter()
                .any(|value| matches!(value, Value::Real(n) if !n.is_finite()))
        {
            return Err(PdfError::unsafe_rewrite(
                "reorder page box must contain four finite numbers",
            ));
        }
    }
    if let Some(value) = inherited.resources
        && !matches!(resolved(document, value, context)?, Value::Dict(_))
    {
        return Err(PdfError::unsafe_rewrite(
            "reorder Resources must resolve to dictionary",
        ));
    }
    if let Some(value) = inherited.rotate
        && !matches!(resolved(document, value, context)?, Value::Integer(value) if value % 90 == 0)
    {
        return Err(PdfError::unsafe_rewrite(
            "reorder Rotate must be an integer multiple of 90",
        ));
    }
    Ok(())
}
fn equal_inherited(
    left_doc: &PdfDocument,
    left: Inherited<'_>,
    right_doc: &PdfDocument,
    right: Inherited<'_>,
    resources_transformed: bool,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    let empty = Value::Dict(BTreeMap::new());
    let zero = Value::Integer(0);
    let lm = left.media.expect("validated MediaBox");
    let rm = right.media.expect("validated MediaBox");
    if !resources_transformed
        && resolved(left_doc, left.resources.unwrap_or(&empty), context)?
            != resolved(right_doc, right.resources.unwrap_or(&empty), context)?
    {
        return Ok(false);
    }
    for (left, right) in [
        (lm, rm),
        (left.crop.unwrap_or(lm), right.crop.unwrap_or(rm)),
        (left.rotate.unwrap_or(&zero), right.rotate.unwrap_or(&zero)),
    ] {
        if resolved(left_doc, left, context)? != resolved(right_doc, right, context)? {
            return Ok(false);
        }
    }
    Ok(true)
}
fn clone_object(
    object: &IndirectObject,
    context: &OperationContext,
    reservations: &mut Vec<Reservation>,
) -> Result<IndirectObject, PdfError> {
    let cost = value_cost(&object.value, context)?
        .checked_add(std::mem::size_of::<IndirectObject>())
        .ok_or_else(|| PdfError::limit("reorder object clone size overflows"))?;
    reservations.push(
        context
            .reserve(cost as u64)
            .map_err(PdfError::from_context)?,
    );
    Ok(object.clone())
}
fn value_cost(value: &Value, context: &OperationContext) -> Result<usize, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    let mut cost = std::mem::size_of::<Value>();
    match value {
        Value::Name(bytes) | Value::String(bytes) => {
            cost = cost
                .checked_add(bytes.len())
                .ok_or_else(|| PdfError::limit("reorder value size overflows"))?
        }
        Value::Array(values) => {
            for value in values {
                cost = cost
                    .checked_add(value_cost(value, context)?)
                    .ok_or_else(|| PdfError::limit("reorder array size overflows"))?;
            }
        }
        Value::Dict(values) => {
            for (key, value) in values {
                let child = value_cost(value, context)?;
                cost = cost
                    .checked_add(key.len())
                    .and_then(|cost| cost.checked_add(child))
                    .ok_or_else(|| PdfError::limit("reorder dictionary size overflows"))?;
            }
        }
        _ => {}
    }
    Ok(cost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfErrorCode};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};

    fn fixture() -> PdfDocument {
        PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/reorder-structures.pdf"),
                OpenOptions::default(),
            )
            .unwrap()
    }
    fn context(live: u64, work: u64) -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: live,
                max_work_units: work,
            }),
            CancellationToken::new(),
        )
    }
    fn generous() -> OperationContext {
        context(10_000_000, 10_000_000)
    }
    fn page(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn stage(document: &PdfDocument, plan: &PageReorderPlan) -> PdfDocument {
        let mut parsed = document.parsed().clone();
        for (reference, object) in &plan.replacements {
            parsed.objects.insert(*reference, object.clone());
        }
        document.with_parsed(parsed)
    }
    fn mutate(
        document: &PdfDocument,
        reference: ObjectRef,
        key: &[u8],
        value: Value,
    ) -> PdfDocument {
        let mut parsed = document.parsed().clone();
        let object = parsed.objects.get_mut(&reference).unwrap();
        let Value::Dict(values) = &mut object.value else {
            panic!("fixture dictionary")
        };
        values.insert(key.to_vec(), value);
        document.with_parsed(parsed)
    }

    #[test]
    fn nested_reorder_retains_catalog_closures_and_effective_inheritance() {
        let original = fixture();
        let context = generous();
        let requested = [page(6), page(4), page(5)];
        let plan = plan_reorder(&original, &requested, &context).unwrap();
        let candidate = stage(&original, &plan);
        verify_reorder(
            &original,
            &candidate,
            &plan,
            &plan.replacements,
            None,
            None,
            &context,
        )
        .unwrap()
        .validate(&required_checks())
        .unwrap();
        assert_eq!(candidate.page_refs().unwrap(), requested);
        assert_eq!(candidate.parsed().trailer, original.parsed().trailer);
        for (reference, object) in &original.parsed().objects {
            if !plan.replacements.contains_key(reference) {
                assert!(
                    std::ptr::eq(object, candidate.parsed().object(*reference).unwrap()),
                    "untouched object {reference:?}"
                );
            }
        }
        // Page 6 moves from parent 3 into parent 2's first leaf slot. Page 4
        // remains under parent 2; page 5 moves to parent 3. Root topology stays.
        assert_eq!(
            plan.replacements.keys().copied().collect::<Vec<_>>(),
            [page(2), page(3), page(5), page(6)]
        );
        for (output_index, original_index) in [2, 0, 1].into_iter().enumerate() {
            assert_eq!(
                candidate.page_geometry(output_index).unwrap(),
                original.page_geometry(original_index).unwrap()
            );
        }
        assert_eq!(
            crate::read_document_info(&candidate).unwrap(),
            crate::read_document_info(&original).unwrap()
        );
        assert_eq!(
            crate::read_outlines(&candidate).unwrap(),
            crate::read_outlines(&original).unwrap()
        );
        let old_destinations = crate::read_named_destinations(&original).unwrap();
        let new_destinations = crate::read_named_destinations(&candidate).unwrap();
        assert_eq!(old_destinations.len(), new_destinations.len());
        let old_order = original.page_refs().unwrap();
        for (old, new) in old_destinations.iter().zip(&new_destinations) {
            assert_eq!(old.name, new.name);
            let stable_page = old_order[old.page_index];
            assert_eq!(
                new.page_index,
                requested
                    .iter()
                    .position(|reference| *reference == stable_page)
                    .unwrap()
            );
        }
        assert_eq!(
            crate::read_embedded_attachments(&candidate).unwrap(),
            crate::read_embedded_attachments(&original).unwrap()
        );
    }

    #[test]
    fn identity_reorder_has_no_object_deltas_and_flat_nonzero_generation_works() {
        let original = fixture();
        let context = generous();
        let identity = plan_reorder(&original, &[page(4), page(5), page(6)], &context).unwrap();
        assert!(identity.replacements.is_empty());
        // Independent original flat PDF with proper nonzero xref generations.
        let objects = [
            (0, "<< /Type /Catalog /Pages 2 0 R >>"),
            (
                0,
                "<< /Type /Pages /Kids [3 1 R 4 2 R] /Count 2 /MediaBox [0 0 200 300] >>",
            ),
            (1, "<< /Type /Page /Parent 2 0 R >>"),
            (2, "<< /Type /Page /Parent 2 0 R >>"),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, (generation, body)) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(
                format!("{} {generation} obj\n{body}\nendobj\n", index + 1).as_bytes(),
            );
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for (offset, (generation, _)) in offsets.into_iter().zip(objects) {
            bytes.extend_from_slice(format!("{offset:010} {generation:05} n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        let flat = PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap();
        let requested = [
            ObjectRef {
                number: 4,
                generation: 2,
            },
            ObjectRef {
                number: 3,
                generation: 1,
            },
        ];
        let plan = plan_reorder(&flat, &requested, &context).unwrap();
        assert_eq!(plan.replacements.len(), 1);
        let candidate = stage(&flat, &plan);
        assert_eq!(candidate.page_refs().unwrap(), requested);
        verify_reorder(
            &flat,
            &candidate,
            &plan,
            &plan.replacements,
            None,
            None,
            &context,
        )
        .unwrap()
        .validate(&required_checks())
        .unwrap();
    }

    #[test]
    fn inherited_defaults_are_materialized_before_cross_parent_move() {
        let original = fixture();
        let mut parsed = original.parsed().clone();
        for reference in [page(3), page(6), page(7)] {
            let Value::Dict(values) = &mut parsed.objects.get_mut(&reference).unwrap().value else {
                panic!()
            };
            for key in [b"Resources".as_slice(), b"CropBox", b"Rotate"] {
                values.remove(key);
            }
        }
        let document = original.with_parsed(parsed);
        let context = generous();
        let plan = plan_reorder(&document, &[page(6), page(4), page(5)], &context).unwrap();
        let candidate = stage(&document, &plan);
        let values = dictionary(&candidate.parsed().object(page(6)).unwrap().value).unwrap();
        assert_eq!(
            values.get(b"Resources".as_slice()),
            Some(&Value::Dict(BTreeMap::new()))
        );
        assert_eq!(values.get(b"Rotate".as_slice()), Some(&Value::Integer(0)));
        assert_eq!(
            values.get(b"CropBox".as_slice()),
            values.get(b"MediaBox".as_slice())
        );
        verify_reorder(
            &document,
            &candidate,
            &plan,
            &plan.replacements,
            None,
            None,
            &context,
        )
        .unwrap()
        .validate(&required_checks())
        .unwrap();
    }

    #[test]
    fn direct_null_inheritable_entries_are_absent_and_materialized_when_moved() {
        let original = fixture();
        let mut parsed = original.parsed().clone();
        let Value::Dict(values) = &mut parsed.objects.get_mut(&page(6)).unwrap().value else {
            panic!()
        };
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            values.insert(key.to_vec(), Value::Null);
        }
        let document = original.with_parsed(parsed);
        let context = generous();
        let plan = plan_reorder(&document, &[page(6), page(4), page(5)], &context).unwrap();
        let candidate = stage(&document, &plan);
        let values = dictionary(&candidate.parsed().object(page(6)).unwrap().value).unwrap();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            assert!(!matches!(values.get(key), None | Some(Value::Null)));
        }
        verify_reorder(
            &document,
            &candidate,
            &plan,
            &plan.replacements,
            None,
            None,
            &context,
        )
        .unwrap()
        .validate(&required_checks())
        .unwrap();
        assert_eq!(values.get(b"Rotate".as_slice()), Some(&Value::Integer(270)));
    }

    #[test]
    fn malformed_trees_permutations_and_injected_inheritance_loss_fail() {
        let original = fixture();
        let context = generous();
        for requested in [
            vec![page(4)],
            vec![page(4), page(4), page(6)],
            vec![
                page(4),
                page(5),
                ObjectRef {
                    number: 6,
                    generation: 1,
                },
            ],
        ] {
            assert!(plan_reorder(&original, &requested, &context).is_err());
        }
        for malformed in [
            mutate(&original, page(2), b"Count", Value::Integer(99)),
            mutate(&original, page(4), b"Parent", Value::Ref(page(3))),
            mutate(
                &original,
                page(2),
                b"Kids",
                Value::Array(vec![Value::Ref(page(4)), Value::Ref(page(4))]),
            ),
            mutate(
                &original,
                page(2),
                b"Kids",
                Value::Array(vec![Value::Ref(page(7))]),
            ),
        ] {
            assert!(plan_reorder(&malformed, &[page(6), page(4), page(5)], &context).is_err());
        }
        let plan = plan_reorder(&original, &[page(6), page(4), page(5)], &context).unwrap();
        let staged = stage(&original, &plan);
        let damaged = mutate(&staged, page(6), b"Rotate", Value::Integer(90));
        assert!(
            verify_reorder(
                &original,
                &damaged,
                &plan,
                &plan.replacements,
                None,
                None,
                &context
            )
            .unwrap()
            .validate(&required_checks())
            .is_err()
        );
    }

    #[test]
    fn source_pages_keys_reject_self_consistent_private_data_loss() {
        let original = mutate(
            &fixture(),
            page(2),
            b"Private",
            Value::String(b"preserve me".to_vec()),
        );
        let context = generous();
        let mut plan = plan_reorder(&original, &[page(6), page(4), page(5)], &context).unwrap();
        let Value::Dict(keys) = &mut plan.replacements.get_mut(&page(2)).unwrap().value else {
            unreachable!()
        };
        assert!(keys.remove(b"Private".as_slice()).is_some());
        let candidate = stage(&original, &plan);
        // The staged output and expected write dictionary agree, while the
        // independent source-derived tree dictionary contract must reject it.
        assert_eq!(
            candidate.parsed().object(page(2)).unwrap().value,
            plan.replacements[&page(2)].value
        );
        let report = verify_reorder(
            &original,
            &candidate,
            &plan,
            &plan.replacements,
            None,
            None,
            &context,
        )
        .unwrap();
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code == code("page_tree_values")
                    && check.status == CheckStatus::Failed)
        );
        assert!(report.validate(&required_checks()).is_err());
    }

    #[test]
    fn source_child_slots_reject_a_valid_rebalanced_tree_with_same_page_order() {
        let original = fixture();
        let context = generous();
        let requested = [page(6), page(4), page(5)];
        let mut plan = plan_reorder(&original, &requested, &context).unwrap();
        // Honest output puts [6,4] under 2 and [5] under 3. Move 4 to 3 while
        // retaining order and each page's original effective inherited values.
        for (parent, children) in [(page(2), vec![page(6)]), (page(3), vec![page(4), page(5)])] {
            let Value::Dict(keys) = &mut plan.replacements.get_mut(&parent).unwrap().value else {
                unreachable!()
            };
            keys.insert(b"Count".to_vec(), Value::Integer(children.len() as i64));
            keys.insert(
                b"Kids".to_vec(),
                Value::Array(children.into_iter().map(Value::Ref).collect()),
            );
        }
        let mut moved = original.parsed().object(page(4)).unwrap().clone();
        let Value::Dict(keys) = &mut moved.value else {
            unreachable!()
        };
        keys.insert(b"Parent".to_vec(), Value::Ref(page(3)));
        let old_parent = dictionary(&original.parsed().object(page(2)).unwrap().value).unwrap();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            keys.insert(key.to_vec(), old_parent[key].clone());
        }
        plan.replacements.insert(page(4), moved);
        let candidate = stage(&original, &plan);
        validate_tree(&candidate, &context).unwrap();
        assert_eq!(candidate.page_refs().unwrap(), requested);
        for (output, input) in [2, 0, 1].into_iter().enumerate() {
            assert_eq!(
                candidate.page_geometry(output).unwrap(),
                original.page_geometry(input).unwrap()
            );
        }
        let report = verify_reorder(
            &original,
            &candidate,
            &plan,
            &plan.replacements,
            None,
            None,
            &context,
        )
        .unwrap();
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code == code("page_tree_values")
                    && check.status == CheckStatus::Failed)
        );
        assert!(report.validate(&required_checks()).is_err());
    }

    #[test]
    fn reorder_leaf_permissions_match_actual_changes_and_survive_delta_transfer() {
        let original = fixture();
        let context = generous();
        let mut plan = plan_reorder(&original, &[page(6), page(4), page(5)], &context).unwrap();
        for key in LEAF_KEYS {
            assert!(!plan.allows_key(page(4), key), "same-parent page {key:?}");
            assert!(plan.allows_key(page(5), key), "moved page {key:?}");
            assert!(plan.allows_key(page(6), key), "moved page {key:?}");
            assert!(!plan.allows_key(page(2), key), "Pages node {key:?}");
        }
        assert!(!plan.allows_key(page(6), b"Annots"));
        let _transferred = std::mem::take(&mut plan.replacements);
        assert!(plan.allows_key(page(6), b"Parent"));
        assert!(!plan.allows_key(page(4), b"Resources"));
    }

    #[test]
    fn cancellation_limits_and_snapshot_binding_fail_without_source_mutation() {
        let original = fixture();
        let cancelled = generous();
        cancelled.cancellation().cancel();
        assert_eq!(
            plan_reorder(&original, &[page(6), page(4), page(5)], &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        for limited in [context(0, 10_000_000), context(10_000_000, 1)] {
            assert_eq!(
                plan_reorder(&original, &[page(6), page(4), page(5)], &limited)
                    .unwrap_err()
                    .code,
                PdfErrorCode::ResourceLimit
            );
            assert_eq!(limited.budget().snapshot().live_bytes, 0);
        }
        let context = generous();
        let plan = plan_reorder(&original, &[page(6), page(4), page(5)], &context).unwrap();
        let candidate = stage(&original, &plan);
        assert!(
            verify_reorder(
                &fixture(),
                &candidate,
                &plan,
                &plan.replacements,
                None,
                None,
                &context
            )
            .is_err()
        );
        assert_eq!(original.page_refs().unwrap(), [page(4), page(5), page(6)]);
        let work = context.budget().snapshot().work_units;
        drop(plan);
        assert_eq!(context.budget().snapshot().live_bytes, 0);
        assert_eq!(context.budget().snapshot().work_units, work);
    }
}
