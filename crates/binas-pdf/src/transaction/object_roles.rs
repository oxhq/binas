//! Staged object identities must belong to a declared operation's source roles.
//! Matching the replacement map is not sufficient authorization to change an
//! arbitrary Info, bookmark or opaque object. Per-role checks constrain keys.

use std::collections::BTreeSet;

use binas_core::{OperationContext, Reservation};

use super::{Intent, PdfSharedEditMode};
use crate::{
    PdfDocument, PdfError,
    object::ObjectRef,
    preservation::{labels, pages},
};

pub(super) fn verify(
    original: &PdfDocument,
    candidate: &PdfDocument,
    intent: &Intent,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if original.source_id() != candidate.source_id() {
        return Ok(false);
    }
    let limits = &original.parsed().limits;
    if intent.objects.len() > limits.max_objects
        || intent.streams.len() > limits.max_container_items
        || intent.annotations.len() > limits.max_container_items
        || intent
            .checkboxes
            .as_ref()
            .is_some_and(|plan| plan.effects().len() > limits.max_container_items)
        || intent
            .geometry
            .as_ref()
            .is_some_and(|plan| plan.effects().len() > limits.max_container_items)
    {
        return Err(PdfError::limit(
            "operation object roles exceed configured limits",
        ));
    }
    let mut allowed = BTreeSet::new();
    let mut reservations = Vec::new();
    {
        let mut include = |reference| {
            search_work(allowed.len(), context)?;
            if allowed.contains(&reference) {
                return Ok(());
            }
            if allowed.len() >= limits.max_objects {
                return Err(PdfError::limit(
                    "authorized object count exceeds max_objects",
                ));
            }
            // Logical references and their reservation records; BTree nodes,
            // allocator overhead and spare Vec capacity are not process memory.
            let reservation = context
                .reserve(
                    (std::mem::size_of::<ObjectRef>() + std::mem::size_of::<Reservation>()) as u64,
                )
                .map_err(PdfError::from_context)?;
            reservations
                .try_reserve(1)
                .map_err(|_| PdfError::limit("object role reservation allocation failed"))?;
            reservations.push(reservation);
            allowed.insert(reference);
            Ok(())
        };
        for (output, stream) in &intent.streams {
            context.charge_work(1).map_err(PdfError::from_context)?;
            let declared = ObjectRef {
                number: stream.effect.output_stream.number,
                generation: stream.effect.output_stream.generation,
            };
            if *output != declared {
                return Ok(false);
            }
            include(declared)?;
            if stream.effect.source_stream != stream.effect.output_stream {
                if stream.effect.shared != PdfSharedEditMode::CloneForPage
                    || stream.owners.len() != 1
                {
                    return Ok(false);
                }
                // A clone also replaces its sole original page Contents slot.
                // page_keys proves the specific source slot and fresh target.
                include(stream.owners[0].page)?;
            }
        }
        for reference in intent.annotations.keys() {
            include(*reference)?;
        }
        if let Some(checkboxes) = &intent.checkboxes {
            for effect in checkboxes.effects() {
                include(ObjectRef {
                    number: effect.field.number,
                    generation: effect.field.generation,
                })?;
                include(ObjectRef {
                    number: effect.widget.number,
                    generation: effect.widget.generation,
                })?;
            }
        }
        if let Some(geometry) = &intent.geometry {
            for effect in geometry.effects() {
                include(ObjectRef {
                    number: effect.page.number,
                    generation: effect.page.generation,
                })?;
            }
        }
        if let Some(flatten) = &intent.flatten {
            for reference in flatten.changed_refs() {
                include(reference)?;
            }
        }
        if let Some(text_fields) = &intent.text_fields {
            for reference in text_fields.changed_refs() {
                include(reference)?;
            }
        }
        if let Some(duplicate) = &intent.duplicate {
            for reference in duplicate.changed_refs() {
                include(reference)?;
            }
        }
        if let Some(delete) = &intent.delete {
            for reference in delete.changed_refs() {
                include(reference)?;
            }
        }
        if intent.reorder.is_some() {
            pages::visit_tree_objects(original, context, &mut include)?;
        }
        if intent.labels.is_some() {
            labels::visit_tree_objects(original, context, &mut include)?;
        }
    }
    for reference in intent.objects.keys() {
        search_work(allowed.len(), context)?;
        if !allowed.contains(reference) {
            return Ok(false);
        }
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(true)
}

fn search_work(records: usize, context: &OperationContext) -> Result<(), PdfError> {
    context
        .charge_work(u64::from(usize::BITS - records.leading_zeros()) + 1)
        .map_err(PdfError::from_context)
}

#[cfg(test)]
mod tests {
    use super::super::{PdfRevisionPlan, required, verify_candidate};
    use super::*;
    use crate::{
        AnnotationContentsMutationRequest, OpenOptions, PageTransform, PdfEngine, PdfErrorCode,
        PdfPageTextEditRequest, PdfPageTransformRequest, PdfRevision, PdfRevisionRequest,
        SurgicalTextEditRequest, object::Value, preservation::geometry,
    };
    use binas_core::{
        BudgetLimits, CancellationToken, ResourceBudget, RevisionId,
        transaction::{PlanEnvelope, TransactionError},
        verification::CheckStatus,
    };
    use std::collections::BTreeMap;

    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 4 * 1024 * 1024,
                max_work_units: 16 * 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }

    fn fixture() -> PdfDocument {
        let content = "BT (A) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [0 << /S /D /P (p-) /Vendor 8 0 R >>] /Opaque (keep) >> /Vendor 8 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(),
            "<< /Type /Page /Parent 2 0 R /Contents 5 0 R /Annots [6 0 R] >>".into(),
            "<< /Type /Page /Parent 2 0 R >>".into(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Annot /Subtype /Text /P 3 0 R /Rect [1 2 20 30] /Contents (old) >>".into(),
            "<< /Title (Original title) >>".into(),
            "<< /Title (Opaque original) >>".into(),
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
            format!("trailer\n<< /Size 9 /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    fn empty_intent() -> Intent {
        Intent {
            objects: BTreeMap::new(),
            streams: BTreeMap::new(),
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
        }
    }

    fn candidate(original: &PdfDocument, intent: &Intent) -> PdfDocument {
        let mut parsed = original.parsed().clone();
        for (reference, object) in &intent.objects {
            parsed.objects.insert(*reference, object.clone());
        }
        original.with_parsed(parsed)
    }

    #[test]
    fn self_consistent_info_and_opaque_changes_cannot_commit_with_correct_geometry() {
        for number in [7, 8] {
            let original = fixture();
            let mut revision = PdfRevision::new(original.clone());
            let snapshot = revision.snapshot();
            let context = context();
            let mut intent = empty_intent();
            let mut geometry = geometry::plan(
                &original,
                &[PdfPageTransformRequest {
                    page_index: 0,
                    transform: PageTransform {
                        rotation_degrees: Some(90),
                        ..Default::default()
                    },
                }],
                &context,
            )
            .unwrap();
            geometry::apply(&mut geometry, &original, &mut intent.objects, &context).unwrap();
            intent.geometry = Some(geometry);
            assert!(verify(&original, &candidate(&original, &intent), &intent, &context).unwrap());
            let mut extra = original.parsed().object(reference(number)).unwrap().clone();
            let Value::Dict(dictionary) = &mut extra.value else {
                panic!()
            };
            dictionary.remove(b"Title".as_slice());
            intent.objects.insert(reference(number), extra);
            let damaged = candidate(&original, &intent);
            let evidence = verify_candidate(&original, &damaged, &intent, &context).unwrap();
            assert!(
                evidence
                    .checks()
                    .iter()
                    .any(|check| check.code.code() == "expected_changes"
                        && check.status == CheckStatus::Passed)
            );
            assert!(
                evidence
                    .checks()
                    .iter()
                    .any(|check| check.code.code() == "authorized_objects"
                        && check.status == CheckStatus::Failed)
            );
            let checks = required(&intent);
            assert!(evidence.validate(&checks).is_err());
            let plan = PdfRevisionPlan {
                envelope: PlanEnvelope::new(snapshot, intent, checks),
                store: original.parsed().objects.clone(),
                _decoded_reservations: Vec::new(),
            };
            assert!(matches!(
                revision.commit(&plan, &context),
                Err(TransactionError::Evidence(_))
            ));
            assert_eq!(revision.snapshot(), snapshot);
            assert_eq!(revision.page_geometry(0).unwrap().rotation_degrees, 0);
            assert_eq!(revision.text().unwrap(), ["A"]);
            assert_eq!(
                revision
                    .state
                    .value()
                    .parsed()
                    .object(reference(number))
                    .unwrap()
                    .value,
                original.parsed().object(reference(number)).unwrap().value
            );
        }
    }

    #[test]
    fn combined_text_note_geometry_reorder_and_direct_labels_are_authorized() {
        let mut revision = PdfRevision::new(fixture());
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    text: vec![SurgicalTextEditRequest {
                        old_text: "A".into(),
                        replacement: "B".into(),
                        match_index: 0,
                    }],
                    annotations: vec![AnnotationContentsMutationRequest {
                        annotation_index: 0,
                        contents: "new".into(),
                    }],
                    reorder_pages: Some(vec![1, 0]),
                    page_transforms: vec![PdfPageTransformRequest {
                        page_index: 0,
                        transform: PageTransform {
                            rotation_degrees: Some(90),
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        revision.commit(&plan, &context).unwrap();
        assert_eq!(revision.snapshot().revision(), RevisionId::new(1));
        assert_eq!(revision.text().unwrap(), ["B"]);
        assert_eq!(revision.page_geometry(1).unwrap().rotation_degrees, 90);
        assert_eq!(
            revision
                .page_labels()
                .unwrap()
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["p-2", "p-1"]
        );
    }

    #[test]
    fn page_local_stream_clone_is_authorized_without_reorder_or_geometry() {
        let source = PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/content-sequence.pdf"),
                OpenOptions::default(),
            )
            .unwrap();
        let mut revision = PdfRevision::new(source);
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    page_text: vec![PdfPageTextEditRequest {
                        page_index: 0,
                        edit: SurgicalTextEditRequest {
                            old_text: "Ω".into(),
                            replacement: "Ψ".into(),
                            match_index: 0,
                        },
                        shared: PdfSharedEditMode::CloneForPage,
                    }],
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        revision.commit(&plan, &context).unwrap();
        assert_eq!(
            revision.text().unwrap(),
            ["Ψ", "Ω", "Ж", "Ψ", "Ж", "Ж", "Ж", "Ю"]
        );
    }

    #[test]
    fn label_authorization_stops_at_structural_objects_not_opaque_references() {
        let original = fixture();
        let context = context();
        let mut intent = empty_intent();
        let order = [1, 0];
        let pages = original.page_refs().unwrap();
        let reorder = pages::plan_reorder(&original, &[pages[1], pages[0]], &context).unwrap();
        let labels = labels::plan_reordered_labels(&original, &order, &context).unwrap();
        intent.objects.extend(reorder.replacements.clone());
        intent.objects.extend(labels.changes.clone());
        intent.reorder = Some(reorder);
        intent.labels = Some(labels);
        assert!(verify(&original, &candidate(&original, &intent), &intent, &context).unwrap());
        // Object 8 is reachable from both catalog and raw label specifications.
        // Those references do not turn the opaque dictionary into a label node.
        intent.objects.insert(
            reference(8),
            original.parsed().object(reference(8)).unwrap().clone(),
        );
        assert!(!verify(&original, &candidate(&original, &intent), &intent, &context).unwrap());
    }

    #[test]
    fn authorization_work_and_memory_reservations_remain_bounded() {
        let original = fixture();
        let ctx = context();
        let mut intent = empty_intent();
        intent.annotations.insert(reference(6), b"new".to_vec());
        intent.objects.insert(
            reference(6),
            original.parsed().object(reference(6)).unwrap().clone(),
        );
        assert!(verify(&original, &original, &intent, &ctx).unwrap());
        assert_eq!(ctx.budget().snapshot().live_bytes, 0);
        let limited = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: u64::MAX,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            verify(&original, &original, &intent, &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let work = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: u64::MAX,
                max_work_units: 1,
            }),
            CancellationToken::new(),
        );
        for _ in 0..2 {
            assert_eq!(
                verify(&original, &original, &intent, &work)
                    .unwrap_err()
                    .code,
                PdfErrorCode::ResourceLimit
            );
        }
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert_eq!(
            verify(&original, &original, &intent, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
    }
}
