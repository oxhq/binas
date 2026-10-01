//! Absolute page boxes and rotation, resolved against stable source page identities.
//!
//! The independent effective-geometry expectation is kept apart from emitted PDF
//! dictionary changes. The enclosing transaction checks other requested objects
//! and catalog/trailer preservation; this module does not transform content.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SourceId,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};

use crate::{
    PageGeometry, PageTransform, PdfDocument, PdfError, PdfReferenceTarget,
    object::{IndirectObject, ObjectRef, Value},
    store::ObjectStore,
};

#[derive(Clone, Debug, PartialEq)]
pub struct PdfPageTransformRequest {
    /// Zero-based index in the source revision, before any requested reorder.
    pub page_index: usize,
    /// Absolute boxes and rotation only; scale and translate are unsupported.
    pub transform: PageTransform,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfPageGeometryEffect {
    pub page: PdfReferenceTarget,
    pub before: PageGeometry,
    pub after: PageGeometry,
}

#[derive(Clone, Copy, Debug)]
struct SourceGeometry {
    effective: PageGeometry,
    // Absence is meaningful: a media-only request changes the default CropBox.
    crop_present: bool,
}

#[derive(Debug)]
pub(crate) struct GeometryPlan {
    source: SourceId,
    store: ObjectStore,
    original: BTreeMap<ObjectRef, SourceGeometry>,
    changes: BTreeMap<ObjectRef, BTreeMap<Vec<u8>, Value>>,
    effects: Vec<PdfPageGeometryEffect>,
    effect_indices: BTreeMap<ObjectRef, usize>,
    default_crop: BTreeSet<ObjectRef>,
    // Logical retained records and cloned values; not allocator/process memory.
    _reservations: Vec<Reservation>,
}

impl GeometryPlan {
    pub(crate) fn effects(&self) -> &[PdfPageGeometryEffect] {
        &self.effects
    }

    pub(crate) fn expected_for(&self, page: ObjectRef) -> Option<&PageGeometry> {
        self.effect_indices
            .get(&page)
            .map(|index| &self.effects[*index].after)
    }

