//! Public retained-checkbox workflow and rollback regressions on an original
//! frozen fixture. Independent encoded-appearance/RGB proof belongs to its oracle.
use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget,
    transaction::TransactionError,
};
use binas_pdf::{
    CheckboxFieldMutationRequest, EngineConfig, Limits, OpenOptions, PageTransform, PdfDocument,
    PdfEngine, PdfErrorCode, PdfPageTransformRequest, PdfReferenceIndex, PdfReferenceOrigin,
    PdfReferencePathStep, PdfReferenceTarget, PdfRevision, PdfRevisionRequest, PdfWriterMode,
    list_annotations, list_form_fields, read_document_info, read_embedded_attachment_bytes,
    read_embedded_attachments, read_named_destinations, read_outlines, read_page_labels,
    read_xmp_metadata,
};

const SOURCE: &[u8] = include_bytes!("../../../qa/preservation/fixtures/checkbox-states.pdf");

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
fn checkbox(name: &str, checked: bool) -> CheckboxFieldMutationRequest {
    CheckboxFieldMutationRequest {
        field_name: name.into(),
        checked,
        match_index: 0,
    }
}
fn request(accept: bool, subscribe: bool) -> PdfRevisionRequest {
    PdfRevisionRequest {
        checkboxes: vec![
            checkbox("accept", accept),
            checkbox("preferences.subscribe", subscribe),
        ],
        ..Default::default()
    }
}
fn state(checked: bool) -> &'static [u8] {
    if checked { b"Yes" } else { b"Off" }
}

fn assert_effects(plan: &binas_pdf::PdfRevisionPlan, before: [bool; 2], after: [bool; 2]) {
    let effects: Vec<_> = plan.checkbox_effects().collect();
    assert_eq!(effects.len(), 2);
    for (index, field, widget, page) in [(0, 5, 5, 3), (1, 7, 8, 4)] {
        let effect = effects
            .iter()
            .find(|effect| effect.field == reference(field))
            .unwrap();
        assert_eq!(effect.widget, reference(widget));
        assert_eq!(effect.page, reference(page));
        assert_eq!(effect.checked, after[index]);
        assert_eq!(effect.before_value.as_deref(), Some(state(before[index])));
        assert_eq!(effect.before_appearance_state, state(before[index]));
        assert_eq!(effect.after_state, state(after[index]));
    }
    assert!(
        plan.changed_objects()
            .all(|object| ![11, 12].contains(&object.number)),
        "existing shared appearance programs must not be rewritten"
    );
}

