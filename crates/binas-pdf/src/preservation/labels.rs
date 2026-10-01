//! Follow-page labels for reorder, duplication, or one-page deletion with a flat tree.
//! Existing tree topology and
//! opaque dictionary entries survive; only label indices, /St and /Limits change.
//!
//! Traversal and rendered label bytes spend the supplied work budget. Dictionary
//! clones, temporary tree/spec vectors and retained expected-label strings are
//! bounded by PDF container/token/page limits but do not yet hold live-memory
//! reservations. This planner does not provide complete memory accounting.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::OperationContext;

use crate::{
    PdfDocument, PdfError,
    document_structures::{PageLabel, PageLabelSpec},
    object::{IndirectObject, ObjectRef, Value},
    parser::ParsedDocument,
};

#[derive(Debug)]
pub(crate) struct LabelReorder {
    pub changes: BTreeMap<ObjectRef, IndirectObject>,
    pub expected_labels: Vec<PageLabel>,
}

type Dictionary = BTreeMap<Vec<u8>, Value>;

struct Node {
    reference: Option<ObjectRef>,
    dictionary: Dictionary,
    children: Vec<Node>,
    entries: usize,
}

struct Entry {
    index: usize,
    reference: Option<ObjectRef>,
    dictionary: Dictionary,
    spec: PageLabelSpec,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Mapping {
    Permutation,
    Occurrences,
    /// One removed source page, with the remaining pages in their source order.
    DeleteOne,
}

fn valid_mapping(
    source_count: usize,
    old_indices: &[usize],
    mapping: Mapping,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    match mapping {
        Mapping::Permutation if source_count != old_indices.len() => return Ok(false),
        Mapping::DeleteOne
            if source_count <= 1 || old_indices.len().checked_add(1) != Some(source_count) =>
        {
            return Ok(false);
        }
        _ => {}
    }
    let _scratch = context
        .reserve(source_count as u64)
        .map_err(PdfError::from_context)?;
    let mut seen = vec![false; source_count];
    let mut previous = None;
    for &index in old_indices {
        charge(context, 1)?;
        if index >= source_count
            || (mapping != Mapping::Occurrences && seen[index])
            || (mapping == Mapping::DeleteOne && previous.is_some_and(|last| last >= index))
        {
            return Ok(false);
        }
        seen[index] = true;
        previous = Some(index);
    }
    charge(context, seen.len())?;
    Ok(if mapping == Mapping::DeleteOne {
        seen.iter().filter(|&&present| !present).count() == 1
    } else {
        seen.iter().all(|&present| present)
    })
}

fn charge(context: &OperationContext, units: usize) -> Result<(), PdfError> {
    context
        .charge_work(units as u64)
        .map_err(PdfError::from_context)
}

fn dictionary(value: &Value) -> Result<&Dictionary, PdfError> {
    match value {
        Value::Dict(value) => Ok(value),
        _ => Err(PdfError::syntax(
            "page label object must be a dictionary",
            0,
        )),
    }
}

fn present<'a>(dictionary: &'a Dictionary, key: &[u8]) -> Option<&'a Value> {
    dictionary
        .get(key)
        .filter(|value| !matches!(value, Value::Null))
}

fn resolve<'a>(
    parsed: &'a ParsedDocument,
    value: &'a Value,
) -> Result<(&'a Dictionary, Option<ObjectRef>), PdfError> {
    let resolved = match value {
        Value::Ref(reference) => {
            let object = parsed.object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported(
                    "page label dictionaries must not be streams",
                ));
            }
            Ok((dictionary(&object.value)?, Some(*reference)))
        }
        _ => Ok((dictionary(value)?, None)),
    }?;
    if matches!(resolved.0.get(b"Type".as_slice()), Some(Value::Name(kind)) if matches!(kind.as_slice(), b"Page" | b"Pages" | b"Catalog"))
    {
        return Err(PdfError::unsafe_rewrite(
            "page label dictionary aliases a page or catalog role",
        ));
    }
    if let (Some(reference), Value::Dict(trailer)) = (resolved.1, &parsed.trailer)
        && trailer.get(b"Root".as_slice()) == Some(&Value::Ref(reference))
    {
        return Err(PdfError::unsafe_rewrite(
            "page label dictionary aliases the catalog",
        ));
    }
    Ok(resolved)
}

fn load(
    parsed: &ParsedDocument,
    value: &Value,
    depth: usize,
    seen: &mut BTreeSet<ObjectRef>,
    entries: &mut Vec<Entry>,
    context: &OperationContext,
) -> Result<Node, PdfError> {
    charge(context, 1)?;
    if depth > parsed.limits.max_parser_depth || seen.len() >= parsed.limits.max_container_items {
        return Err(PdfError::limit("page label tree exceeds traversal limits"));
    }
    let (dict, reference) = resolve(parsed, value)?;
    if reference.is_some_and(|reference| !seen.insert(reference)) {
        return Err(PdfError::syntax(
            "cycle or shared node in page label tree",
            0,
        ));
    }
    let begin = entries.len();
    let mut children = Vec::new();
    match (present(dict, b"Nums"), present(dict, b"Kids")) {
        (Some(Value::Array(nums)), None) => {
            if !nums.len().is_multiple_of(2) {
                return Err(PdfError::syntax("page label /Nums must contain pairs", 0));
            }
            if nums.len() > parsed.limits.max_container_items {
                return Err(PdfError::limit("page label /Nums exceeds container limit"));
            }
            for pair in nums.chunks_exact(2) {
                charge(context, 1)?;
                let index = match pair[0] {
                    Value::Integer(index) if index >= 0 => usize::try_from(index)
                        .map_err(|_| PdfError::limit("page label index exceeds usize"))?,
                    _ => return Err(PdfError::syntax("invalid page label index", 0)),
                };
                if entries.last().is_some_and(|entry| entry.index >= index) {
                    return Err(PdfError::syntax(
                        "page label indices must be strictly ordered and unique",
                        0,
                    ));
                }
                if entries.len() >= parsed.limits.max_container_items {
                    return Err(PdfError::limit("page label entries exceed container limit"));
                }
                let (raw, reference) = resolve(parsed, &pair[1])?;
                let mut normalized = raw.clone();
                for key in [b"S".as_slice(), b"P", b"St"] {
                    if normalized.get(key) == Some(&Value::Null) {
                        normalized.remove(key);
                    }
                }
                entries.push(Entry {
                    index,
                    reference,
                    dictionary: raw.clone(),
                    spec: PageLabelSpec::parse(parsed, &Value::Dict(normalized))?,
                });
            }
        }
        (None, Some(Value::Array(kids))) => {
            if kids.len() > parsed.limits.max_container_items {
                return Err(PdfError::limit("page label /Kids exceeds container limit"));
            }
            for kid in kids {
                if !matches!(kid, Value::Ref(_)) {
                    return Err(PdfError::syntax("page label children must be indirect", 0));
                }
                let child = load(parsed, kid, depth + 1, seen, entries, context)?;
                if child.entries == 0 {
                    return Err(PdfError::syntax("page label child is empty", 0));
                }
                children.push(child);
            }
        }
        (None, None) => {}
        _ => return Err(PdfError::syntax("invalid page label /Nums or /Kids", 0)),
    }
    let count = entries.len() - begin;
    if let Some(limits) = present(dict, b"Limits") {
        let expected = if count == 0 {
            None
        } else {
            Some(Value::Array(vec![
                Value::Integer(entries[begin].index as i64),
                Value::Integer(entries.last().unwrap().index as i64),
            ]))
        };
        if expected.as_ref() != Some(limits) {
            return Err(PdfError::syntax(
                "page label /Limits do not match entries",
                0,
            ));
        }
    }
    Ok(Node {
        reference,
        dictionary: dict.clone(),
        children,
        entries: count,
    })
}