    pub(crate) fn allows_key(&self, page: ObjectRef, key: &[u8]) -> bool {
        self.changes
            .get(&page)
            .is_some_and(|changes| changes.contains_key(key))
    }
}

pub(crate) fn plan(
    document: &PdfDocument,
    requests: &[PdfPageTransformRequest],
    context: &OperationContext,
) -> Result<GeometryPlan, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if requests.len() > document.parsed().limits.max_container_items {
        return Err(PdfError::limit(
            "page geometry request count exceeds container limit",
        ));
    }
    super::pages::validate_tree(document, context)?;
    context
        .charge_work(document.parsed().objects.len() as u64)
        .map_err(PdfError::from_context)?;
    let pages = document.page_refs()?;
    let mut temporary = Vec::new();
    reserve(
        &mut temporary,
        pages.len().checked_mul(std::mem::size_of::<ObjectRef>()),
        context,
    )?;
    let mut merged = BTreeMap::<ObjectRef, PageTransform>::new();
    for request in requests {
        context.charge_work(1).map_err(PdfError::from_context)?;
        validate_request(&request.transform)?;
        let page = *pages
            .get(request.page_index)
            .ok_or_else(|| PdfError::selection("geometry page index is out of range"))?;
        if !merged.contains_key(&page) {
            reserve(
                &mut temporary,
                Some(std::mem::size_of::<(ObjectRef, PageTransform)>()),
                context,
            )?;
        }
        let previous = merged.entry(page).or_default();
        if (previous.rotation_degrees.is_some() && request.transform.rotation_degrees.is_some())
            || (previous.media_box.is_some() && request.transform.media_box.is_some())
            || (previous.crop_box.is_some() && request.transform.crop_box.is_some())
        {
            return Err(PdfError::unsafe_rewrite(
                "geometry requests overlap on the same page dictionary key",
            ));
        }
        previous.rotation_degrees = previous
            .rotation_degrees
            .or(request.transform.rotation_degrees);
        previous.media_box = previous.media_box.or(request.transform.media_box);
        previous.crop_box = previous.crop_box.or(request.transform.crop_box);
    }
    let mut result = GeometryPlan {
        source: document.source_id(),
        store: document.parsed().objects.clone(),
        original: BTreeMap::new(),
        changes: BTreeMap::new(),
        effects: Vec::new(),
        effect_indices: BTreeMap::new(),
        default_crop: BTreeSet::new(),
        _reservations: Vec::new(),
    };
    // Source and unselected pages must have known valid geometry as well. A
    // successful repair of one page cannot conceal unknown source geometry.
    for page in pages {
        let source = read_source(document, page, context)?;
        reserve(
            &mut result._reservations,
            Some(std::mem::size_of::<(ObjectRef, SourceGeometry)>()),
            context,
        )?;
        if result.original.insert(page, source).is_some() {
            return Err(PdfError::unsafe_rewrite(
                "geometry source repeats a page identity",
            ));
        }
    }
    for (page, transform) in merged {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let before = result.original[&page];
        let mut after = before.effective;
        if let Some(media) = transform.media_box {
            after.media_box = media;
            if !before.crop_present && transform.crop_box.is_none() {
                after.crop_box = media;
            }
        }
        if let Some(crop) = transform.crop_box {
            after.crop_box = crop;
        }
        if let Some(rotation) = transform.rotation_degrees {
            after.rotation_degrees = rotation.rem_euclid(360);
        }
        validate_geometry(&after)?;
        let mut changes = BTreeMap::new();
        if let Some(media) = transform.media_box {
            insert_change(
                &mut changes,
                b"MediaBox",
                rectangle_value(media),
                &mut result._reservations,
                context,
            )?;
        }
        if transform.crop_box.is_some() {
            insert_change(
                &mut changes,
                b"CropBox",
                rectangle_value(after.crop_box),
                &mut result._reservations,
                context,
            )?;
        }
        if transform.media_box.is_some() && transform.crop_box.is_none() && !before.crop_present {
            reserve(
                &mut result._reservations,
                Some(std::mem::size_of::<ObjectRef>()),
                context,
            )?;
            result.default_crop.insert(page);
        }
        if transform.rotation_degrees.is_some() {
            insert_change(
                &mut changes,
                b"Rotate",
                Value::Integer(i64::from(after.rotation_degrees)),
                &mut result._reservations,
                context,
            )?;
        }
        reserve(
            &mut result._reservations,
            Some(
                std::mem::size_of::<PdfPageGeometryEffect>()
                    + std::mem::size_of::<(ObjectRef, usize)>()
                    + std::mem::size_of::<ObjectRef>(),
            ),
            context,
        )?;
        result.effect_indices.insert(page, result.effects.len());
        result.effects.push(PdfPageGeometryEffect {
            page: PdfReferenceTarget {
                number: page.number,
                generation: page.generation,
            },
            before: before.effective,
            after,
        });
        result.changes.insert(page, changes);
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(result)
}

/// Overlay geometry after all other page changes. Reorder's materialized
/// inheritable keys are an implementation detail, not conflicting user requests.
pub(crate) fn apply(
    plan: &mut GeometryPlan,
    document: &PdfDocument,
    objects: &mut BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    bound(plan, document)?;
    context.checkpoint().map_err(PdfError::from_context)?;
    for (page, changes) in &mut plan.changes {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if plan.default_crop.contains(page)
            && composed_crop_present(document, objects, *page, context)?
        {
            // Reorder may materialize the old crop or introduce a new parent
            // crop. Only in that case make the requested new default explicit.
            let expected = plan.effects[plan.effect_indices[page]].after.crop_box;
            insert_change(
                changes,
                b"CropBox",
                rectangle_value(expected),
                &mut plan._reservations,
                context,
            )?;
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = objects.entry(*page) {
            let original = document.parsed().object(*page)?;
            reserve(
                &mut plan._reservations,
                value_cost(
                    &original.value,
                    0,
                    document.parsed().limits.max_parser_depth,
                    context,
                )?
                .checked_add(std::mem::size_of::<IndirectObject>()),
                context,
            )?;
            entry.insert(original.clone());
        }
        let object = objects.get_mut(page).expect("inserted geometry page");
        if object.stream.is_some() {
            return Err(PdfError::unsafe_rewrite(
                "geometry page dictionary cannot be a stream",
            ));
        }
        let Value::Dict(dictionary) = &mut object.value else {
            return Err(PdfError::unsafe_rewrite(
                "geometry page is not a dictionary",
            ));
        };
        for (key, value) in changes {
            reserve(
                &mut plan._reservations,
                value_cost(value, 0, document.parsed().limits.max_parser_depth, context)?
                    .checked_add(key.len()),
                context,
            )?;
            dictionary.insert(key.clone(), value.clone());
        }
    }
    context.checkpoint().map_err(PdfError::from_context)
}

const CHECKS: [&str; 4] = [
    "stable_pages",
    "requested_geometry",
    "unselected_geometry",
    "page_keys",
];

fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.geometry", name).expect("static geometry code")
}

pub(crate) fn required_checks() -> RequiredChecks {
    RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
        .expect("static geometry obligations")
}

/// Compare effective geometry to the independent source-derived intent. Other
/// page keys must exactly match the enclosing transaction's composed changes;
/// those other operations retain responsibility for their semantic validation.
pub(crate) fn verify(
    original: &PdfDocument,
    candidate: &PdfDocument,
    plan: &GeometryPlan,
    expected_objects: &BTreeMap<ObjectRef, IndirectObject>,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    bound(plan, original)?;
    if candidate.source_id() != plan.source {
        return Err(PdfError::selection("geometry candidate source mismatch"));
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    super::pages::validate_tree(candidate, context)?;
    context
        .charge_work(candidate.parsed().objects.len() as u64)
        .map_err(PdfError::from_context)?;
    let pages = candidate.page_refs()?;
    let mut temporary = Vec::new();
    reserve(
        &mut temporary,
        pages.len().checked_mul(std::mem::size_of::<ObjectRef>()),
        context,
    )?;
    let mut seen = BTreeSet::new();
    let mut stable = pages.len() == plan.original.len();
    let mut requested = true;
    let mut unselected = true;
    let mut keys = true;
    for page in pages {
        context.charge_work(1).map_err(PdfError::from_context)?;
        reserve(
            &mut temporary,
            Some(std::mem::size_of::<ObjectRef>()),
            context,
        )?;
        stable &= seen.insert(page) && plan.original.contains_key(&page);
        let Some(before) = plan.original.get(&page) else {
            continue;
        };
        let actual = read_for(candidate, page, context)?;
        match plan.expected_for(page) {
            Some(expected) => requested &= actual == *expected,
            None => unselected &= actual == before.effective,
        }
        let object = candidate.parsed().object(page)?;
        let expected = match expected_objects.get(&page) {
            Some(object) => object,
            None => original.parsed().object(page)?,
        };
        let Value::Dict(actual_dictionary) = &object.value else {
            return Err(PdfError::verification(
                "geometry candidate page is not a dictionary",
            ));
        };
        let Value::Dict(expected_dictionary) = &expected.value else {
            return Err(PdfError::verification(
                "geometry expected page is not a dictionary",
            ));
        };
        let changes = plan.changes.get(&page);
        for (key, value) in actual_dictionary {
            context.charge_work(1).map_err(PdfError::from_context)?;
            let wanted = changes
                .and_then(|values| values.get(key))
                .or_else(|| expected_dictionary.get(key));
            keys &= wanted == Some(value);
        }
        for key in expected_dictionary
            .keys()
            .chain(changes.into_iter().flat_map(|values| values.keys()))
        {
            context.charge_work(1).map_err(PdfError::from_context)?;
            keys &= actual_dictionary.contains_key(key);
        }
        keys &= object.stream.as_deref() == expected.stream.as_deref();
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(EvidenceReport::new(CHECKS.into_iter().zip([stable, requested, unselected, keys]).map(|(name, passed)| CheckEvidence {
        code: code(name),
        status: if passed { CheckStatus::Passed } else { CheckStatus::Failed },
        detail: "Stable page identities, source-derived effective boxes/rotation and exact composed page keys; content, appearance and conformance are not transformed or inferred".into(),
    }).collect()))
}

pub(crate) fn read_for(
    document: &PdfDocument,
    page: ObjectRef,
    context: &OperationContext,
) -> Result<PageGeometry, PdfError> {
    read_source(document, page, context).map(|source| source.effective)
}

fn composed_crop_present(
    document: &PdfDocument,
    objects: &BTreeMap<ObjectRef, IndirectObject>,
    mut page: ObjectRef,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    for _ in 0..=document.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let object = match objects.get(&page) {
            Some(object) => object,
            None => document.parsed().object(page)?,
        };
        let Value::Dict(dictionary) = &object.value else {
            return Err(PdfError::unsafe_rewrite(
                "composed geometry page node is not a dictionary",
            ));
        };
        if let Some(value) = dictionary.get(b"CropBox".as_slice())
            && !matches!(resolve(document, value, context)?, Value::Null)
        {
            return Ok(true);
        }
        match dictionary.get(b"Parent".as_slice()) {
            Some(Value::Ref(parent)) => page = *parent,
            None | Some(Value::Null) => return Ok(false),
            Some(_) => {
                return Err(PdfError::unsafe_rewrite(
                    "composed geometry Parent must be a reference",
                ));
            }
        }
    }
    Err(PdfError::limit(
        "composed geometry inheritance depth exceeds limit",
    ))
}

fn read_source(
    document: &PdfDocument,
    page: ObjectRef,
    context: &OperationContext,
) -> Result<SourceGeometry, PdfError> {
    let leaf = document.parsed().object(page)?;
    let Value::Dict(leaf_dictionary) = &leaf.value else {
        return Err(PdfError::unsafe_rewrite(
            "geometry page is not a dictionary",
        ));
    };
    if leaf.stream.is_some()
        || !matches!(leaf_dictionary.get(b"Type".as_slice()), Some(Value::Name(kind)) if kind == b"Page")
    {
        return Err(PdfError::unsafe_rewrite(
            "geometry identity must be a non-stream Page",
        ));
    }
    let mut media = None;
    let mut crop = None;
    let mut rotate = None;
    let mut current = page;
    let mut seen = BTreeSet::new();
    let mut temporary = Vec::new();
    for depth in 0..=document.parsed().limits.max_parser_depth {
        context.charge_work(1).map_err(PdfError::from_context)?;
        reserve(
            &mut temporary,
            Some(std::mem::size_of::<ObjectRef>()),
            context,
        )?;
        if !seen.insert(current) {
            return Err(PdfError::unsafe_rewrite(
                "cycle in geometry page inheritance",
            ));
        }
        let object = document.parsed().object(current)?;
        let Value::Dict(dictionary) = &object.value else {
            return Err(PdfError::unsafe_rewrite(
                "geometry inheritance node is not a dictionary",
            ));
        };
        if object.stream.is_some()
            || (depth != 0
                && !matches!(dictionary.get(b"Type".as_slice()), Some(Value::Name(kind)) if kind == b"Pages"))
        {
            return Err(PdfError::unsafe_rewrite(
                "geometry parent must be a non-stream Pages node",
            ));
        }
        for (key, slot) in [
            (b"MediaBox".as_slice(), &mut media),
            (b"CropBox".as_slice(), &mut crop),
            (b"Rotate".as_slice(), &mut rotate),
        ] {
            if slot.is_none()
                && let Some(value) = dictionary.get(key)
            {
                let value = resolve(document, value, context)?;
                if !matches!(value, Value::Null) {
                    *slot = Some(value);
                }
            }
        }
        match dictionary.get(b"Parent".as_slice()) {
            None | Some(Value::Null) => break,
            Some(Value::Ref(parent)) => {
                if depth == document.parsed().limits.max_parser_depth {
                    return Err(PdfError::limit("geometry inheritance depth exceeds limit"));
                }
                current = *parent;
            }
            Some(_) => {
                return Err(PdfError::unsafe_rewrite(
                    "geometry Parent must be a reference",
                ));
            }
        }
    }
    let media_box = rectangle(
        media.ok_or_else(|| PdfError::unsafe_rewrite("geometry page has no effective MediaBox"))?,
    )?;
    let crop_box = crop.map(rectangle).transpose()?.unwrap_or(media_box);
    let rotation_degrees = match rotate {
        None => 0,
        Some(Value::Integer(rotation)) if rotation % 90 == 0 => rotation.rem_euclid(360) as i32,
        Some(_) => {
            return Err(PdfError::unsafe_rewrite(
                "geometry Rotate must be an integer multiple of 90",
            ));
        }
    };
    let direct_box = |key: &[u8]| -> Result<Option<[f64; 4]>, PdfError> {
        match leaf_dictionary.get(key) {
            None => Ok(None),
            Some(value) => match resolve(document, value, context)? {
                Value::Null => Ok(None),
                value => rectangle(value).map(Some),
            },
        }
    };
    let effective = PageGeometry {
        media_box,
        crop_box,
        bleed_box: direct_box(b"BleedBox")?,
        trim_box: direct_box(b"TrimBox")?,
        art_box: direct_box(b"ArtBox")?,
        rotation_degrees,
    };
    validate_geometry(&effective)?;
    Ok(SourceGeometry {
        effective,
        crop_present: crop.is_some(),
    })
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
                    return Err(PdfError::unsafe_rewrite(
                        "geometry value cannot reference a stream",
                    ));
                }
                value = &object.value;
            }
            _ => return Ok(value),
        }
    }
    Err(PdfError::limit("geometry value indirection exceeds limit"))
}