fn assert_preserved_outputs(revision: &PdfRevision, checked: [bool; 2], order: [u32; 2]) {
    let original = open(SOURCE);
    let original_references = PdfReferenceIndex::build(&original, &context()).unwrap();
    let mut expected_fields = list_form_fields(&original).unwrap();
    // The read API includes the nonterminal preferences group as well.
    assert_eq!(expected_fields.len(), 3);
    for field in &mut expected_fields {
        let selected = match field.name.as_str() {
            "accept" => checked[0],
            "preferences.subscribe" => checked[1],
            "preferences" => continue,
            other => panic!("unexpected field {other}"),
        };
        field.value = Some(String::from_utf8(state(selected).to_vec()).unwrap());
    }
    let mut expected_annotations = list_annotations(&original).unwrap();
    expected_annotations.sort_by_key(|annotation| {
        (
            order
                .iter()
                .position(|page| *page == annotation.page_object_number)
                .unwrap(),
            annotation.index,
        )
    });
    for (index, annotation) in expected_annotations.iter_mut().enumerate() {
        annotation.index = index;
        annotation.page_index = order
            .iter()
            .position(|page| *page == annotation.page_object_number)
            .unwrap();
    }
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = revision.export(mode, &context()).unwrap();
        assert_eq!(exported.snapshot, revision.snapshot());
        assert_eq!(exported.mode, mode);
        if mode == PdfWriterMode::Incremental {
            assert!(exported.output.as_bytes().starts_with(SOURCE));
        }
        let reopened = open(exported.output.as_bytes());
        assert_eq!(list_form_fields(&reopened).unwrap(), expected_fields);
        assert_eq!(list_annotations(&reopened).unwrap(), expected_annotations);
        assert_eq!(
            read_document_info(&reopened).unwrap(),
            read_document_info(&original).unwrap()
        );
        assert_eq!(
            read_xmp_metadata(&reopened).unwrap(),
            read_xmp_metadata(&original).unwrap()
        );
        assert_eq!(
            read_outlines(&reopened).unwrap(),
            read_outlines(&original).unwrap()
        );
        let destinations = read_named_destinations(&reopened).unwrap();
        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].name, "subscribe");
        assert_eq!(
            destinations[0].page_index,
            order.iter().position(|page| *page == 4).unwrap()
        );
        let labels = read_page_labels(&reopened).unwrap();
        for (position, page) in order.iter().enumerate() {
            assert_eq!(
                labels[position].label,
                if *page == 3 { "A-1" } else { "B-1" }
            );
            assert_eq!(
                reopened.page_geometry(position).unwrap(),
                revision.page_geometry(position).unwrap()
            );
            assert_eq!(
                reopened
                    .extract_page_text_spans(position)
                    .unwrap()
                    .spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<Vec<_>>(),
                original
                    .extract_page_text_spans(if *page == 3 { 0 } else { 1 })
                    .unwrap()
                    .spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<Vec<_>>(),
            );
        }
        let attachments = read_embedded_attachments(&reopened).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "note.txt");
        assert_eq!(
            read_embedded_attachment_bytes(&reopened, &attachments[0]).unwrap(),
            b"Original checkbox attachment; preserve NUL:\0 and newline.\n"
        );
        let references = PdfReferenceIndex::build(&reopened, &context()).unwrap();
        assert_eq!(
            references.records().len(),
            original_references.records().len()
        );
        assert!(references.records().iter().all(|edge| !edge.dangling));
        for edge in original_references.records() {
            if edge.origin == PdfReferenceOrigin::Object(reference(2))
                && edge.path.first() == Some(&PdfReferencePathStep::Key(b"Kids".to_vec()))
            {
                continue;
            }
            // Includes AcroForm registration, nested Kids/Parent, page Annots/P,
            // shared Off/Yes AP references, link, metadata and attachment edges.
            assert!(
                references.records().contains(edge),
                "unrequested reference change: {edge:?}"
            );
        }
        let reopened_revision = PdfRevision::new(reopened);
        assert_eq!(reopened_revision.page_ids().unwrap(), order.map(reference));
        // Plan the opposite state without committing: before-state evidence
        // reads both current /V and widget /AS through the supported public API.
        let inspect_states = reopened_revision
            .plan(request(!checked[0], !checked[1]), &context())
            .unwrap();
        assert_effects(&inspect_states, checked, [!checked[0], !checked[1]]);
    }
}

#[test]
fn combined_and_nested_checkbox_states_commit_once_and_preserve_both_writer_outputs() {
    let original = open(SOURCE);
    let source_fields = list_form_fields(&original).unwrap();
    let mut revision = PdfRevision::new(original.clone());
    let snapshot = revision.snapshot();
    let context = context();
    let plan = revision.plan(request(true, false), &context).unwrap();
    assert_effects(&plan, [false, true], [true, false]);
    assert_eq!(revision.snapshot(), snapshot);
    assert_eq!(list_form_fields(&original).unwrap(), source_fields);
    let receipt = revision.commit(&plan, &context).unwrap();
    assert_eq!(receipt.before, snapshot);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(receipt.after.source(), snapshot.source());
    assert_eq!(list_form_fields(&original).unwrap(), source_fields);
    assert_preserved_outputs(&revision, [true, false], [3, 4]);
}