fn write(
    mut node: Node,
    nums: &[(usize, Value)],
    parsed: &ParsedDocument,
    changes: &mut BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<Value, PdfError> {
    charge(context, 1)?;
    if node.children.is_empty() {
        let length = nums
            .len()
            .checked_mul(2)
            .ok_or_else(|| PdfError::limit("page label array overflows"))?;
        if length > parsed.limits.max_container_items {
            return Err(PdfError::limit(
                "reordered page label leaf exceeds container limit",
            ));
        }
        let mut values = Vec::with_capacity(length);
        for (index, spec) in nums {
            charge(context, 1)?;
            values.extend([Value::Integer(*index as i64), spec.clone()]);
        }
        node.dictionary
            .insert(b"Nums".to_vec(), Value::Array(values));
    } else {
        let last = node.children.len() - 1;
        let mut position = 0;
        let mut kids = Vec::with_capacity(node.children.len());
        for (index, child) in node.children.into_iter().enumerate() {
            let end = if index == last {
                nums.len()
            } else {
                position + child.entries
            };
            kids.push(write(
                child,
                &nums[position..end],
                parsed,
                changes,
                context,
            )?);
            position = end;
        }
        node.dictionary.insert(b"Kids".to_vec(), Value::Array(kids));
    }
    if present(&node.dictionary, b"Limits").is_some() {
        node.dictionary.insert(
            b"Limits".to_vec(),
            Value::Array(vec![
                Value::Integer(nums[0].0 as i64),
                Value::Integer(nums.last().unwrap().0 as i64),
            ]),
        );
    }
    let value = Value::Dict(node.dictionary);
    if let Some(reference) = node.reference {
        let original = parsed.object(reference)?;
        if value != original.value {
            let mut object = original.clone();
            object.value = value;
            changes.insert(reference, object);
        }
        Ok(Value::Ref(reference))
    } else {
        Ok(value)
    }
}

pub(crate) fn plan_reordered_labels(
    document: &PdfDocument,
    old_indices: &[usize],
    context: &OperationContext,
) -> Result<LabelReorder, PdfError> {
    plan_labels(document, old_indices, Mapping::Permutation, context)
}

/// Each output occurrence inherits the explicit label of its selected source
/// page. Every source page must occur at least once; removal is outside this
/// helper's topology-preserving scope. The caller independently verifies the map.
/// When PageLabels is absent, ordinary output physical numbering remains absent
/// from the catalog rather than creating explicit duplicate numbers.
pub(crate) fn plan_occurrence_labels(
    document: &PdfDocument,
    old_indices: &[usize],
    context: &OperationContext,
) -> Result<LabelReorder, PdfError> {
    plan_labels(document, old_indices, Mapping::Occurrences, context)
}

/// Retained labels after deleting exactly one page. Nonempty flat number trees
/// keep their root identity and opaque keys; nested-tree shrink needs its own
/// topology policy. Absent or empty labels keep ordinary physical numbering.
pub(crate) fn plan_deleted_labels(
    document: &PdfDocument,
    old_indices: &[usize],
    context: &OperationContext,
) -> Result<LabelReorder, PdfError> {
    plan_labels(document, old_indices, Mapping::DeleteOne, context)
}

fn plan_labels(
    document: &PdfDocument,
    old_indices: &[usize],
    mapping: Mapping,
    context: &OperationContext,
) -> Result<LabelReorder, PdfError> {
    charge(context, 1)?;
    let parsed = document.parsed();
    if old_indices.len() > parsed.limits.max_pages
        || old_indices.len() > parsed.limits.max_container_items
    {
        return Err(PdfError::limit("page label permutation exceeds limits"));
    }
    charge(context, parsed.objects.len())?;
    let source_count = document.page_count()?;
    if !valid_mapping(source_count, old_indices, mapping, context)? {
        return Err(PdfError::unsafe_rewrite(
            "page label mapping is invalid for its declared operation",
        ));
    }
    let Value::Ref(catalog_ref) = dictionary(&parsed.trailer)?
        .get(b"Root".as_slice())
        .ok_or_else(|| PdfError::syntax("missing catalog", 0))?
    else {
        return Err(PdfError::unsupported(
            "page labels require an indirect catalog",
        ));
    };
    let catalog = dictionary(&parsed.object(*catalog_ref)?.value)?;
    let mut result = LabelReorder {
        changes: BTreeMap::new(),
        expected_labels: (0..old_indices.len())
            .map(|page_index| PageLabel {
                page_index,
                label: (page_index + 1).to_string(),
            })
            .collect(),
    };
    let Some(root) = present(catalog, b"PageLabels") else {
        return Ok(result);
    };
    let mut entries = Vec::new();
    let tree = load(parsed, root, 0, &mut BTreeSet::new(), &mut entries, context)?;
    if mapping == Mapping::DeleteOne && present(&tree.dictionary, b"Kids").is_some() {
        return Err(PdfError::unsupported(
            "page deletion currently requires flat page labels",
        ));
    }
    if entries.is_empty() {
        return Ok(result);
    }
    if entries.last().unwrap().index >= source_count {
        return Err(PdfError::syntax(
            "page label range begins beyond document pages",
            0,
        ));
    }
    let identity = old_indices.iter().copied().eq(0..source_count);
    let mut nums = Vec::with_capacity(if identity { 0 } else { old_indices.len() });
    for (new_index, &old_index) in old_indices.iter().enumerate() {
        charge(context, 1)?;
        let active = entries.partition_point(|entry| entry.index <= old_index);
        let (mut raw, mut spec, offset) = if active == 0 {
            (
                BTreeMap::from([(b"S".to_vec(), Value::Name(b"D".to_vec()))]),
                PageLabelSpec {
                    style: Some(crate::document_structures::PageLabelStyle::Decimal),
                    prefix: String::new(),
                    start: 1,
                },
                old_index,
            )
        } else {
            let entry = &entries[active - 1];
            (
                entry.dictionary.clone(),
                entry.spec.clone(),
                old_index - entry.index,
            )
        };
        if spec.style.is_some() {
            spec.start = spec
                .start
                .checked_add(offset)
                .ok_or_else(|| PdfError::limit("page label number overflows"))?;
        }
        let label = spec.label(0, parsed.limits.max_token_bytes)?;
        charge(context, label.len())?;
        result.expected_labels[new_index].label = label;
        if identity {
            continue;
        }
        if spec.style.is_some() {
            raw.insert(
                b"St".to_vec(),
                Value::Integer(
                    i64::try_from(spec.start)
                        .map_err(|_| PdfError::limit("page label /St exceeds i64"))?,
                ),
            );
        }
        nums.push((new_index, Value::Dict(raw)));
    }
    if identity {
        return Ok(result);
    }
    let value = write(tree, &nums, parsed, &mut result.changes, context)?;
    if &value != root {
        let mut object = parsed.object(*catalog_ref)?.clone();
        let mut catalog = catalog.clone();
        catalog.insert(b"PageLabels".to_vec(), value);
        object.value = Value::Dict(catalog);
        result.changes.insert(*catalog_ref, object);
    }
    Ok(result)
}

/// Checks the source label tree and requested source-page permutation directly.
/// Staged replacement objects and rendered labels are not preservation evidence.
/// Existing `load` workspace remains bounded by PDF limits, as in the planner;
/// only the permutation scratch has an explicit live reservation here.
pub(crate) fn verify(
    original: &PdfDocument,
    candidate: &PdfDocument,
    old_indices: &[usize],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    verify_catalog_scope(
        original,
        candidate,
        old_indices,
        &[],
        Mapping::Permutation,
        context,
    )
}

/// A composed flatten transaction separately requires source-derived AcroForm
/// registration evidence. Only that declared catalog key is outside label scope.
pub(crate) fn verify_with_acroform_change(
    original: &PdfDocument,
    candidate: &PdfDocument,
    old_indices: &[usize],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    verify_catalog_scope(
        original,
        candidate,
        old_indices,
        &[b"AcroForm"],
        Mapping::Permutation,
        context,
    )
}

pub(crate) fn verify_occurrence_mapping(
    original: &PdfDocument,
    candidate: &PdfDocument,
    old_indices: &[usize],
    acroform_transformed: bool,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    verify_catalog_scope(
        original,
        candidate,
        old_indices,
        if acroform_transformed {
            &[b"AcroForm"]
        } else {
            &[]
        },
        Mapping::Occurrences,
        context,
    )
}

/// Additional catalog keys are checked independently by the deletion planner.
/// This helper retains the exact source label root/specification identities and
/// raw dictionaries through the requested one-page removal.
pub(crate) fn verify_deleted_labels(
    original: &PdfDocument,
    candidate: &PdfDocument,
    old_indices: &[usize],
    other_catalog_keys: &[&[u8]],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    verify_catalog_scope(
        original,
        candidate,
        old_indices,
        other_catalog_keys,
        Mapping::DeleteOne,
        context,
    )
}

fn verify_catalog_scope(
    original: &PdfDocument,
    candidate: &PdfDocument,
    old_indices: &[usize],
    other_keys: &[&[u8]],
    mapping: Mapping,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    charge(context, 1)?;
    let source = original.parsed();
    let actual = candidate.parsed();
    if original.source_id() != candidate.source_id() {
        return Ok(false);
    }
    if old_indices.len() > source.limits.max_pages
        || old_indices.len() > source.limits.max_container_items
    {
        return Err(PdfError::limit(
            "page label verification permutation exceeds limits",
        ));
    }
    charge(context, source.objects.len())?;
    charge(context, actual.objects.len())?;
    let source_count = original.page_count()?;
    if candidate.page_count()? != old_indices.len() {
        return Ok(false);
    }
    if !valid_mapping(source_count, old_indices, mapping, context)? {
        return Ok(false);
    }
    let Some(Value::Ref(catalog_ref)) = dictionary(&source.trailer)?.get(b"Root".as_slice()) else {
        return Err(PdfError::unsupported(
            "page labels require an indirect catalog",
        ));
    };
    if dictionary(&actual.trailer)?.get(b"Root".as_slice()) != Some(&Value::Ref(*catalog_ref)) {
        return Ok(false);
    }
    let before_object = source.object(*catalog_ref)?;
    let after_object = actual.object(*catalog_ref)?;
    if before_object.stream.is_some() || after_object.stream.is_some() {
        return Ok(false);
    }
    let before_catalog = dictionary(&before_object.value)?;
    let after_catalog = dictionary(&after_object.value)?;
    let mut direct_keys = other_keys.to_vec();
    direct_keys.push(b"PageLabels");
    let Some(before_root) = present(before_catalog, b"PageLabels") else {
        return same_dictionary(before_catalog, after_catalog, other_keys, context);
    };
    let Some(after_root) = present(after_catalog, b"PageLabels") else {
        return Ok(false);
    };
    let catalog_matches = match (before_root, after_root) {
        (Value::Ref(before), Value::Ref(after)) if before == after => {
            same_dictionary(before_catalog, after_catalog, other_keys, context)?
        }
        (Value::Dict(_), Value::Dict(_)) => {
            same_dictionary(before_catalog, after_catalog, &direct_keys, context)?
        }
        _ => false,
    };
    if !catalog_matches {
        return Ok(false);
    }
    let mut before_entries = Vec::new();
    let before_tree = load(
        source,
        before_root,
        0,
        &mut BTreeSet::new(),
        &mut before_entries,
        context,
    )?;
    let mut after_entries = Vec::new();
    let after_tree = load(
        actual,
        after_root,
        0,
        &mut BTreeSet::new(),
        &mut after_entries,
        context,
    )?;
    if mapping == Mapping::DeleteOne
        && (present(&before_tree.dictionary, b"Kids").is_some()
            || present(&after_tree.dictionary, b"Kids").is_some())
    {
        return Err(PdfError::unsupported(
            "page deletion currently requires flat page labels",
        ));
    }
    if before_entries
        .last()
        .is_some_and(|entry| entry.index >= source_count)
    {
        return Err(PdfError::syntax(
            "page label range begins beyond document pages",
            0,
        ));
    }
    // Original indirect label specifications remain objects even when reordered
    // /Nums entries inline their dictionaries. Do not authorize mutating them.
    for entry in &before_entries {
        charge(context, 1)?;
        if let Some(reference) = entry.reference {
            let before = source.object(reference)?;
            let after = actual.object(reference)?;
            if after.stream.is_some() || !same_value(&before.value, &after.value, context)? {
                return Ok(false);
            }
        }
    }
    let identity = old_indices.iter().copied().eq(0..source_count);
    if identity || before_entries.is_empty() {
        return same_tree(&before_tree, &after_tree, None, context);
    }
    if after_entries.len() != old_indices.len() {
        return Ok(false);
    }
    if mapping == Mapping::DeleteOne {
        let permitted: &[&[u8]] = if present(&before_tree.dictionary, b"Limits").is_some() {
            &[b"Nums", b"Limits"]
        } else {
            &[b"Nums"]
        };
        if before_tree.reference != after_tree.reference
            || !same_dictionary(
                &before_tree.dictionary,
                &after_tree.dictionary,
                permitted,
                context,
            )?
            || (present(&before_tree.dictionary, b"Limits").is_some()
                && present(&after_tree.dictionary, b"Limits").is_none())
        {
            return Ok(false);
        }
    } else {
        let Some(extra) = old_indices.len().checked_sub(before_entries.len()) else {
            return Ok(false);
        };
        if !same_tree(&before_tree, &after_tree, Some(extra), context)? {
            return Ok(false);
        }
    }
    for (new_index, (&old_index, entry)) in old_indices.iter().zip(&after_entries).enumerate() {
        charge(context, 1)?;
        if entry.index != new_index {
            return Ok(false);
        }
        charge(
            context,
            (usize::BITS - before_entries.len().leading_zeros()) as usize,
        )?;
        let active = before_entries.partition_point(|source| source.index <= old_index);
        if active == 0 {
            // Before the first explicit range, source page numbers are decimal.
            let start = old_index
                .checked_add(1)
                .and_then(|value| i64::try_from(value).ok())
                .ok_or_else(|| PdfError::limit("page label default start overflows"))?;
            if entry.dictionary.len() != 2
                || !matches!(entry.dictionary.get(b"S".as_slice()), Some(Value::Name(name)) if name == b"D")
                || entry.dictionary.get(b"St".as_slice()) != Some(&Value::Integer(start))
            {
                return Ok(false);
            }
        } else {
            let before = &before_entries[active - 1];
            if before.spec.style.is_some() {
                let start = before
                    .spec
                    .start
                    .checked_add(old_index - before.index)
                    .and_then(|value| i64::try_from(value).ok())
                    .ok_or_else(|| PdfError::limit("page label mapped start overflows"))?;
                if entry.dictionary.get(b"St".as_slice()) != Some(&Value::Integer(start))
                    || !same_dictionary(&before.dictionary, &entry.dictionary, &[b"St"], context)?
                {
                    return Ok(false);
                }
            } else if !same_dictionary(&before.dictionary, &entry.dictionary, &[], context)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Visits catalog, source label-tree nodes and original indirect label specs.
/// Opaque referenced objects are not label roles. The existing bounded loader
/// workspace has the same explicit accounting limits as label planning.
pub(crate) fn visit_tree_objects(
    document: &PdfDocument,
    context: &OperationContext,
    mut visit: impl FnMut(ObjectRef) -> Result<(), PdfError>,
) -> Result<(), PdfError> {
    charge(context, 1)?;
    let parsed = document.parsed();
    let Some(Value::Ref(catalog)) = dictionary(&parsed.trailer)?.get(b"Root".as_slice()) else {
        return Err(PdfError::unsupported(
            "page labels require an indirect catalog",
        ));
    };
    visit(*catalog)?;
    let catalog = dictionary(&parsed.object(*catalog)?.value)?;
    let Some(root) = present(catalog, b"PageLabels") else {
        return Ok(());
    };
    let mut entries = Vec::new();
    let mut nodes = BTreeSet::new();
    let _tree = load(parsed, root, 0, &mut nodes, &mut entries, context)?;
    for reference in nodes {
        charge(context, 1)?;
        visit(reference)?;
    }
    for entry in entries {
        charge(context, 1)?;
        if let Some(reference) = entry.reference {
            visit(reference)?;
        }
    }
    Ok(())
}

/// `extra` is present only on the final-child path. Every other subtree retains
/// its source entry capacity; the final leaf absorbs the expanded page entries.
fn same_tree(
    before: &Node,
    after: &Node,
    extra: Option<usize>,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    charge(context, 1)?;
    if before.reference != after.reference || before.children.len() != after.children.len() {
        return Ok(false);
    }
    let Some(extra) = extra else {
        if before.entries != after.entries
            || !same_dictionary(&before.dictionary, &after.dictionary, &[], context)?
        {
            return Ok(false);
        }
        for (before, after) in before.children.iter().zip(&after.children) {
            if !same_tree(before, after, None, context)? {
                return Ok(false);
            }
        }
        return Ok(true);
    };
    if before.entries.checked_add(extra) != Some(after.entries) {
        return Ok(false);
    }
    let limits_change = present(&before.dictionary, b"Limits").is_some();
    if limits_change && present(&after.dictionary, b"Limits").is_none() {
        return Ok(false);
    }
    let permitted: &[&[u8]] = match (before.children.is_empty(), limits_change) {
        (true, true) => &[b"Nums", b"Limits"],
        (true, false) => &[b"Nums"],
        (false, true) => &[b"Limits"],
        (false, false) => &[],
    };
    if !same_dictionary(&before.dictionary, &after.dictionary, permitted, context)? {
        return Ok(false);
    }
    let last = before.children.len().saturating_sub(1);
    for (index, (before, after)) in before.children.iter().zip(&after.children).enumerate() {
        // Some(0) still permits requested /Nums and /Limits changes; None means
        // an identity operation and requires every original value unchanged.
        if !same_tree(
            before,
            after,
            Some(if index == last { extra } else { 0 }),
            context,
        )? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn same_dictionary(
    before: &Dictionary,
    after: &Dictionary,
    permitted: &[&[u8]],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    charge(context, before.len())?;
    charge(context, after.len())?;
    let include = |key: &&Vec<u8>| !permitted.contains(&key.as_slice());
    let mut before = before.iter().filter(|(key, _)| include(key));
    let mut after = after.iter().filter(|(key, _)| include(key));
    loop {
        match (before.next(), after.next()) {
            (None, None) => return Ok(true),
            (Some((before_key, before)), Some((after_key, after))) => {
                charge(context, before_key.len())?;
                charge(context, after_key.len())?;
                if before_key != after_key || !same_value(before, after, context)? {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
}

fn same_value(before: &Value, after: &Value, context: &OperationContext) -> Result<bool, PdfError> {
    charge(context, 1)?;
    match (before, after) {
        (Value::Dict(before), Value::Dict(after)) => same_dictionary(before, after, &[], context),
        (Value::Array(before), Value::Array(after)) => {
            if before.len() != after.len() {
                return Ok(false);
            }
            for (before, after) in before.iter().zip(after) {
                if !same_value(before, after, context)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Name(before), Value::Name(after))
        | (Value::String(before), Value::String(after)) => {
            charge(context, before.len())?;
            charge(context, after.len())?;
            Ok(before == after)
        }
        _ => Ok(before == after),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfErrorCode};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};

    fn context(work: u64) -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: u64::MAX,
                max_work_units: work,
            }),
            CancellationToken::new(),
        )
    }

    fn document(labels: &str, extra: &[&str]) -> PdfDocument {
        let mut objects = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {labels} >>"),
            "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R] /Count 4 >>".into(),
        ];
        for _ in 0..4 {
            objects.push("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".into());
        }
        objects.extend(extra.iter().map(|s| s.to_string()));
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![0];
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
        );
        for offset in offsets.iter().skip(1) {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len()
            )
            .as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    fn key(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }

    fn delete_for_labels(source: &PdfDocument, removed: usize) -> PdfDocument {
        let indices: Vec<_> = (0..4).filter(|&index| index != removed).collect();
        let labels = plan_deleted_labels(source, &indices, &context(u64::MAX)).unwrap();
        let mut parsed = source.parsed().clone();
        let Value::Dict(pages) = &mut parsed.objects.get_mut(&key(2)).unwrap().value else {
            panic!()
        };
        pages.insert(b"Count".to_vec(), Value::Integer(3));
        pages.insert(
            b"Kids".to_vec(),
            Value::Array(
                indices
                    .iter()
                    .map(|&index| Value::Ref(key(index as u32 + 3)))
                    .collect(),
            ),
        );
        for (reference, object) in labels.changes {
            parsed.objects.insert(reference, object);
        }
        source.with_parsed(parsed)
    }

    #[test]
    fn deleted_page_flat_labels_preserve_raw_root_specs_and_retained_names() {
        let source = document(
            "/PageLabels 7 0 R",
            &[
                "<< /Nums [0 8 0 R 2 << /P (Only) /Private (prefix) >>] /Limits [0 2] /Private (root) >>",
                "<< /S /D /P (P) /St 4 /Private (spec) >>",
            ],
        );
        let candidate = delete_for_labels(&source, 1);
        assert_eq!(
            crate::read_page_labels(&candidate)
                .unwrap()
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["P4", "Only", "Only"]
        );
        assert!(
            verify_deleted_labels(&source, &candidate, &[0, 2, 3], &[], &context(u64::MAX))
                .unwrap()
        );
        assert_eq!(
            source.parsed().object(key(8)).unwrap().value,
            candidate.parsed().object(key(8)).unwrap().value
        );
        assert_eq!(
            dictionary(&candidate.parsed().object(key(7)).unwrap().value)
                .unwrap()
                .get(b"Limits".as_slice()),
            Some(&Value::Array(vec![Value::Integer(0), Value::Integer(2)]))
        );
        for number in [7, 8] {
            let changed = mutate_dictionary(&candidate, number, |values| {
                values.remove(b"Private".as_slice());
            });
            assert!(
                !verify_deleted_labels(&source, &changed, &[0, 2, 3], &[], &context(u64::MAX))
                    .unwrap()
            );
        }
        let changed = mutate_dictionary(&candidate, 7, |values| {
            let Value::Array(nums) = values.get_mut(b"Nums".as_slice()).unwrap() else {
                panic!()
            };
            let Value::Dict(spec) = &mut nums[3] else {
                panic!()
            };
            spec.insert(b"Private".to_vec(), Value::String(b"forged".to_vec()));
        });
        // Visible labels alone cannot establish preservation of opaque spec data.
        assert_eq!(
            crate::read_page_labels(&changed).unwrap(),
            crate::read_page_labels(&candidate).unwrap()
        );
        assert!(
            !verify_deleted_labels(&source, &changed, &[0, 2, 3], &[], &context(u64::MAX)).unwrap()
        );
        let changed = mutate_dictionary(&candidate, 7, |values| {
            values.remove(b"Limits".as_slice());
        });
        assert!(
            !verify_deleted_labels(&source, &changed, &[0, 2, 3], &[], &context(u64::MAX)).unwrap()
        );
    }

    #[test]
    fn deleted_page_direct_and_absent_labels_keep_their_declared_numbering() {
        for raw in [
            "",
            "/PageLabels null",
            "/PageLabels << /Nums [] /Private (empty) >>",
            "/PageLabels << /Nums [0 << /S /D /P (P) /St 4 >>] /Limits [0 0] /Private (root) >>",
        ] {
            let source = document(raw, &[]);
            for removed in [0, 3] {
                let candidate = delete_for_labels(&source, removed);
                let indices: Vec<_> = (0..4).filter(|&index| index != removed).collect();
                assert!(
                    verify_deleted_labels(&source, &candidate, &indices, &[], &context(u64::MAX))
                        .unwrap()
                );
                let labels = crate::read_page_labels(&candidate).unwrap();
                let expected: Vec<_> = if raw.contains("/St 4") {
                    indices
                        .iter()
                        .map(|index| format!("P{}", index + 4))
                        .collect()
                } else {
                    (1..=3).map(|number| number.to_string()).collect()
                };
                assert_eq!(
                    labels.iter().map(|label| &label.label).collect::<Vec<_>>(),
                    expected.iter().collect::<Vec<_>>()
                );
                let changed = mutate_dictionary(&candidate, 1, |values| {
                    values.insert(b"Private".to_vec(), Value::Bool(true));
                });
                assert!(
                    !verify_deleted_labels(&source, &changed, &indices, &[], &context(u64::MAX))
                        .unwrap()
                );
            }
        }
    }

    #[test]
    fn deleted_page_label_maps_reject_nested_trees_invalid_removals_and_limits() {
        let source = document("/PageLabels << /Nums [0 << /S /D >>] >>", &[]);
        for indices in [
            vec![],
            vec![0, 1],
            vec![0, 1, 2, 3],
            vec![0, 0, 2],
            vec![2, 1, 0],
            vec![0, 1, 4],
        ] {
            assert!(plan_deleted_labels(&source, &indices, &context(u64::MAX)).is_err());
        }
        let nested = nested_labels();
        assert_eq!(
            plan_deleted_labels(&nested, &[0, 2, 3], &context(u64::MAX))
                .unwrap_err()
                .code,
            crate::PdfErrorCode::UnsupportedFeature
        );
        assert_eq!(
            plan_deleted_labels(&source, &[0, 2, 3], &context(1))
                .unwrap_err()
                .code,
            crate::PdfErrorCode::ResourceLimit
        );
        let cancelled = context(u64::MAX);
        cancelled.cancellation().cancel();
        assert_eq!(
            plan_deleted_labels(&source, &[0, 2, 3], &cancelled)
                .unwrap_err()
                .code,
            crate::PdfErrorCode::Cancelled
        );
        let candidate = delete_for_labels(&source, 1);
        assert!(
            !verify_deleted_labels(&source, &candidate, &[0, 0, 3], &[], &context(u64::MAX))
                .unwrap()
        );
        assert!(
            !verify_deleted_labels(&source, &candidate, &[0, 3, 2], &[], &context(u64::MAX))
                .unwrap()
        );
        assert_eq!(
            verify_deleted_labels(&source, &candidate, &[0, 2, 3], &[], &context(1))
                .unwrap_err()
                .code,
            crate::PdfErrorCode::ResourceLimit
        );
    }

    // Independent, trivial flat page-tree author for the label-only tests.
    // Production duplication has separate page, ownership and field checks.
    fn duplicate_first_for_labels(source: &PdfDocument) -> PdfDocument {
        let fresh = key(source
            .parsed()
            .objects
            .keys()
            .map(|r| r.number)
            .max()
            .unwrap()
            + 1);
        let mut parsed = source.parsed().clone();
        let mut clone = parsed.object(key(3)).unwrap().clone();
        clone.offset = 0;
        clone.stream_offset = 0;
        parsed.objects.insert(fresh, clone);
        let Value::Dict(pages) = &mut parsed.objects.get_mut(&key(2)).unwrap().value else {
            panic!()
        };
        pages.insert(
            b"Kids".to_vec(),
            Value::Array(vec![
                Value::Ref(key(3)),
                Value::Ref(fresh),
                Value::Ref(key(4)),
                Value::Ref(key(5)),
                Value::Ref(key(6)),
            ]),
        );
        pages.insert(b"Count".to_vec(), Value::Integer(5));
        let labels = plan_occurrence_labels(source, &[0, 0, 1, 2, 3], &context(u64::MAX)).unwrap();
        for (reference, object) in labels.changes {
            parsed.objects.insert(reference, object);
        }
        source.with_parsed(parsed)
    }

    #[test]
    fn duplicate_occurrences_inherit_explicit_labels_and_preserve_raw_tree_data() {
        let source = nested_labels();
        let actual = duplicate_first_for_labels(&source);
        let labels = crate::read_page_labels(&actual).unwrap();
        assert_eq!(
            labels
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["ΩI", "ΩI", "ΩII", "aa", "bb"]
        );
        assert!(
            verify_occurrence_mapping(
                &source,
                &actual,
                &[0, 0, 1, 2, 3],
                false,
                &context(u64::MAX)
            )
            .unwrap()
        );
        assert!(!verify(&source, &actual, &[0, 0, 1, 2, 3], &context(u64::MAX)).unwrap());
        assert!(plan_reordered_labels(&source, &[0, 0, 1, 2, 3], &context(u64::MAX)).is_err());
        let damaged = mutate_dictionary(&actual, 9, |leaf| {
            leaf.remove(b"LeafData".as_slice());
        });
        same_visible_labels(&actual, &damaged);
        assert!(
            !verify_occurrence_mapping(
                &source,
                &damaged,
                &[0, 0, 1, 2, 3],
                false,
                &context(u64::MAX)
            )
            .unwrap()
        );
        let damaged = mutate_dictionary(&actual, 11, |spec| {
            spec.remove(b"SpecData".as_slice());
        });
        same_visible_labels(&actual, &damaged);
        assert!(
            !verify_occurrence_mapping(
                &source,
                &damaged,
                &[0, 0, 1, 2, 3],
                false,
                &context(u64::MAX)
            )
            .unwrap()
        );
    }

    #[test]
    fn duplicate_without_explicit_labels_keeps_physical_numbering_and_catalog() {
        for labels in [
            "",
            "/PageLabels null",
            "/PageLabels << /Nums [] /Private (keep) >>",
        ] {
            let source = document(labels, &[]);
            let actual = duplicate_first_for_labels(&source);
            assert_eq!(
                source.parsed().object(key(1)).unwrap().value,
                actual.parsed().object(key(1)).unwrap().value
            );
            assert_eq!(
                crate::read_page_labels(&actual)
                    .unwrap()
                    .iter()
                    .map(|label| label.label.as_str())
                    .collect::<Vec<_>>(),
                ["1", "2", "3", "4", "5"]
            );
            assert!(
                verify_occurrence_mapping(
                    &source,
                    &actual,
                    &[0, 0, 1, 2, 3],
                    false,
                    &context(u64::MAX)
                )
                .unwrap()
            );
        }
    }

    #[test]
    fn occurrence_mapping_validates_source_bounds_not_expanded_output_bounds() {
        let source = document("/PageLabels << /Nums [0 << /S /D >>] >>", &[]);
        assert!(plan_occurrence_labels(&source, &[0, 1, 2, 3, 4], &context(u64::MAX)).is_err());
        for removed in [&[][..], &[0, 1, 2][..], &[0, 0, 0, 0, 0][..]] {
            assert!(plan_occurrence_labels(&source, removed, &context(u64::MAX)).is_err());
        }
        let candidate = duplicate_first_for_labels(&source);
        assert!(
            !verify_occurrence_mapping(
                &source,
                &candidate,
                &[0, 1, 2, 3, 4],
                false,
                &context(u64::MAX)
            )
            .unwrap()
        );
        assert!(
            !verify_occurrence_mapping(
                &source,
                &candidate,
                &[0, 1, 1, 2, 3],
                false,
                &context(u64::MAX)
            )
            .unwrap()
        );
        let malformed = document("/PageLabels << /Nums [4 << /S /D >>] >>", &[]);
        assert!(plan_occurrence_labels(&malformed, &[0, 0, 1, 2, 3], &context(u64::MAX)).is_err());
        let cancelled = context(u64::MAX);
        cancelled.cancellation().cancel();
        assert_eq!(
            plan_occurrence_labels(&source, &[0, 0, 1, 2, 3], &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        assert_eq!(
            plan_occurrence_labels(&source, &[0, 0, 1, 2, 3], &context(2))
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }

    #[test]
    fn nested_tree_preserves_unicode_styles_and_opaque_keys() {
        let document = document(
            "/PageLabels 7 0 R",
            &[
                "<< /Kids [8 0 R 9 0 R] /Limits [0 2] /Vendor 10 0 R >>",
                "<< /Nums [0 << /S /R /P <FEFF03A9> /Vendor 10 0 R >>] /Limits [0 0] /LeafData (keep) >>",
                "<< /Nums [2 << /S /a /St 27 >>] /Limits [2 2] /LeafData (also keep) >>",
                "<< /Private (opaque) >>",
            ],
        );
        let plan = plan_reordered_labels(&document, &[3, 0, 2, 1], &context(u64::MAX)).unwrap();
        assert_eq!(
            plan.expected_labels
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            vec!["bb", "ΩI", "aa", "ΩII"]
        );
        assert_eq!(
            plan.changes.keys().map(|r| r.number).collect::<Vec<_>>(),
            vec![7, 8, 9]
        );
        let root = dictionary(&plan.changes[&key(7)].value).unwrap();
        assert_eq!(
            root[b"Kids".as_slice()],
            Value::Array(vec![Value::Ref(key(8)), Value::Ref(key(9))])
        );
        assert_eq!(root[b"Vendor".as_slice()], Value::Ref(key(10)));
        let last_leaf = dictionary(&plan.changes[&key(9)].value).unwrap();
        assert_eq!(
            last_leaf[b"LeafData".as_slice()],
            Value::String(b"also keep".to_vec())
        );
        let Value::Array(nums) = &last_leaf[b"Nums".as_slice()] else {
            panic!()
        };
        let roman = dictionary(&nums[1]).unwrap();
        assert_eq!(
            roman[b"P".as_slice()],
            Value::String(vec![254, 255, 3, 169])
        );
        assert_eq!(roman[b"Vendor".as_slice()], Value::Ref(key(10)));
        assert_eq!(document.parsed().objects.changes().len(), 0);
    }

    #[test]
    fn decimal_ranges_and_leading_defaults_follow_pages() {
        let document = document(
            "/PageLabels << /Nums [2 << /S /D /St 8 /P (p-) >>] /Private (keep) >>",
            &[],
        );
        let plan = plan_reordered_labels(&document, &[3, 1, 2, 0], &context(u64::MAX)).unwrap();
        assert_eq!(
            plan.expected_labels
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            vec!["p-9", "2", "p-8", "1"]
        );
        assert_eq!(plan.changes.len(), 1);
        let catalog = dictionary(&plan.changes[&key(1)].value).unwrap();
        assert_eq!(
            dictionary(&catalog[b"PageLabels".as_slice()]).unwrap()[b"Private".as_slice()],
            Value::String(b"keep".to_vec())
        );
    }

    #[test]
    fn absent_or_empty_labels_keep_default_positions() {
        for labels in ["", "/PageLabels << /Nums [] /Private (keep) >>"] {
            let document = document(labels, &[]);
            let plan = plan_reordered_labels(&document, &[3, 2, 1, 0], &context(u64::MAX)).unwrap();
            assert!(plan.changes.is_empty());
            assert_eq!(plan.expected_labels[0].label, "1");
        }
    }

    #[test]
    fn malformed_trees_and_specs_fail_explicitly() {
        for tree in [
            "<< /Nums [0 << /S /D >> 0 << /S /R >>] >>",
            "<< /Nums [0 << /St 0 >>] >>",
            "<< /Nums [0 << /S /Unknown >>] >>",
            "<< /Nums [4 << /S /D >>] >>",
            "<< /Nums [0 << /S /D >>] /Limits [0 3] >>",
            "<< /Kids [7 0 R] >>",
        ] {
            let document = document("/PageLabels 7 0 R", &[tree]);
            assert!(
                plan_reordered_labels(&document, &[3, 2, 1, 0], &context(u64::MAX)).is_err(),
                "{tree}"
            );
        }
    }

    #[test]
    fn null_entries_are_absent_and_raw_prefix_only_specs_survive() {
        let document = document(
            "/PageLabels << /Kids null /Limits null /Nums [0 << /S null /P <FEFF03A9> /St null /Opaque null >>] >>",
            &[],
        );
        let plan = plan_reordered_labels(&document, &[3, 2, 1, 0], &context(u64::MAX)).unwrap();
        assert!(plan.expected_labels.iter().all(|label| label.label == "Ω"));
        let catalog = dictionary(&plan.changes[&key(1)].value).unwrap();
        let root = dictionary(&catalog[b"PageLabels".as_slice()]).unwrap();
        let Value::Array(nums) = &root[b"Nums".as_slice()] else {
            panic!()
        };
        assert_eq!(
            dictionary(&nums[1]).unwrap()[b"Opaque".as_slice()],
            Value::Null
        );
        let absent = self::document("/PageLabels null", &[]);
        assert!(
            plan_reordered_labels(&absent, &[3, 2, 1, 0], &context(u64::MAX))
                .unwrap()
                .changes
                .is_empty()
        );
    }

    #[test]
    fn indirect_tree_depth_is_bounded() {
        let mut nodes = (7..140)
            .map(|number| format!("<< /Kids [{} 0 R] >>", number + 1))
            .collect::<Vec<_>>();
        nodes.push("<< /Nums [0 << /S /D >>] >>".into());
        let refs = nodes.iter().map(String::as_str).collect::<Vec<_>>();
        let document = document("/PageLabels 7 0 R", &refs);
        assert_eq!(
            plan_reordered_labels(&document, &[3, 2, 1, 0], &context(u64::MAX))
                .err()
                .unwrap()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }

    #[test]
    fn large_decimal_and_prefix_only_labels_charge_rendered_bytes() {
        let decimal = document(
            "/PageLabels << /Nums [0 << /S /D /St 1000000000000 >>] >>",
            &[],
        );
        let bounded = context(100);
        let plan = plan_reordered_labels(&decimal, &[3, 2, 1, 0], &bounded).unwrap();
        assert_eq!(plan.expected_labels[0].label, "1000000000003");
        assert!(bounded.budget().snapshot().work_units < 100);
        let prefix = document(
            "/PageLabels << /Nums [0 << /P (only) /St 9223372036854775807 >>] >>",
            &[],
        );
        let plan = plan_reordered_labels(&prefix, &[3, 2, 1, 0], &context(100)).unwrap();
        assert!(
            plan.expected_labels
                .iter()
                .all(|label| label.label == "only")
        );
    }

    #[test]
    fn identity_validates_labels_without_rewriting_them() {
        let source = document(
            "/PageLabels << /Nums [0 << /S /D /St 9223372036854775807 >>] >>",
            &[],
        );
        let plan = plan_reordered_labels(&source, &[0, 1, 2, 3], &context(150)).unwrap();
        assert!(plan.changes.is_empty());
        assert_eq!(plan.expected_labels[3].label, "9223372036854775810");
        let malformed = document("/PageLabels << /Nums [0 << /St 0 >>] >>", &[]);
        assert!(plan_reordered_labels(&malformed, &[0, 1, 2, 3], &context(150)).is_err());
    }

    #[test]
    fn label_nodes_cannot_alias_other_document_roles() {
        for reference in [1, 2, 3] {
            let source = document(&format!("/PageLabels {reference} 0 R"), &[]);
            assert_eq!(
                plan_reordered_labels(&source, &[3, 2, 1, 0], &context(100))
                    .err()
                    .unwrap()
                    .code,
                PdfErrorCode::UnsafeRewrite
            );
        }
        let source = document("/PageLabels << /Nums [0 3 0 R] >>", &[]);
        assert!(plan_reordered_labels(&source, &[3, 2, 1, 0], &context(100)).is_err());
    }

    #[test]
    fn cancellation_work_and_render_bounds_are_enforced() {
        let document = document(
            "/PageLabels << /Nums [0 << /S /R /St 9223372036854775807 >>] >>",
            &[],
        );
        assert_eq!(
            plan_reordered_labels(&document, &[3, 2, 1, 0], &context(u64::MAX))
                .err()
                .unwrap()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let document = self::document("/PageLabels << /Nums [0 << /S /D >>] >>", &[]);
        assert_eq!(
            plan_reordered_labels(&document, &[3, 2, 1, 0], &context(2))
                .err()
                .unwrap()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let cancelled = context(u64::MAX);
        cancelled.cancellation().cancel();
        assert_eq!(
            plan_reordered_labels(&document, &[3, 2, 1, 0], &cancelled)
                .err()
                .unwrap()
                .code,
            PdfErrorCode::Cancelled
        );
        assert!(plan_reordered_labels(&document, &[0, 0, 2, 3], &context(u64::MAX)).is_err());
    }

    fn reordered(source: &PdfDocument, order: &[usize]) -> PdfDocument {
        let pages = source.page_refs().unwrap();
        let requested: Vec<_> = order.iter().map(|index| pages[*index]).collect();
        let pages =
            crate::preservation::pages::plan_reorder(source, &requested, &context(u64::MAX))
                .unwrap();
        let labels = plan_reordered_labels(source, order, &context(u64::MAX)).unwrap();
        let mut parsed = source.parsed().clone();
        for (reference, replacement) in pages.replacements.into_iter().chain(labels.changes) {
            parsed.objects.insert(reference, replacement);
        }
        source.with_parsed(parsed)
    }

    fn mutate_dictionary(
        source: &PdfDocument,
        number: u32,
        mutate: impl FnOnce(&mut Dictionary),
    ) -> PdfDocument {
        let mut parsed = source.parsed().clone();
        let mut object = parsed.object(key(number)).unwrap().clone();
        let Value::Dict(dictionary) = &mut object.value else {
            panic!("dictionary")
        };
        mutate(dictionary);
        parsed.objects.insert(key(number), object);
        source.with_parsed(parsed)
    }

    fn direct_labels(catalog: &mut Dictionary) -> &mut Dictionary {
        let Some(Value::Dict(labels)) = catalog.get_mut(b"PageLabels".as_slice()) else {
            panic!("direct labels")
        };
        labels
    }

    fn first_spec(labels: &mut Dictionary) -> &mut Dictionary {
        let Some(Value::Array(nums)) = labels.get_mut(b"Nums".as_slice()) else {
            panic!("label pairs")
        };
        let Value::Dict(spec) = &mut nums[1] else {
            panic!("direct spec")
        };
        spec
    }

    fn nested_labels() -> PdfDocument {
        document(
            "/PageLabels 7 0 R /Lang (en-GB)",
            &[
                "<< /Kids [8 0 R 9 0 R] /Limits [0 2] /Vendor 10 0 R >>",
                "<< /Nums [0 11 0 R] /Limits [0 0] /LeafData (first) >>",
                "<< /Nums [2 << /S /a /St 27 /SpecData (last) >>] /Limits [2 2] /LeafData (last) >>",
                "<< /Private (opaque) >>",
                "<< /S /R /P <FEFF03A9> /Vendor 10 0 R /SpecData (first) >>",
            ],
        )
    }

    fn same_visible_labels(left: &PdfDocument, right: &PdfDocument) {
        assert_eq!(
            crate::read_page_labels(left).unwrap(),
            crate::read_page_labels(right).unwrap()
        );
    }

    #[test]
    fn verifier_accepts_nested_direct_default_and_prefix_only_mappings() {
        let order = [3, 0, 2, 1];
        let source = nested_labels();
        let candidate = reordered(&source, &order);
        assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
        assert_eq!(
            crate::read_page_labels(&candidate)
                .unwrap()
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["bb", "ΩI", "aa", "ΩII"]
        );
        // Independent page geometry changes do not grant any catalog/label edits.
        let candidate = mutate_dictionary(&candidate, 6, |page| {
            page.insert(b"Rotate".to_vec(), Value::Integer(180));
        });
        assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
        let order = [3, 1, 2, 0];
        let source = document(
            "/PageLabels << /Nums [2 << /S /D /St 8 /P (p-) /Opaque null >>] /Private (keep) >>",
            &[],
        );
        let candidate = reordered(&source, &order);
        assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
        assert_eq!(
            crate::read_page_labels(&candidate)
                .unwrap()
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["p-9", "2", "p-8", "1"]
        );
        let source = document(
            "/PageLabels << /Kids null /Limits null /Nums [0 << /S null /P <FEFF03A9> /St null /Opaque null >>] >>",
            &[],
        );
        let candidate = reordered(&source, &order);
        assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
        assert_eq!(source.parsed().objects.changes().len(), 0);
    }

    #[test]
    fn verifier_rejects_catalog_key_loss_with_unchanged_visible_labels() {
        let source = document(
            "/Lang (en-GB) /PageLayout /SinglePage /PageLabels << /Nums [0 << /S /D >>] >>",
            &[],
        );
        let order = [3, 2, 1, 0];
        let candidate = reordered(&source, &order);
        for removed in [b"Lang".as_slice(), b"PageLayout"] {
            let damaged = mutate_dictionary(&candidate, 1, |catalog| {
                catalog.remove(removed);
            });
            same_visible_labels(&candidate, &damaged);
            assert!(!verify(&source, &damaged, &order, &context(u64::MAX)).unwrap());
        }
        assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
    }

    #[test]
    fn verifier_rejects_tree_opaque_key_loss_with_unchanged_visible_labels() {
        let source = nested_labels();
        let order = [3, 0, 2, 1];
        let candidate = reordered(&source, &order);
        for (number, removed) in [
            (7, b"Vendor".as_slice()),
            (8, b"LeafData"),
            (9, b"LeafData"),
        ] {
            let damaged = mutate_dictionary(&candidate, number, |dictionary| {
                dictionary.remove(removed);
            });
            same_visible_labels(&candidate, &damaged);
            assert!(!verify(&source, &damaged, &order, &context(u64::MAX)).unwrap());
        }
    }

    #[test]
    fn verifier_rejects_spec_opaque_loss_and_equivalent_text_reencoding() {
        let source = document(
            "/PageLabels << /Nums [0 << /P (P) /S null /St null /Opaque << /Keep 42 >> >>] >>",
            &[],
        );
        let order = [3, 2, 1, 0];
        let candidate = reordered(&source, &order);
        for change in 0..4 {
            let damaged = mutate_dictionary(&candidate, 1, |catalog| {
                let spec = first_spec(direct_labels(catalog));
                match change {
                    0 => {
                        spec.remove(b"Opaque".as_slice());
                    }
                    1 => {
                        spec.insert(b"P".to_vec(), Value::String(vec![254, 255, 0, b'P']));
                    }
                    2 => {
                        spec.remove(b"St".as_slice());
                    }
                    _ => {
                        spec.remove(b"S".as_slice());
                    }
                }
            });
            same_visible_labels(&candidate, &damaged);
            assert!(!verify(&source, &damaged, &order, &context(u64::MAX)).unwrap());
        }
    }

    #[test]
    fn verifier_rejects_changed_topology_and_validly_redistributed_ranges() {
        let source = document(
            "/PageLabels 7 0 R",
            &[
                "<< /Kids [8 0 R 9 0 R] /Limits [0 2] >>",
                "<< /Nums [0 << /P (same) >>] /Limits [0 0] /LeafData (first) >>",
                "<< /Nums [2 << /P (same) >>] /Limits [2 2] /LeafData (last) >>",
            ],
        );
        let order = [3, 0, 2, 1];
        let candidate = reordered(&source, &order);
        let first = dictionary(&candidate.parsed().object(key(8)).unwrap().value).unwrap();
        let last = dictionary(&candidate.parsed().object(key(9)).unwrap().value).unwrap();
        // Swap the nodes and their range containers together. The reader still
        // sees sorted identical labels; original node identities no longer agree.
        let damaged = mutate_dictionary(&candidate, 7, |root| {
            root.insert(
                b"Kids".to_vec(),
                Value::Array(vec![Value::Ref(key(9)), Value::Ref(key(8))]),
            );
        });
        let damaged = mutate_dictionary(&damaged, 8, |node| {
            for name in [b"Nums".as_slice(), b"Limits"] {
                node.insert(name.to_vec(), last[name].clone());
            }
        });
        let damaged = mutate_dictionary(&damaged, 9, |node| {
            for name in [b"Nums".as_slice(), b"Limits"] {
                node.insert(name.to_vec(), first[name].clone());
            }
        });
        same_visible_labels(&candidate, &damaged);
        assert!(!verify(&source, &damaged, &order, &context(u64::MAX)).unwrap());
        // Retain node order but move one entry across the original leaf capacity.
        let Value::Array(first_nums) = &first[b"Nums".as_slice()] else {
            panic!()
        };
        let Value::Array(last_nums) = &last[b"Nums".as_slice()] else {
            panic!()
        };
        let mut expanded = first_nums.clone();
        expanded.extend_from_slice(&last_nums[..2]);
        let damaged = mutate_dictionary(&candidate, 8, |node| {
            node.insert(b"Nums".to_vec(), Value::Array(expanded));
            node.insert(
                b"Limits".to_vec(),
                Value::Array(vec![Value::Integer(0), Value::Integer(1)]),
            );
        });
        let damaged = mutate_dictionary(&damaged, 9, |node| {
            node.insert(b"Nums".to_vec(), Value::Array(last_nums[2..].to_vec()));
            node.insert(
                b"Limits".to_vec(),
                Value::Array(vec![Value::Integer(2), Value::Integer(3)]),
            );
        });
        same_visible_labels(&candidate, &damaged);
        assert!(!verify(&source, &damaged, &order, &context(u64::MAX)).unwrap());
    }

    #[test]
    fn verifier_preserves_limits_presence_and_original_indirect_specs() {
        let source = nested_labels();
        let order = [3, 0, 2, 1];
        let candidate = reordered(&source, &order);
        let missing_limits = mutate_dictionary(&candidate, 8, |node| {
            node.remove(b"Limits".as_slice());
        });
        same_visible_labels(&candidate, &missing_limits);
        assert!(!verify(&source, &missing_limits, &order, &context(u64::MAX)).unwrap());
        let damaged_spec = mutate_dictionary(&candidate, 11, |spec| {
            spec.remove(b"SpecData".as_slice());
        });
        same_visible_labels(&candidate, &damaged_spec);
        assert!(!verify(&source, &damaged_spec, &order, &context(u64::MAX)).unwrap());
        let source = document("/PageLabels << /Nums [0 << /S /D >>] /Limits null >>", &[]);
        let candidate = reordered(&source, &order);
        let added_limits = mutate_dictionary(&candidate, 1, |catalog| {
            direct_labels(catalog).insert(
                b"Limits".to_vec(),
                Value::Array(vec![Value::Integer(0), Value::Integer(3)]),
            );
        });
        same_visible_labels(&candidate, &added_limits);
        assert!(!verify(&source, &added_limits, &order, &context(u64::MAX)).unwrap());
    }

    #[test]
    fn verifier_identity_absent_and_empty_cases_allow_no_label_changes() {
        for labels in [
            "",
            "/PageLabels null",
            "/PageLabels << >>",
            "/PageLabels << /Nums [] /Kids null /Private (keep) >>",
        ] {
            let source = document(labels, &[]);
            let order = [3, 2, 1, 0];
            let candidate = reordered(&source, &order);
            assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
            let damaged = mutate_dictionary(&candidate, 1, |catalog| {
                catalog.insert(
                    b"PageLabels".to_vec(),
                    Value::Dict(BTreeMap::from([(b"Nums".to_vec(), Value::Array(vec![]))])),
                );
            });
            same_visible_labels(&candidate, &damaged);
            assert!(!verify(&source, &damaged, &order, &context(u64::MAX)).unwrap());
        }
        let source = nested_labels();
        let order = [0, 1, 2, 3];
        let candidate = reordered(&source, &order);
        assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
        let damaged = mutate_dictionary(&candidate, 8, |node| {
            node.remove(b"LeafData".as_slice());
        });
        same_visible_labels(&candidate, &damaged);
        assert!(!verify(&source, &damaged, &order, &context(u64::MAX)).unwrap());
    }

    #[test]
    fn verifier_mapping_source_and_operation_budgets_are_enforced() {
        let source = nested_labels();
        let order = [3, 0, 2, 1];
        let candidate = reordered(&source, &order);
        assert!(!verify(&source, &candidate, &[3, 0, 2, 0], &context(u64::MAX)).unwrap());
        assert!(!verify(&source, &candidate, &[3, 0, 2], &context(u64::MAX)).unwrap());
        assert!(!verify(&source, &candidate, &[0, 1, 2, 3], &context(u64::MAX)).unwrap());
        let foreign = document("/PageLabels << /Nums [0 << /S /D >>] >>", &[]);
        assert!(!verify(&source, &foreign, &order, &context(u64::MAX)).unwrap());
        let direct_child = document("/PageLabels << /Kids [<< /Nums [0 << /S /D >>] >>] >>", &[]);
        assert_eq!(
            verify(
                &direct_child,
                &direct_child,
                &[0, 1, 2, 3],
                &context(u64::MAX)
            )
            .unwrap_err()
            .code,
            PdfErrorCode::InvalidSyntax,
        );
        assert_eq!(
            verify(&source, &candidate, &order, &context(0))
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let cancelled = context(u64::MAX);
        cancelled.cancellation().cancel();
        assert_eq!(
            verify(&source, &candidate, &order, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        let no_memory = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: u64::MAX,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            verify(&source, &candidate, &order, &no_memory)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert!(verify(&source, &candidate, &order, &context(u64::MAX)).unwrap());
        assert_eq!(source.parsed().objects.changes().len(), 0);
    }
}