fn rectangle(value: &Value) -> Result<[f64; 4], PdfError> {
    let Value::Array(values) = value else {
        return Err(PdfError::unsafe_rewrite(
            "geometry box must be a four-number array",
        ));
    };
    if values.len() != 4 {
        return Err(PdfError::unsafe_rewrite(
            "geometry box must have four coordinates",
        ));
    }
    let mut result = [0.0; 4];
    for (output, input) in result.iter_mut().zip(values) {
        *output = match input {
            Value::Integer(number) => *number as f64,
            Value::Real(number) => *number,
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "geometry box contains a non-number",
                ));
            }
        };
    }
    validate_box(result)?;
    Ok(result)
}

fn validate_box(value: [f64; 4]) -> Result<(), PdfError> {
    let width = value[2] - value[0];
    let height = value[3] - value[1];
    if !value.iter().all(|number| number.is_finite())
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
    {
        return Err(PdfError::unsafe_rewrite(
            "geometry boxes require finite coordinates and positive finite dimensions",
        ));
    }
    Ok(())
}

fn validate_geometry(value: &PageGeometry) -> Result<(), PdfError> {
    validate_box(value.media_box)?;
    validate_box(value.crop_box)?;
    for rectangle in [value.bleed_box, value.trim_box, value.art_box]
        .into_iter()
        .flatten()
    {
        validate_box(rectangle)?;
    }
    if value.crop_box[0] < value.media_box[0]
        || value.crop_box[1] < value.media_box[1]
        || value.crop_box[2] > value.media_box[2]
        || value.crop_box[3] > value.media_box[3]
    {
        return Err(PdfError::unsafe_rewrite(
            "geometry CropBox must be contained by MediaBox",
        ));
    }
    if value.rotation_degrees % 90 != 0 {
        return Err(PdfError::unsafe_rewrite(
            "geometry rotation must be a multiple of 90",
        ));
    }
    Ok(())
}

