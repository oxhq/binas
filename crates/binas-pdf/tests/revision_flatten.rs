//! Public selected-field flatten workflows on the frozen original QA fixture.
//! Exact source-object, paint-matrix and rendered-output proof lives in the oracle.
use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget,
    transaction::TransactionError,
};
use binas_pdf::{
    CheckboxFieldMutationRequest, EngineConfig, Limits, OpenOptions, PageTransform, PdfDocument,
    PdfEngine, PdfErrorCode, PdfFieldFlattenRequest, PdfPageTextEditRequest,
    PdfPageTransformRequest, PdfReferenceIndex, PdfReferenceTarget, PdfRevision, PdfRevisionPlan,
    PdfRevisionRequest, PdfSharedEditMode, PdfWriterMode, SurgicalTextEditRequest,
    list_annotations, list_form_fields, read_document_info, read_embedded_attachment_bytes,
    read_embedded_attachments, read_named_destinations, read_outlines, read_page_labels,
    read_xmp_metadata,
};

const SOURCE: &[u8] = include_bytes!("../../../qa/preservation/fixtures/targeted-flatten.pdf");

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

fn field(name: &str) -> PdfFieldFlattenRequest {
    PdfFieldFlattenRequest {
        field_name: name.into(),
        match_index: 0,
    }
}

fn flatten(name: &str) -> PdfRevisionRequest {
    PdfRevisionRequest {
        flatten_fields: vec![field(name)],
        ..Default::default()
    }
}

fn checkbox(checked: bool) -> PdfRevisionRequest {
    PdfRevisionRequest {
        checkboxes: vec![CheckboxFieldMutationRequest {
            field_name: "accept".into(),
            checked,
            match_index: 0,
        }],
        ..Default::default()
    }
}

fn title_edit() -> PdfPageTextEditRequest {
    PdfPageTextEditRequest {
        page_index: 0,
        shared: PdfSharedEditMode::Reject,
        edit: SurgicalTextEditRequest {
            old_text: "PAGE ALPHA".into(),
            replacement: "PAGE GAMMA".into(),
            match_index: 0,
        },
    }
}

fn assert_answer_effects(plan: &PdfRevisionPlan) {
    let effects = plan.flatten_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].field, reference(8));
    let owners = effects[0]
        .widgets
        .iter()
        .map(|effect| (effect.widget, effect.page, effect.appearance))
        .collect::<Vec<_>>();
    assert_eq!(
        owners,
        vec![
            (reference(9), reference(3), reference(14)),
            (reference(10), reference(4), reference(14)),
        ]
    );
    for effect in &effects[0].widgets {
        assert!(!effect.resource_name.is_empty());
        assert_ne!(effect.resource_name, b"BinasFlatten0");
        assert!(
            effect
                .placement_matrix
                .iter()
                .all(|value| value.is_finite())
        );
    }
}

fn assert_active_outputs(
    revision: &PdfRevision,
    removed_fields: &[&str],
    accept: bool,
    order: [u32; 2],
) {
    let original = open(SOURCE);
    let original_fields = list_form_fields(&original).unwrap();
    let removed_widgets = original_fields
        .iter()
        .filter(|field| removed_fields.contains(&field.name.as_str()))
        .flat_map(|field| field.widget_refs.iter().map(|widget| widget.object_number))
        .collect::<Vec<_>>();
    let prune_group =
        removed_fields.contains(&"group.answer") && removed_fields.contains(&"group.keep");
    let mut expected_fields = original_fields
        .into_iter()
        .filter(|field| {
            !(removed_fields.contains(&field.name.as_str()) || prune_group && field.name == "group")
        })
        .collect::<Vec<_>>();
    for (index, field) in expected_fields.iter_mut().enumerate() {
        field.index = index;
        field
            .widget_refs
            .retain(|widget| !removed_widgets.contains(&widget.object_number));
        if field.name == "accept" {
            field.value = Some(if accept { "Yes" } else { "Off" }.into());
        }
    }
    let mut expected_annotations = list_annotations(&original).unwrap();
    expected_annotations.retain(|annotation| {
        !annotation
            .object_number
            .is_some_and(|number| removed_widgets.contains(&number))
    });
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
    assert_eq!(revision.annotations().unwrap(), expected_annotations);
    let source_attachments = read_embedded_attachments(&original).unwrap();
    assert_eq!(source_attachments.len(), 1);
    let attachment = read_embedded_attachment_bytes(&original, &source_attachments[0]).unwrap();
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
        let attachments = read_embedded_attachments(&reopened).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "note.txt");
        assert_eq!(
            read_embedded_attachment_bytes(&reopened, &attachments[0]).unwrap(),
            attachment
        );
        let destinations = read_named_destinations(&reopened).unwrap();
        assert_eq!(destinations.len(), 1);
        assert_eq!(destinations[0].name, "bravo");
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
        }
        assert!(
            PdfReferenceIndex::build(&reopened, &context())
                .unwrap()
                .records()
                .iter()
                .all(|edge| !edge.dangling)
        );
        let reopened_revision = PdfRevision::new(reopened);
        assert_eq!(reopened_revision.page_ids().unwrap(), order.map(reference));
        let checkbox_plan = reopened_revision
            .plan(checkbox(!accept), &context())
            .unwrap();
        let states = checkbox_plan.checkbox_effects().collect::<Vec<_>>();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].field, reference(12));
        assert_eq!(
            states[0].before_value.as_deref(),
            Some(if accept {
                b"Yes".as_slice()
            } else {
                b"Off".as_slice()
            })
        );
    }
}

