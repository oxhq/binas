//! Page preservation derived from source dictionaries and declared stream slots.
//! Staged replacement dictionaries are deliberately not preservation evidence.

use std::collections::BTreeMap;

use binas_core::{OperationContext, Reservation};

use super::{Intent, PdfSharedEditMode};
use crate::{
    PdfDocument, PdfError,
    object::{ObjectRef, Value},
    ownership::PageOwnedSlot,
};

type Substitutions = BTreeMap<(ObjectRef, PageOwnedSlot), (ObjectRef, ObjectRef)>;

pub(super) fn verify(
    original: &PdfDocument,
    candidate: &PdfDocument,
    intent: &Intent,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if original.source_id() != candidate.source_id() {
        return Ok(false);
    }
    // The legacy page walk has configured graph limits. Subsequent comparisons
    // poll the operation context and account logical temporary records.
    context
        .charge_work(original.parsed().objects.len() as u64)
        .map_err(PdfError::from_context)?;
    let pages = original.page_refs()?;
    let mut reservations = vec![reserve(
        pages.len().checked_mul(std::mem::size_of::<ObjectRef>()),
        context,
    )?];
    let mut substitutions = Substitutions::new();
    for (output, stream) in &intent.streams {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let source = ObjectRef {
            number: stream.effect.source_stream.number,
            generation: stream.effect.source_stream.generation,
        };
        if output.number != stream.effect.output_stream.number
            || output.generation != stream.effect.output_stream.generation
        {
            return Ok(false);
        }
        if source == *output {
            continue;
        }
        if stream.effect.shared != PdfSharedEditMode::CloneForPage
            || stream.owners.len() != 1
            || original.parsed().objects.contains_key(output)
        {
            return Ok(false);
        }
        let owner = stream.owners[0];
        if substitutions.len() >= original.parsed().limits.max_container_items {
            return Err(PdfError::limit("page content substitutions exceed limit"));
        }
        reservations.push(reserve(
            Some(std::mem::size_of::<(
                (ObjectRef, PageOwnedSlot),
                (ObjectRef, ObjectRef),
            )>()),
            context,
        )?);
        if substitutions
            .insert((owner.page, owner.slot), (source, *output))
            .is_some()
        {
            return Ok(false);
        }
    }
    let mut applied = 0;
    let depth_limit = original.parsed().limits.max_parser_depth;
    for page in pages {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let before = original.parsed().object(page)?;
        let after = candidate.parsed().object(page)?;
        let (Value::Dict(before_keys), Value::Dict(after_keys)) = (&before.value, &after.value)
        else {
            return Ok(false);
        };
        context
            .charge_work(before.stream.as_deref().map_or(0, |bytes| bytes.len()) as u64)
            .map_err(PdfError::from_context)?;
        if before.stream.as_deref() != after.stream.as_deref() {
            return Ok(false);
        }
        let permitted = |key: &[u8]| {
            intent
                .reorder
                .as_ref()
                .is_some_and(|plan| plan.allows_key(page, key))
                || intent
                    .geometry
                    .as_ref()
                    .is_some_and(|plan| plan.allows_key(page, key))
                || intent
                    .flatten
                    .as_ref()
                    .is_some_and(|plan| plan.allows_page_key(page, key))
        };
        for (key, value) in before_keys {
            context
                .charge_work(key.len() as u64 + 1)
                .map_err(PdfError::from_context)?;
            if permitted(key) {
                continue;
            }
            let Some(actual) = after_keys.get(key) else {
                return Ok(false);
            };
            let equal = if key == b"Contents" {
                contents_equal(
                    page,
                    value,
                    actual,
                    &substitutions,
                    &mut applied,
                    depth_limit,
                    context,
                )?
            } else {
                value_equal(value, actual, 0, depth_limit, context)?
            };
            if !equal {
                return Ok(false);
            }
        }
        for key in after_keys.keys() {
            context
                .charge_work(key.len() as u64 + 1)
                .map_err(PdfError::from_context)?;
            if !before_keys.contains_key(key) && !permitted(key) {
                return Ok(false);
            }
        }
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    // Reject declarations for missing pages, absent Contents or wrong slot kinds.
    Ok(applied == substitutions.len())
}

fn contents_equal(
    page: ObjectRef,
    before: &Value,
    after: &Value,
    substitutions: &Substitutions,
    applied: &mut usize,
    depth_limit: usize,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    let mut slot_equal = |slot, before: &Value, after: &Value| -> Result<bool, PdfError> {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if let Some((source, output)) = substitutions.get(&(page, slot)) {
            *applied += 1;
            Ok(before == &Value::Ref(*source) && after == &Value::Ref(*output))
        } else {
            value_equal(before, after, 0, depth_limit, context)
        }
    };
    match (before, after) {
        (Value::Array(before), Value::Array(after)) if before.len() == after.len() => {
            for (index, (before, after)) in before.iter().zip(after).enumerate() {
                if !slot_equal(PageOwnedSlot::ArrayIndex(index), before, after)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Array(_), _) | (_, Value::Array(_)) => Ok(false),
        _ => slot_equal(PageOwnedSlot::Scalar, before, after),
    }
}

pub(super) fn value_equal(
    before: &Value,
    after: &Value,
    depth: usize,
    limit: usize,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > limit {
        return Err(PdfError::limit(
            "page preservation value depth exceeds limit",
        ));
    }
    match (before, after) {
        (Value::Array(before), Value::Array(after)) if before.len() == after.len() => {
            for (before, after) in before.iter().zip(after) {
                if !value_equal(before, after, depth + 1, limit, context)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Dict(before), Value::Dict(after)) if before.len() == after.len() => {
            for ((key, before), (other_key, after)) in before.iter().zip(after) {
                context
                    .charge_work(key.len() as u64)
                    .map_err(PdfError::from_context)?;
                if key != other_key || !value_equal(before, after, depth + 1, limit, context)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Array(_) | Value::Dict(_), _) | (_, Value::Array(_) | Value::Dict(_)) => Ok(false),
        (Value::Name(bytes) | Value::String(bytes), _) => {
            context
                .charge_work(bytes.len() as u64)
                .map_err(PdfError::from_context)?;
            Ok(before == after)
        }
        _ => Ok(before == after),
    }
}

fn reserve(bytes: Option<usize>, context: &OperationContext) -> Result<Reservation, PdfError> {
    context
        .reserve(
            bytes.ok_or_else(|| PdfError::limit("page preservation workspace overflows"))? as u64,
        )
        .map_err(PdfError::from_context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        OpenOptions, PageTransform, PdfEngine, PdfPageTextEditRequest, PdfPageTransformRequest,
        PdfRevision, PdfRevisionRequest, SurgicalTextEditRequest,
        object::IndirectObject,
        preservation::{geometry, pages},
    };
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 16 * 1024 * 1024,
                max_work_units: 64 * 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }

    fn fixture(array: bool) -> PdfDocument {
        let content = b"BT (before) Tj ET";
        let paint = b"q 1 0 0 rg 10 10 20 20 re f Q";
        let stream = |bytes: &[u8]| {
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                bytes.len(),
                std::str::from_utf8(bytes).unwrap()
            )
        };
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] /Resources 8 0 R >>".into(),
            format!(
                "<< /Type /Page /Parent 2 0 R /Resources 8 0 R /Contents {} /Annots [7 0 R] /StructParents 12 /Private << /Keep (opaque) >> >>",
                if array { "[5 0 R 6 0 R]" } else { "5 0 R" }
            ),
            "<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".into(),
            stream(content),
            stream(paint),
            "<< /Type /Annot /Subtype /Text /P 3 0 R /Rect [1 2 3 4] /Contents (retained) >>".into(),
            "<< /ProcSet [/PDF] >>".into(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 9\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 9 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    fn candidate(
        original: &PdfDocument,
        objects: &BTreeMap<ObjectRef, IndirectObject>,
    ) -> PdfDocument {
        let mut parsed = original.parsed().clone();
        for (reference, object) in objects {
            parsed.objects.insert(*reference, object.clone());
        }
        original.with_parsed(parsed)
    }

    fn geometry_request() -> Vec<PdfPageTransformRequest> {
        vec![PdfPageTransformRequest {
            page_index: 0,
            transform: PageTransform {
                rotation_degrees: Some(90),
                ..Default::default()
            },
        }]
    }

    #[test]
    fn geometry_rejects_self_consistent_resource_and_annotation_loss() {
        let original = fixture(true);
        let revision = PdfRevision::new(original.clone());
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    page_transforms: geometry_request(),
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        let source_intent = plan.envelope.intent();
        assert!(
            verify(
                &original,
                &candidate(&original, &source_intent.objects),
                source_intent,
                &context
            )
            .unwrap()
        );
        for key in [
            b"Resources".as_slice(),
            b"Annots",
            b"StructParents",
            b"Private",
        ] {
            let mut objects = source_intent.objects.clone();
            let Value::Dict(page) = &mut objects
                .get_mut(&ObjectRef {
                    number: 3,
                    generation: 0,
                })
                .unwrap()
                .value
            else {
                panic!("page dictionary");
            };
            assert!(page.remove(key).is_some());
            let tampered = candidate(&original, &objects);
            let dishonest_intent = Intent {
                objects,
                streams: BTreeMap::new(),
                annotations: BTreeMap::new(),
                reorder: None,
                labels: None,
                text_pages: BTreeMap::new(),
                font_effects: Vec::new(),
                geometry: Some(geometry::plan(&original, &geometry_request(), &context).unwrap()),
                checkboxes: None,
                flatten: None,
                text_fields: None,
                duplicate: None,
                delete: None,
            };
            assert_eq!(
                tampered
                    .parsed()
                    .object(ObjectRef {
                        number: 3,
                        generation: 0
                    })
                    .unwrap()
                    .value,
                dishonest_intent.objects[&ObjectRef {
                    number: 3,
                    generation: 0
                }]
                    .value
            );
            assert!(
                !verify(&original, &tampered, &dishonest_intent, &context).unwrap(),
                "{key:?}"
            );
        }
    }

    #[test]
    fn rotation_only_rejects_an_invented_crop_with_identical_effective_geometry() {
        let original = fixture(true);
        let revision = PdfRevision::new(original.clone());
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    page_transforms: geometry_request(),
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        let intent = plan.envelope.intent();
        let clean = candidate(&original, &intent.objects);
        assert!(verify(&original, &clean, intent, &context).unwrap());
        let page_reference = ObjectRef {
            number: 3,
            generation: 0,
        };
        let mut objects = intent.objects.clone();
        let Value::Dict(page) = &mut objects.get_mut(&page_reference).unwrap().value else {
            panic!("page dictionary");
        };
        // The source CropBox is absent and rotation does not request one. Adding
        // an explicit copy of MediaBox hides that omission from effective-value
        // checks while changing the behavior of a later MediaBox-only request.
        assert!(!page.contains_key(b"CropBox".as_slice()));
        page.insert(
            b"CropBox".to_vec(),
            Value::Array([0, 0, 200, 200].into_iter().map(Value::Integer).collect()),
        );
        let tampered = candidate(&original, &objects);
        assert_eq!(
            tampered.page_geometry(0).unwrap(),
            clean.page_geometry(0).unwrap()
        );
        let dishonest_intent = Intent {
            objects,
            streams: BTreeMap::new(),
            annotations: BTreeMap::new(),
            reorder: None,
            labels: None,
            text_pages: BTreeMap::new(),
            font_effects: Vec::new(),
            geometry: Some(geometry::plan(&original, &geometry_request(), &context).unwrap()),
            checkboxes: None,
            flatten: None,
            text_fields: None,
            duplicate: None,
            delete: None,
        };
        assert!(!verify(&original, &tampered, &dishonest_intent, &context).unwrap());
    }

    #[test]
    fn same_parent_reorder_rejects_invented_equal_effective_crop_and_resources() {
        let original = fixture(true);
        let revision = PdfRevision::new(original.clone());
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    reorder_pages: Some(vec![1, 0]),
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        let intent = plan.envelope.intent();
        let clean = candidate(&original, &intent.objects);
        assert!(verify(&original, &clean, intent, &context).unwrap());
        let page_reference = ObjectRef {
            number: 4,
            generation: 0,
        };
        let requested_order = [
            page_reference,
            ObjectRef {
                number: 3,
                generation: 0,
            },
        ];
        for (key, value) in [
            (
                b"CropBox".as_slice(),
                Value::Array([0, 0, 200, 200].into_iter().map(Value::Integer).collect()),
            ),
            (
                b"Resources",
                Value::Ref(ObjectRef {
                    number: 8,
                    generation: 0,
                }),
            ),
        ] {
            let mut objects = intent.objects.clone();
            let object = objects
                .entry(page_reference)
                .or_insert_with(|| original.parsed().object(page_reference).unwrap().clone());
            let Value::Dict(page) = &mut object.value else {
                panic!("page dictionary");
            };
            // Both leaves keep their original parent. The reorder changes Kids
            // only; an equal effective value does not authorize a new leaf key.
            assert!(!page.contains_key(key));
            page.insert(key.to_vec(), value);
            let tampered = candidate(&original, &objects);
            assert_eq!(
                tampered.page_geometry(0).unwrap(),
                clean.page_geometry(0).unwrap()
            );
            assert_eq!(
                tampered.inherited_resources(page_reference).unwrap(),
                clean.inherited_resources(page_reference).unwrap(),
            );
            let dishonest_intent = Intent {
                objects,
                streams: BTreeMap::new(),
                annotations: BTreeMap::new(),
                reorder: Some(pages::plan_reorder(&original, &requested_order, &context).unwrap()),
                labels: None,
                text_pages: BTreeMap::new(),
                font_effects: Vec::new(),
                geometry: None,
                checkboxes: None,
                flatten: None,
                text_fields: None,
                duplicate: None,
                delete: None,
            };
            assert!(
                !verify(&original, &tampered, &dishonest_intent, &context).unwrap(),
                "{key:?}"
            );
        }
    }

    #[test]
    fn local_clone_rejects_dropped_nontext_contents_and_preserves_slot_shape() {
        for array in [false, true] {
            let original = fixture(array);
            let revision = PdfRevision::new(original.clone());
            let context = context();
            let plan = revision
                .plan(
                    PdfRevisionRequest {
                        page_text: vec![PdfPageTextEditRequest {
                            page_index: 0,
                            edit: SurgicalTextEditRequest {
                                old_text: "before".into(),
                                replacement: "after".into(),
                                match_index: 0,
                            },
                            shared: PdfSharedEditMode::CloneForPage,
                        }],
                        ..Default::default()
                    },
                    &context,
                )
                .unwrap();
            let intent = plan.envelope.intent();
            assert!(
                verify(
                    &original,
                    &candidate(&original, &intent.objects),
                    intent,
                    &context
                )
                .unwrap()
            );
            let mut objects = intent.objects.clone();
            let Value::Dict(page) = &mut objects
                .get_mut(&ObjectRef {
                    number: 3,
                    generation: 0,
                })
                .unwrap()
                .value
            else {
                panic!("page dictionary");
            };
            let contents = page.get_mut(b"Contents".as_slice()).unwrap();
            if array {
                let Value::Array(streams) = contents else {
                    panic!("Contents array");
                };
                assert_eq!(
                    streams.pop(),
                    Some(Value::Ref(ObjectRef {
                        number: 6,
                        generation: 0
                    }))
                );
            } else {
                *contents = Value::Array(vec![contents.clone()]);
            }
            let tampered = candidate(&original, &objects);
            let dishonest_intent = Intent {
                objects,
                streams: intent
                    .streams
                    .iter()
                    .map(|(reference, stream)| {
                        (
                            *reference,
                            super::super::StreamIntent {
                                expected: stream.expected.clone(),
                                owners: stream.owners.clone(),
                                effect: stream.effect.clone(),
                            },
                        )
                    })
                    .collect(),
                annotations: BTreeMap::new(),
                reorder: None,
                labels: None,
                text_pages: BTreeMap::new(),
                font_effects: Vec::new(),
                geometry: None,
                checkboxes: None,
                flatten: None,
                text_fields: None,
                duplicate: None,
                delete: None,
            };
            assert!(!verify(&original, &tampered, &dishonest_intent, &context).unwrap());
        }
    }
}