fn validate_request(transform: &PageTransform) -> Result<(), PdfError> {
    if transform.translate.is_some() || transform.scale.is_some() {
        return Err(PdfError::unsupported(
            "retained page geometry does not transform content coordinates",
        ));
    }
    if transform.rotation_degrees.is_none()
        && transform.media_box.is_none()
        && transform.crop_box.is_none()
    {
        return Err(PdfError::selection(
            "page geometry request has no box or rotation change",
        ));
    }
    if transform
        .rotation_degrees
        .is_some_and(|rotation| rotation % 90 != 0)
    {
        return Err(PdfError::unsafe_rewrite(
            "geometry rotation must be a multiple of 90",
        ));
    }
    for rectangle in [transform.media_box, transform.crop_box]
        .into_iter()
        .flatten()
    {
        validate_box(rectangle)?;
    }
    Ok(())
}

fn rectangle_value(rectangle: [f64; 4]) -> Value {
    Value::Array(rectangle.into_iter().map(Value::Real).collect())
}

fn bound(plan: &GeometryPlan, document: &PdfDocument) -> Result<(), PdfError> {
    if document.source_id() != plan.source
        || !document.parsed().objects.shares_snapshot_with(&plan.store)
    {
        return Err(PdfError::selection(
            "geometry plan source/object snapshot mismatch",
        ));
    }
    Ok(())
}

