//! Logical page deletion reconciles retained navigation and form registration.
//! Original unreachable objects remain stored; this operation is not redaction.
use std::collections::BTreeSet;

use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget,
    transaction::TransactionError,
};
use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PageTransform, PdfDocument, PdfDuplicateFieldName,
    PdfEngine, PdfErrorCode, PdfExcludedPageTargetPolicy, PdfPageDeleteRequest,
    PdfPageDuplicateRequest, PdfPageTransformRequest, PdfReferenceIndex, PdfReferenceOrigin,
    PdfReferencePathStep, PdfReferenceTarget, PdfRevision, PdfRevisionPlan, PdfRevisionRequest,
    PdfTextFieldFillRequest, PdfWriterMode, list_annotations, list_form_fields, list_streams,
    read_decoded_stream, read_document_info, read_embedded_attachment_bytes,
    read_embedded_attachments, read_named_destinations, read_outlines, read_page_labels,
    read_xmp_metadata,
};

const SOURCE: &[u8] = include_bytes!("../../../qa/preservation/fixtures/text-field-fill.pdf");

fn context() -> OperationContext {
    limited_context(128 * 1024 * 1024, 512 * 1024 * 1024)
}
fn limited_context(memory: u64, work: u64) -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: memory,
            max_work_units: work,
        }),
        CancellationToken::new(),
    )
}
fn open(bytes: &[u8]) -> PdfDocument {
    PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap()
}
fn reference(number: u32) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number,
        generation: 0,
    }
}
fn request(page_index: usize, excluded_targets: PdfExcludedPageTargetPolicy) -> PdfRevisionRequest {
    PdfRevisionRequest {
        delete_page: Some(PdfPageDeleteRequest {
            page_index,
            excluded_targets,
        }),
        ..Default::default()
    }
}
fn delete_bravo() -> PdfRevisionRequest {
    request(
        1,
        PdfExcludedPageTargetPolicy::RemapToPage { page_index: 0 },
    )
}
fn fill(field_name: &str, value: &str) -> PdfRevisionRequest {
    PdfRevisionRequest {
        text_fields: vec![PdfTextFieldFillRequest {
            field_name: field_name.into(),
            value: value.into(),
            match_index: 0,
        }],
        ..Default::default()
    }
}
fn key(name: &[u8]) -> PdfReferencePathStep {
    PdfReferencePathStep::Key(name.to_vec())
}

fn assert_effect(plan: &PdfRevisionPlan) {
    let effect = plan.delete_effect().unwrap();
    assert_eq!(effect.page, reference(4));
    assert_eq!(effect.page_index, 1);
    assert_eq!(effect.remaining_pages, [reference(3)]);
    assert_eq!(
        effect.removed_widgets,
        [reference(10), reference(12), reference(13)]
    );
    assert_eq!(effect.unregistered_fields, [reference(12), reference(13)]);
    assert_eq!(effect.remapped_destinations.len(), 3);
    for (object, path) in [
        (18, vec![key(b"Dest"), PdfReferencePathStep::Index(0)]),
        (26, vec![key(b"Dest"), PdfReferencePathStep::Index(0)]),
        (
            27,
            vec![
                key(b"Dests"),
                key(b"Names"),
                PdfReferencePathStep::Index(1),
                PdfReferencePathStep::Index(0),
            ],
        ),
    ] {
        let remap = effect
            .remapped_destinations
            .iter()
            .find(|remap| remap.object == reference(object) && remap.path == path)
            .unwrap();
        assert_eq!(remap.from, reference(4));
        assert_eq!(remap.to, reference(3));
    }
}

fn reachable(index: &PdfReferenceIndex) -> BTreeSet<PdfReferenceTarget> {
    let mut seen = BTreeSet::new();
    let mut pending = index
        .records()
        .iter()
        .filter(|edge| edge.origin == PdfReferenceOrigin::Trailer)
        .map(|edge| edge.target)
        .collect::<Vec<_>>();
    while let Some(object) = pending.pop() {
        if seen.insert(object) {
            pending.extend(
                index
                    .records()
                    .iter()
                    .filter(|edge| edge.origin == PdfReferenceOrigin::Object(object))
                    .map(|edge| edge.target),
            );
        }
    }
    seen
}