#[test]
fn checkbox_geometry_and_reorder_resolve_source_pages_in_one_commit() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let snapshot = revision.snapshot();
    let context = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                page_transforms: vec![PdfPageTransformRequest {
                    page_index: 0,
                    transform: PageTransform {
                        rotation_degrees: Some(-90),
                        media_box: Some([0., 0., 360., 280.]),
                        crop_box: Some([10., 10., 310., 230.]),
                        ..Default::default()
                    },
                }],
                reorder_pages: Some(vec![1, 0]),
                ..request(true, false)
            },
            &context,
        )
        .unwrap();
    assert_effects(&plan, [false, true], [true, false]);
    let geometry = plan.geometry_effects().collect::<Vec<_>>();
    assert_eq!(geometry.len(), 1);
    assert_eq!(geometry[0].page, reference(3));
    assert_eq!(geometry[0].before.media_box, [0., 0., 320., 240.]);
    assert_eq!(geometry[0].after.rotation_degrees, 270);
    let expected_geometry = geometry[0].after;
    let receipt = revision.commit(&plan, &context).unwrap();
    assert_eq!(receipt.before, snapshot);
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(revision.page_ids().unwrap(), [reference(4), reference(3)]);
    assert_eq!(revision.page_geometry(1).unwrap(), expected_geometry);
    assert_eq!(revision.page_geometry(0).unwrap().rotation_degrees, 0);
    assert_eq!(
        revision.page_geometry(0).unwrap().media_box,
        [0., 0., 320., 240.]
    );
    assert_preserved_outputs(&revision, [true, false], [4, 3]);
}

#[test]
fn repeated_toggles_use_the_current_field_values_and_keep_prior_reorder() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let context = context();
    let first = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![1, 0]),
                ..request(true, false)
            },
            &context,
        )
        .unwrap();
    revision.commit(&first, &context).unwrap();
    let mut before = [true, false];
    for (number, after) in [[false, true], [true, false], [false, true]]
        .into_iter()
        .enumerate()
    {
        let snapshot = revision.snapshot();
        let plan = revision
            .plan(request(after[0], after[1]), &context)
            .unwrap();
        assert_effects(&plan, before, after);
        assert_eq!(revision.snapshot(), snapshot);
        revision.commit(&plan, &context).unwrap();
        assert_eq!(revision.snapshot().revision().value(), number as u64 + 2);
        assert_eq!(revision.page_ids().unwrap(), [reference(4), reference(3)]);
        before = after;
    }
    // The current API treats a same-state request as an ordinary checked commit.
    let same = revision
        .plan(request(before[0], before[1]), &context)
        .unwrap();
    assert_effects(&same, before, before);
    revision.commit(&same, &context).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 5);
    assert_preserved_outputs(&revision, before, [4, 3]);
}

#[test]
fn invalid_selection_and_a_failed_mixed_request_leave_every_checkbox_unchanged() {
    let revision = PdfRevision::new(open(SOURCE));
    let snapshot = revision.snapshot();
    let context = context();
    let mut missing_occurrence = checkbox("accept", true);
    missing_occurrence.match_index = 1;
    let cases = [
        PdfRevisionRequest {
            checkboxes: vec![checkbox("accept", true), checkbox("missing", false)],
            reorder_pages: Some(vec![1, 0]),
            ..Default::default()
        },
        PdfRevisionRequest {
            checkboxes: vec![checkbox("accept", true), checkbox("accept", false)],
            ..Default::default()
        },
        PdfRevisionRequest {
            checkboxes: vec![checkbox("accept", true), checkbox("accept", true)],
            ..Default::default()
        },
        PdfRevisionRequest {
            checkboxes: vec![missing_occurrence],
            ..Default::default()
        },
        PdfRevisionRequest {
            checkboxes: vec![checkbox("", true)],
            ..Default::default()
        },
        PdfRevisionRequest {
            page_transforms: vec![PdfPageTransformRequest {
                page_index: 2,
                transform: PageTransform {
                    rotation_degrees: Some(90),
                    ..Default::default()
                },
            }],
            ..request(true, false)
        },
    ];
    for request in cases {
        assert!(revision.plan(request, &context).is_err());
        assert_eq!(revision.snapshot(), snapshot);
        assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    }
    let unsupported = revision
        .plan(
            PdfRevisionRequest {
                page_transforms: vec![PdfPageTransformRequest {
                    page_index: 0,
                    transform: PageTransform {
                        scale: Some([2., 2.]),
                        ..Default::default()
                    },
                }],
                ..request(true, false)
            },
            &context,
        )
        .unwrap_err();
    assert_eq!(unsupported.code, PdfErrorCode::UnsupportedFeature);
    assert_eq!(revision.snapshot(), snapshot);
    assert_preserved_outputs(&revision, [false, true], [3, 4]);
}