fn reserve(
    reservations: &mut Vec<Reservation>,
    bytes: Option<usize>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let bytes =
        bytes.ok_or_else(|| PdfError::limit("geometry logical allocation size overflows"))?;
    reservations.push(
        context
            .reserve(bytes as u64)
            .map_err(PdfError::from_context)?,
    );
    Ok(())
}

fn insert_change(
    changes: &mut BTreeMap<Vec<u8>, Value>,
    key: &[u8],
    value: Value,
    reservations: &mut Vec<Reservation>,
    context: &OperationContext,
) -> Result<(), PdfError> {
    let cost = value_cost(&value, 0, 2, context)?.checked_add(key.len());
    reserve(reservations, cost, context)?;
    changes.insert(key.to_vec(), value);
    Ok(())
}

fn value_cost(
    value: &Value,
    depth: usize,
    limit: usize,
    context: &OperationContext,
) -> Result<usize, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > limit {
        return Err(PdfError::limit("geometry value depth exceeds limit"));
    }
    let mut result = std::mem::size_of::<Value>();
    match value {
        Value::Name(bytes) | Value::String(bytes) => {
            result = result
                .checked_add(bytes.len())
                .ok_or_else(|| PdfError::limit("geometry value size overflows"))?;
        }
        Value::Array(values) => {
            for value in values {
                result = result
                    .checked_add(value_cost(value, depth + 1, limit, context)?)
                    .ok_or_else(|| PdfError::limit("geometry array size overflows"))?;
            }
        }
        Value::Dict(values) => {
            for (key, value) in values {
                let child = value_cost(value, depth + 1, limit, context)?;
                result = result
                    .checked_add(key.len())
                    .and_then(|total| total.checked_add(child))
                    .ok_or_else(|| PdfError::limit("geometry dictionary size overflows"))?;
            }
        }
        _ => {}
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};

    fn fixture() -> PdfDocument {
        let bodies = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 300] >>",
            "<< /Type /Page /Parent 2 0 R >>",
            "<< /Type /Page /Parent 2 0 R >>",
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in bodies.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 10_000_000,
                max_work_units: 10_000_000,
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

    fn rotate() -> PdfPageTransformRequest {
        PdfPageTransformRequest {
            page_index: 0,
            transform: PageTransform {
                rotation_degrees: Some(90),
                ..Default::default()
            },
        }
    }

    fn stage(original: &PdfDocument, objects: &BTreeMap<ObjectRef, IndirectObject>) -> PdfDocument {
        let mut parsed = original.parsed().clone();
        for (reference, object) in objects {
            parsed.objects.insert(*reference, object.clone());
        }
        original.with_parsed(parsed)
    }

    #[test]
    fn geometry_independent_expectation_rejects_corrupted_emitted_values() {
        let original = fixture();
        let context = context();
        let mut plan = plan(&original, &[rotate()], &context).unwrap();
        let mut objects = BTreeMap::new();
        apply(&mut plan, &original, &mut objects, &context).unwrap();
        verify(
            &original,
            &stage(&original, &objects),
            &plan,
            &objects,
            &context,
        )
        .unwrap()
        .validate(&required_checks())
        .unwrap();

        // A self-consistent corruption of the write plan and staged object must
        // not alter the separately derived requested effective rotation.
        plan.changes
            .get_mut(&reference(3))
            .unwrap()
            .insert(b"Rotate".to_vec(), Value::Integer(180));
        apply(&mut plan, &original, &mut objects, &context).unwrap();
        let report = verify(
            &original,
            &stage(&original, &objects),
            &plan,
            &objects,
            &context,
        )
        .unwrap();
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code == code("requested_geometry")
                    && check.status == CheckStatus::Failed)
        );
        assert!(report.validate(&required_checks()).is_err());
    }

    #[test]
    fn geometry_unselected_effects_and_extra_page_keys_cannot_pass() {
        let original = fixture();
        let context = context();
        let mut plan = plan(&original, &[rotate()], &context).unwrap();
        let mut objects = BTreeMap::new();
        apply(&mut plan, &original, &mut objects, &context).unwrap();
        let mut other = original.parsed().object(reference(4)).unwrap().clone();
        let Value::Dict(dictionary) = &mut other.value else {
            unreachable!()
        };
        dictionary.insert(b"Rotate".to_vec(), Value::Integer(90));
        objects.insert(reference(4), other);
        let report = verify(
            &original,
            &stage(&original, &objects),
            &plan,
            &objects,
            &context,
        )
        .unwrap();
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code == code("unselected_geometry")
                    && check.status == CheckStatus::Failed)
        );

        objects.remove(&reference(4));
        let mut corrupted = stage(&original, &objects).parsed().clone();
        let Value::Dict(dictionary) = &mut corrupted.objects.get_mut(&reference(3)).unwrap().value
        else {
            unreachable!()
        };
        dictionary.insert(b"Unexpected".to_vec(), Value::Bool(true));
        let report = verify(
            &original,
            &original.with_parsed(corrupted),
            &plan,
            &objects,
            &context,
        )
        .unwrap();
        assert!(report.checks().iter().any(|check| check.code == code("page_keys") && check.status == CheckStatus::Failed));
    }

    #[test]
    fn geometry_absent_crop_stays_absent_unless_composition_materialized_it() {
        let original = fixture();
        let context = context();
        let request = PdfPageTransformRequest {
            page_index: 0,
            transform: PageTransform {
                media_box: Some([0.0, 0.0, 400.0, 500.0]),
                ..Default::default()
            },
        };
        let mut ordinary = plan(&original, std::slice::from_ref(&request), &context).unwrap();
        let mut objects = BTreeMap::new();
        apply(&mut ordinary, &original, &mut objects, &context).unwrap();
        let Value::Dict(dictionary) = &objects[&reference(3)].value else {
            unreachable!()
        };
        assert!(!dictionary.contains_key(b"CropBox".as_slice()));
        let candidate = stage(&original, &objects);
        assert_eq!(
            read_for(&candidate, reference(3), &context)
                .unwrap()
                .crop_box,
            [0.0, 0.0, 400.0, 500.0]
        );

        let mut composed = plan(&original, &[request], &context).unwrap();
        let Value::Dict(dictionary) = &mut objects.get_mut(&reference(3)).unwrap().value else {
            unreachable!()
        };
        dictionary.insert(
            b"CropBox".to_vec(),
            rectangle_value([0.0, 0.0, 200.0, 300.0]),
        );
        apply(&mut composed, &original, &mut objects, &context).unwrap();
        verify(
            &original,
            &stage(&original, &objects),
            &composed,
            &objects,
            &context,
        )
        .unwrap()
        .validate(&required_checks())
        .unwrap();
        let Value::Dict(dictionary) = &objects[&reference(3)].value else {
            unreachable!()
        };
        assert_eq!(
            dictionary.get(b"CropBox".as_slice()),
            Some(&rectangle_value([0.0, 0.0, 400.0, 500.0]))
        );
    }

    #[test]
    fn geometry_source_binding_and_invalid_source_fail_closed() {
        let original = fixture();
        let context = context();
        let mut plan = plan(&original, &[rotate()], &context).unwrap();
        let mut parsed = original.parsed().clone();
        let Value::Dict(dictionary) = &mut parsed.objects.get_mut(&reference(4)).unwrap().value
        else {
            unreachable!()
        };
        dictionary.insert(b"Rotate".to_vec(), Value::Integer(45));
        let branch = original.with_parsed(parsed);
        assert!(apply(&mut plan, &branch, &mut BTreeMap::new(), &context).is_err());
        // Even though page 4 is unselected, malformed source rotation cannot be
        // normalized into a misleading successful preservation check.
        assert!(super::plan(&branch, &[rotate()], &context).is_err());

        let mut wrong_parent = original.parsed().clone();
        // Give page 4 a plausible but different Pages parent. Its inherited
        // geometry still reads correctly; actual Kids containment must refuse.
        let mut disconnected = original.parsed().object(reference(2)).unwrap().clone();
        let Value::Dict(dictionary) = &mut disconnected.value else {
            unreachable!()
        };
        dictionary.insert(
            b"Kids".to_vec(),
            Value::Array(vec![Value::Ref(reference(4))]),
        );
        dictionary.insert(b"Count".to_vec(), Value::Integer(1));
        wrong_parent.objects.insert(reference(5), disconnected);
        let Value::Dict(dictionary) =
            &mut wrong_parent.objects.get_mut(&reference(4)).unwrap().value
        else {
            unreachable!()
        };
        dictionary.insert(b"Parent".to_vec(), Value::Ref(reference(5)));
        let wrong_parent = original.with_parsed(wrong_parent);
        assert_eq!(
            read_for(&wrong_parent, reference(4), &context).unwrap(),
            read_for(&original, reference(4), &context).unwrap()
        );
        assert!(super::plan(&wrong_parent, &[rotate()], &context).is_err());

        let mut wrong_count = original.parsed().clone();
        let Value::Dict(dictionary) =
            &mut wrong_count.objects.get_mut(&reference(2)).unwrap().value
        else {
            unreachable!()
        };
        dictionary.insert(b"Count".to_vec(), Value::Integer(3));
        assert!(super::plan(&original.with_parsed(wrong_count), &[rotate()], &context).is_err());
    }
}