/// Derive expected inventories from the separately opened pre-deletion source.
/// This checks requested omissions instead of comparing staging to reparsing.
fn assert_outputs(revision: &PdfRevision, baseline: &PdfDocument) {
    let mut fields = list_form_fields(baseline).unwrap();
    fields.retain(|field| ![Some(12), Some(13)].contains(&field.object_number));
    for (index, field) in fields.iter_mut().enumerate() {
        field.index = index;
        field
            .widget_refs
            .retain(|widget| widget.object_number != 10);
    }
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["group", "group.name", "group.keep"]
    );
    let mut annotations = list_annotations(baseline).unwrap();
    annotations.retain(|annotation| annotation.page_index == 0);
    for (index, annotation) in annotations.iter_mut().enumerate() {
        annotation.index = index;
    }
    assert_eq!(revision.page_ids().unwrap(), [reference(3)]);
    assert_eq!(revision.annotations().unwrap(), annotations);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &context()).unwrap();
        assert_eq!(output.snapshot, revision.snapshot());
        assert_eq!(output.mode, mode);
        if mode == PdfWriterMode::Incremental {
            assert!(output.output.as_bytes().starts_with(SOURCE));
        }
        let reopened = open(output.output.as_bytes());
        assert_eq!(
            PdfRevision::new(reopened.clone()).page_ids().unwrap(),
            [reference(3)]
        );
        assert_eq!(list_form_fields(&reopened).unwrap(), fields);
        assert_eq!(list_annotations(&reopened).unwrap(), annotations);
        assert_eq!(
            reopened.page_geometry(0).unwrap(),
            baseline.page_geometry(0).unwrap()
        );
        assert_eq!(
            reopened
                .extract_page_text_spans(0)
                .unwrap()
                .spans
                .iter()
                .map(|span| &span.text)
                .collect::<Vec<_>>(),
            baseline
                .extract_page_text_spans(0)
                .unwrap()
                .spans
                .iter()
                .map(|span| &span.text)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            read_document_info(&reopened).unwrap(),
            read_document_info(baseline).unwrap()
        );
        assert_eq!(
            read_xmp_metadata(&reopened).unwrap(),
            read_xmp_metadata(baseline).unwrap()
        );
        assert_eq!(
            read_outlines(&reopened).unwrap(),
            read_outlines(baseline).unwrap()
        );
        let destinations = read_named_destinations(&reopened).unwrap();
        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].name, "bravo");
        assert_eq!(destinations[0].page_index, 0);
        let labels = read_page_labels(&reopened).unwrap();
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].label, "A-1");
        let attachments = read_embedded_attachments(&reopened).unwrap();
        let original_attachments = read_embedded_attachments(baseline).unwrap();
        assert_eq!(attachments, original_attachments);
        for attachment in attachments {
            assert_eq!(
                read_embedded_attachment_bytes(&reopened, &attachment).unwrap(),
                read_embedded_attachment_bytes(baseline, &attachment).unwrap()
            );
        }
        // Deleting a page unregisters it but does not erase its old content/APs.
        assert_eq!(
            reopened.inspect().unwrap().object_count,
            baseline.inspect().unwrap().object_count
        );
        let streams = list_streams(baseline).unwrap();
        assert_eq!(list_streams(&reopened).unwrap(), streams);
        for stream in streams {
            assert_eq!(
                read_decoded_stream(&reopened, stream.object).unwrap(),
                read_decoded_stream(baseline, stream.object).unwrap()
            );
        }
        let references = PdfReferenceIndex::build(&reopened, &context()).unwrap();
        assert!(references.records().iter().all(|edge| !edge.dangling));
        let closure = reachable(&references);
        for removed in [4, 6, 10, 12, 13, 15, 16, 17] {
            assert!(
                !closure.contains(&reference(removed)),
                "object {removed} remains reachable"
            );
        }
        for owner in [18, 26, 27] {
            assert!(references.records().iter().any(|edge| edge.origin
                == PdfReferenceOrigin::Object(reference(owner))
                && edge.target == reference(3)));
            assert!(!references.records().iter().any(|edge| edge.origin
                == PdfReferenceOrigin::Object(reference(owner))
                && edge.target == reference(4)));
        }
        for widget in [9, 11] {
            assert!(references.records().iter().any(|edge| edge.origin
                == PdfReferenceOrigin::Object(reference(widget))
                && edge.target == reference(3)
                && edge.path == [key(b"P")]));
        }
    }
}