#[test]
fn cancellation_quota_and_stale_plan_failures_leave_the_last_commit_usable() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let original = revision.snapshot();
    let generous = context();
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision
            .plan(request(true, false), &cancelled)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    for limited in [
        limited_context(0, 1_000_000),
        limited_context(128 * 1024 * 1024, 0),
    ] {
        assert_eq!(
            revision
                .plan(request(true, false), &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }
    let plan = revision.plan(request(true, false), &generous).unwrap();
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    // Permit all sparse replacements, then run out of cumulative check work.
    let exhausted = limited_context(128 * 1024 * 1024, plan.changed_objects().count() as u64);
    assert!(revision.commit(&plan, &exhausted).is_err());
    assert_eq!(revision.snapshot(), original);
    assert_preserved_outputs(&revision, [false, true], [3, 4]);
    revision.commit(&plan, &generous).unwrap();
    let committed = revision.snapshot();
    assert!(matches!(
        revision.commit(&plan, &generous),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(revision.snapshot(), committed);
    assert_preserved_outputs(&revision, [true, false], [3, 4]);
}

#[test]
fn equal_revision_numbers_on_divergent_snapshots_do_not_authorize_a_foreign_plan() {
    let mut left = PdfRevision::new(open(SOURCE));
    let mut right = left.clone();
    let context = context();
    let left_plan = left
        .plan(
            PdfRevisionRequest {
                checkboxes: vec![checkbox("accept", true)],
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    let right_plan = right
        .plan(
            PdfRevisionRequest {
                checkboxes: vec![checkbox("preferences.subscribe", false)],
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    left.commit(&left_plan, &context).unwrap();
    right.commit(&right_plan, &context).unwrap();
    assert_eq!(left.snapshot(), right.snapshot());
    let follow_up = left
        .plan(
            PdfRevisionRequest {
                checkboxes: vec![checkbox("accept", false)],
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    let snapshot = right.snapshot();
    assert!(
        matches!(right.commit(&follow_up, &context), Err(TransactionError::Stage(error)) if error.code == PdfErrorCode::UnsafeRewrite)
    );
    assert_eq!(right.snapshot(), snapshot);
    assert_preserved_outputs(&right, [false, false], [3, 4]);
    assert_preserved_outputs(&left, [true, true], [3, 4]);
}

#[test]
fn output_limit_failure_cannot_discard_a_successful_checkbox_commit() {
    let config = EngineConfig {
        limits: Limits {
            max_output_bytes: 1,
            ..Default::default()
        },
    };
    let document = PdfEngine::new(config)
        .open(SOURCE, OpenOptions::default())
        .unwrap();
    let mut revision = PdfRevision::new(document);
    let context = context();
    let plan = revision.plan(request(true, false), &context).unwrap();
    revision.commit(&plan, &context).unwrap();
    let committed = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        assert_eq!(
            revision.export(mode, &context).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), committed);
    }
    let usable = revision.plan(request(false, true), &context).unwrap();
    assert_effects(&usable, [true, false], [false, true]);
    assert_eq!(revision.snapshot(), committed);
}