#[test]
fn selected_multiwidget_field_flattens_once_and_leaves_other_fields_editable() {
    let original = open(SOURCE);
    let source_fields = list_form_fields(&original).unwrap();
    let mut revision = PdfRevision::new(original.clone());
    let before = revision.snapshot();
    let ctx = context();
    let plan = revision.plan(flatten("group.answer"), &ctx).unwrap();
    assert_answer_effects(&plan);
    assert_eq!(revision.snapshot(), before);
    let receipt = revision.commit(&plan, &ctx).unwrap();
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(receipt.after.source(), before.source());
    assert_eq!(list_form_fields(&original).unwrap(), source_fields);
    assert_active_outputs(&revision, &["group.answer"], true, [3, 4]);
}

#[test]
fn flatten_reorder_and_geometry_bind_both_widgets_to_their_original_pages() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![1, 0]),
                page_transforms: vec![
                    PdfPageTransformRequest {
                        page_index: 0,
                        transform: PageTransform {
                            rotation_degrees: Some(-90),
                            media_box: Some([0., 0., 340., 300.]),
                            crop_box: Some([10., 10., 290., 250.]),
                            ..Default::default()
                        },
                    },
                    PdfPageTransformRequest {
                        page_index: 1,
                        transform: PageTransform {
                            rotation_degrees: Some(180),
                            ..Default::default()
                        },
                    },
                ],
                ..flatten("group.answer")
            },
            &ctx,
        )
        .unwrap();
    assert_answer_effects(&plan);
    let geometries = plan
        .geometry_effects()
        .map(|effect| (effect.page, effect.after))
        .collect::<Vec<_>>();
    assert_eq!(geometries.len(), 2);
    revision.commit(&plan, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 1);
    assert_eq!(revision.page_ids().unwrap(), [reference(4), reference(3)]);
    for (page, expected) in geometries {
        let position = if page == reference(3) { 1 } else { 0 };
        assert_eq!(revision.page_geometry(position).unwrap(), expected);
    }
    assert_eq!(revision.page_geometry(1).unwrap().rotation_degrees, 270);
    assert_active_outputs(&revision, &["group.answer"], true, [4, 3]);
}

#[test]
fn later_flatten_prunes_empty_parent_and_keeps_surviving_checkbox_usable() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let first = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![1, 0]),
                ..flatten("group.answer")
            },
            &ctx,
        )
        .unwrap();
    revision.commit(&first, &ctx).unwrap();
    let second = revision.plan(flatten("group.keep"), &ctx).unwrap();
    let effects = second.flatten_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].field, reference(11));
    assert_eq!(effects[0].widgets.len(), 1);
    assert_eq!(effects[0].widgets[0].page, reference(3));
    revision.commit(&second, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 2);
    assert_active_outputs(&revision, &["group.answer", "group.keep"], true, [4, 3]);
    let toggle = revision.plan(checkbox(false), &ctx).unwrap();
    revision.commit(&toggle, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 3);
    assert_active_outputs(&revision, &["group.answer", "group.keep"], false, [4, 3]);
    let committed = revision.snapshot();
    for name in ["group.answer", "group.keep", "group"] {
        assert!(revision.plan(flatten(name), &ctx).is_err());
        assert_eq!(revision.snapshot(), committed);
    }
}

#[test]
fn invalid_or_duplicate_field_selection_cannot_partially_flatten_a_batch() {
    let revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let ctx = context();
    let requests = [
        vec![field("group.answer"), field("missing")],
        vec![field("group.answer"), field("group.answer")],
        vec![field("group.answer"), field("group")],
        vec![field("group")],
        vec![field("")],
        vec![PdfFieldFlattenRequest {
            field_name: "group.answer".into(),
            match_index: 1,
        }],
    ];
    for fields in requests {
        assert!(
            revision
                .plan(
                    PdfRevisionRequest {
                        flatten_fields: fields,
                        reorder_pages: Some(vec![1, 0]),
                        ..Default::default()
                    },
                    &ctx
                )
                .is_err()
        );
        assert_eq!(revision.snapshot(), before);
        assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    }
    assert_active_outputs(&revision, &[], true, [3, 4]);
}