#[test]
fn deletion_reconciles_navigation_labels_and_form_registration_in_both_writers() {
    let source = open(SOURCE);
    let mut revision = PdfRevision::new(source.clone());
    let before = revision.snapshot();
    let ctx = context();
    let plan = revision.plan(delete_bravo(), &ctx).unwrap();
    assert_effect(&plan);
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    let receipt = revision.commit(&plan, &ctx).unwrap();
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), before.source());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(source.inspect().unwrap().page_count, 2);
    assert_outputs(&revision, &source);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        let reopened = open(output.output.as_bytes());
        assert_eq!(reopened.inspect().unwrap().object_count, 34);
        assert_eq!(
            reachable(&PdfReferenceIndex::build(&reopened, &ctx).unwrap()).len(),
            26
        );
    }
}

#[test]
fn explicit_target_policy_invalid_indices_and_mixed_requests_publish_no_deletion() {
    let revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let ctx = context();
    for invalid in [
        request(1, PdfExcludedPageTargetPolicy::Reject),
        request(2, PdfExcludedPageTargetPolicy::Reject),
        request(usize::MAX, PdfExcludedPageTargetPolicy::Reject),
        request(
            1,
            PdfExcludedPageTargetPolicy::RemapToPage { page_index: 1 },
        ),
        request(
            1,
            PdfExcludedPageTargetPolicy::RemapToPage { page_index: 2 },
        ),
        PdfRevisionRequest {
            reorder_pages: Some(vec![1, 0]),
            ..delete_bravo()
        },
        PdfRevisionRequest {
            text_fields: fill("group.name", "Changed").text_fields,
            ..delete_bravo()
        },
    ] {
        assert!(revision.plan(invalid, &ctx).is_err());
        assert_eq!(revision.snapshot(), before);
        assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    }
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        let reopened = open(output.output.as_bytes());
        assert_eq!(
            list_form_fields(&reopened).unwrap(),
            list_form_fields(&open(SOURCE)).unwrap()
        );
        assert_eq!(read_named_destinations(&reopened).unwrap()[0].page_index, 1);
    }
}

#[test]
fn cancelled_and_limited_deletions_retry_without_publishing_partial_state() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let ctx = context();
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision.plan(delete_bravo(), &cancelled).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    for limited in [
        limited_context(0, 512 * 1024 * 1024),
        limited_context(128 * 1024 * 1024, 0),
    ] {
        assert_eq!(
            revision.plan(delete_bravo(), &limited).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), before);
    }
    let plan = revision.plan(delete_bravo(), &ctx).unwrap();
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    let exhausted = limited_context(128 * 1024 * 1024, plan.changed_objects().count() as u64);
    assert!(revision.commit(&plan, &exhausted).is_err());
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    revision.commit(&plan, &ctx).unwrap();
    let committed = revision.snapshot();
    assert!(matches!(
        revision.commit(&plan, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(revision.snapshot(), committed);
    assert_outputs(&revision, &open(SOURCE));
}

#[test]
fn source_identity_and_equal_revision_branches_reject_foreign_deletion_plans() {
    let ctx = context();
    let mut left = PdfRevision::new(open(SOURCE));
    let plan = left.plan(delete_bravo(), &ctx).unwrap();
    let mut changed = SOURCE.to_vec();
    let marker = b"Original retained text-field fixture";
    let offset = changed
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    changed[offset] = b'o';
    assert_eq!(changed.len(), SOURCE.len());
    let mut other = PdfRevision::new(open(&changed));
    let before = other.snapshot();
    assert!(matches!(
        other.commit(&plan, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(other.snapshot(), before);
    let mut right = left.clone();
    for (revision, rotation) in [(&mut left, 90), (&mut right, 180)] {
        let edit = revision
            .plan(
                PdfRevisionRequest {
                    page_transforms: vec![PdfPageTransformRequest {
                        page_index: 0,
                        transform: PageTransform {
                            rotation_degrees: Some(rotation),
                            ..Default::default()
                        },
                    }],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        revision.commit(&edit, &ctx).unwrap();
    }
    assert_eq!(left.snapshot(), right.snapshot());
    let foreign = left.plan(delete_bravo(), &ctx).unwrap();
    let before = right.snapshot();
    assert!(
        matches!(right.commit(&foreign, &ctx), Err(TransactionError::Stage(error)) if error.code == PdfErrorCode::UnsafeRewrite)
    );
    assert_eq!(right.snapshot(), before);
    assert_eq!(right.page_geometry(0).unwrap().rotation_degrees, 180);
    let own = right.plan(delete_bravo(), &ctx).unwrap();
    right.commit(&own, &ctx).unwrap();
    assert_eq!(right.page_ids().unwrap(), [reference(3)]);
    assert_eq!(right.page_geometry(0).unwrap().rotation_degrees, 180);
}

#[test]
fn surviving_multiwidget_field_keeps_committed_value_and_accepts_a_later_fill() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let edited = revision
        .plan(fill("group.name", "Before deletion"), &ctx)
        .unwrap();
    assert_eq!(edited.text_field_effects().next().unwrap().widgets.len(), 2);
    revision.commit(&edited, &ctx).unwrap();
    let baseline = open(
        revision
            .export(PdfWriterMode::Compact, &ctx)
            .unwrap()
            .output
            .as_bytes(),
    );
    let deleted = revision.plan(delete_bravo(), &ctx).unwrap();
    revision.commit(&deleted, &ctx).unwrap();
    assert_outputs(&revision, &baseline);
    let edited = revision
        .plan(fill("group.name", "After deletion"), &ctx)
        .unwrap();
    let effect = edited.text_field_effects().next().unwrap();
    assert_eq!(effect.before_value, "Before deletion");
    assert_eq!(effect.widgets.len(), 1);
    assert_eq!(effect.widgets[0].widget, reference(9));
    assert_eq!(effect.widgets[0].page, reference(3));
    revision.commit(&edited, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 3);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        let reopened = open(output.output.as_bytes());
        let fields = list_form_fields(&reopened).unwrap();
        assert_eq!(fields.len(), 3);
        let original_fields = list_form_fields(&open(SOURCE)).unwrap();
        for field in &fields {
            let original = original_fields
                .iter()
                .find(|original| original.name == field.name)
                .unwrap();
            assert_eq!(field.default_value, original.default_value);
            assert_eq!(
                field.value,
                if field.name == "group.name" {
                    Some("After deletion".into())
                } else {
                    original.value.clone()
                }
            );
        }
        assert_eq!(read_named_destinations(&reopened).unwrap()[0].page_index, 0);
        assert_eq!(read_page_labels(&reopened).unwrap()[0].label, "A-1");
    }
}

#[test]
fn deleted_selections_and_the_last_remaining_page_are_refused() {
    let ctx = context();
    let mut revision = PdfRevision::new(open(SOURCE));
    let stale_field = revision
        .plan(fill("code", "Deleted selection"), &ctx)
        .unwrap();
    let deleted = revision.plan(delete_bravo(), &ctx).unwrap();
    revision.commit(&deleted, &ctx).unwrap();
    let before = revision.snapshot();
    assert!(matches!(
        revision.commit(&stale_field, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert!(revision.plan(fill("code", "Missing"), &ctx).is_err());
    assert!(revision.page_geometry(1).is_err());
    for policy in [
        PdfExcludedPageTargetPolicy::Reject,
        PdfExcludedPageTargetPolicy::RemapToPage { page_index: 0 },
    ] {
        assert!(revision.plan(request(0, policy), &ctx).is_err());
        assert_eq!(revision.snapshot(), before);
    }
    assert_outputs(&revision, &open(SOURCE));
}

#[test]
fn deleting_a_duplicate_unregisters_only_its_fields_then_allows_another_deletion() {
    let ctx = context();
    let mut revision = PdfRevision::new(open(SOURCE));
    let duplication = revision
        .plan(
            PdfRevisionRequest {
                duplicate_page: Some(PdfPageDuplicateRequest {
                    page_index: 0,
                    fields: ["name", "keep"]
                        .into_iter()
                        .map(|name| PdfDuplicateFieldName {
                            field_name: format!("group.{name}"),
                            new_field_name: format!("group.{name}_copy"),
                        })
                        .collect(),
                }),
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    let copied = duplication.duplicate_effect().unwrap().page;
    revision.commit(&duplication, &ctx).unwrap();
    assert_eq!(
        revision.page_ids().unwrap(),
        [reference(3), copied, reference(4)]
    );
    let deletion = revision
        .plan(request(1, PdfExcludedPageTargetPolicy::Reject), &ctx)
        .unwrap();
    let effect = deletion.delete_effect().unwrap();
    assert_eq!(effect.page, copied);
    assert_eq!(effect.remaining_pages, [reference(3), reference(4)]);
    assert!(effect.remapped_destinations.is_empty());
    assert_eq!(effect.unregistered_fields.len(), 2);
    revision.commit(&deletion, &ctx).unwrap();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        let reopened = open(output.output.as_bytes());
        assert_eq!(
            list_form_fields(&reopened).unwrap(),
            list_form_fields(&open(SOURCE)).unwrap()
        );
        assert_eq!(
            list_annotations(&reopened).unwrap(),
            list_annotations(&open(SOURCE)).unwrap()
        );
        assert_eq!(read_named_destinations(&reopened).unwrap()[0].page_index, 1);
        assert_eq!(
            read_page_labels(&reopened)
                .unwrap()
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["A-1", "B-1"]
        );
    }
    let deletion = revision.plan(delete_bravo(), &ctx).unwrap();
    revision.commit(&deletion, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 3);
    assert_eq!(revision.page_ids().unwrap(), [reference(3)]);
    assert!(
        revision
            .plan(fill("group.name_copy", "Gone"), &ctx)
            .is_err()
    );
    let survivor = revision
        .plan(fill("group.name", "Original survived"), &ctx)
        .unwrap();
    revision.commit(&survivor, &ctx).unwrap();
}

#[test]
fn export_limits_leave_the_committed_deletion_available_for_later_edits() {
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_output_bytes: 1,
            ..Default::default()
        },
    })
    .open(SOURCE, OpenOptions::default())
    .unwrap();
    let mut revision = PdfRevision::new(document);
    let ctx = context();
    let plan = revision.plan(delete_bravo(), &ctx).unwrap();
    revision.commit(&plan, &ctx).unwrap();
    let before = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        assert_eq!(
            revision.export(mode, &ctx).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), before);
    }
    let edit = revision
        .plan(fill("group.name", "Still editable"), &ctx)
        .unwrap();
    assert_eq!(edit.text_field_effects().next().unwrap().widgets.len(), 1);
    revision.commit(&edit, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 2);
    assert_eq!(revision.page_ids().unwrap(), [reference(3)]);
}