#[test]
fn conflicting_checkbox_or_same_page_text_batch_refuses_but_separate_commits_work() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let ctx = context();
    for request in [
        PdfRevisionRequest {
            flatten_fields: vec![field("group.answer")],
            ..checkbox(false)
        },
        PdfRevisionRequest {
            page_text: vec![title_edit()],
            ..flatten("group.answer")
        },
    ] {
        assert_eq!(
            revision.plan(request, &ctx).unwrap_err().code,
            PdfErrorCode::UnsupportedFeature
        );
        assert_eq!(revision.snapshot(), before);
    }
    assert_active_outputs(&revision, &[], true, [3, 4]);
    let edit = revision
        .plan(
            PdfRevisionRequest {
                page_text: vec![title_edit()],
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    revision.commit(&edit, &ctx).unwrap();
    let flattened = revision.plan(flatten("group.answer"), &ctx).unwrap();
    revision.commit(&flattened, &ctx).unwrap();
    let toggle = revision.plan(checkbox(false), &ctx).unwrap();
    revision.commit(&toggle, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 3);
    assert_active_outputs(&revision, &["group.answer"], false, [3, 4]);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = revision.export(mode, &ctx).unwrap();
        let text = PdfRevision::new(open(exported.output.as_bytes()))
            .text()
            .unwrap();
        assert!(text.iter().any(|value| value == "PAGE GAMMA"));
        assert!(!text.iter().any(|value| value == "PAGE ALPHA"));
    }
}

#[test]
fn cancellation_and_quota_failures_publish_nothing_and_leave_plan_retryable() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let generous = context();
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision
            .plan(flatten("group.answer"), &cancelled)
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
                .plan(flatten("group.answer"), &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }
    let plan = revision.plan(flatten("group.answer"), &generous).unwrap();
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    let exhausted = limited_context(128 * 1024 * 1024, plan.changed_objects().count() as u64);
    assert!(revision.commit(&plan, &exhausted).is_err());
    assert_eq!(revision.snapshot(), before);
    assert_active_outputs(&revision, &[], true, [3, 4]);
    revision.commit(&plan, &generous).unwrap();
    let committed = revision.snapshot();
    assert!(matches!(
        revision.commit(&plan, &generous),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(revision.snapshot(), committed);
    assert_active_outputs(&revision, &["group.answer"], true, [3, 4]);
}

#[test]
fn different_source_or_equal_numbered_divergent_revision_rejects_foreign_flatten_plan() {
    let mut left = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let initial = left.plan(flatten("group.answer"), &ctx).unwrap();
    let mut bytes = SOURCE.to_vec();
    let marker = b"Original targeted flatten fixture";
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    bytes[offset] = b'o';
    let mut different_source = PdfRevision::new(open(&bytes));
    let before = different_source.snapshot();
    assert!(matches!(
        different_source.commit(&initial, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(different_source.snapshot(), before);

    let mut right = left.clone();
    for (revision, rotation) in [(&mut left, 90), (&mut right, 180)] {
        let plan = revision
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
        revision.commit(&plan, &ctx).unwrap();
    }
    assert_eq!(left.snapshot(), right.snapshot());
    let foreign = left.plan(flatten("group.answer"), &ctx).unwrap();
    let before = right.snapshot();
    assert!(
        matches!(right.commit(&foreign, &ctx), Err(TransactionError::Stage(error)) if error.code == PdfErrorCode::UnsafeRewrite)
    );
    assert_eq!(right.snapshot(), before);
    assert_eq!(right.page_geometry(0).unwrap().rotation_degrees, 180);
    let own = right.plan(flatten("group.answer"), &ctx).unwrap();
    right.commit(&own, &ctx).unwrap();
    assert_active_outputs(&right, &["group.answer"], true, [3, 4]);
    assert_active_outputs(&left, &[], true, [3, 4]);
}

#[test]
fn failed_output_does_not_restore_an_already_flattened_field() {
    let config = EngineConfig {
        limits: Limits {
            max_output_bytes: 1,
            ..Default::default()
        },
    };
    let source = PdfEngine::new(config)
        .open(SOURCE, OpenOptions::default())
        .unwrap();
    let mut revision = PdfRevision::new(source);
    let ctx = context();
    let plan = revision.plan(flatten("group.answer"), &ctx).unwrap();
    revision.commit(&plan, &ctx).unwrap();
    let committed = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        assert_eq!(
            revision.export(mode, &ctx).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), committed);
    }
    assert!(revision.plan(flatten("group.answer"), &ctx).is_err());
    let remaining = revision.plan(flatten("group.keep"), &ctx).unwrap();
    assert_eq!(
        remaining.flatten_effects().next().unwrap().field,
        reference(11)
    );
    let toggle = revision.plan(checkbox(false), &ctx).unwrap();
    revision.commit(&toggle, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 2);
    assert!(
        !revision
            .annotations()
            .unwrap()
            .iter()
            .any(|annotation| { matches!(annotation.object_number, Some(9 | 10)) })
    );
}
