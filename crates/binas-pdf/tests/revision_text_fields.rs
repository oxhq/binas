//! Public retained text-field workflows on the original frozen form fixture.
//! Exact appearance streams, shared-AP preservation and rendering use the oracle.
use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget,
    transaction::TransactionError,
};
use binas_pdf::{
    CheckboxFieldMutationRequest, EngineConfig, Limits, OpenOptions, PageTransform, PdfDocument,
    PdfEngine, PdfErrorCode, PdfFieldFlattenRequest, PdfPageTransformRequest, PdfReferenceIndex,
    PdfReferenceTarget, PdfRevision, PdfRevisionPlan, PdfRevisionRequest, PdfTextFieldFillRequest,
    PdfWriterMode, list_annotations, list_form_fields, read_document_info,
    read_embedded_attachment_bytes, read_embedded_attachments, read_named_destinations,
    read_outlines, read_page_labels, read_xmp_metadata,
};

const SOURCE: &[u8] = include_bytes!("../../../qa/preservation/fixtures/text-field-fill.pdf");
const NAME: &str = "Café (A) \\ B";
const CODE: &str = "Ready (2)";

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
fn fill(field_name: &str, value: &str) -> PdfTextFieldFillRequest {
    PdfTextFieldFillRequest {
        field_name: field_name.into(),
        value: value.into(),
        match_index: 0,
    }
}
fn request(name: &str, code: &str) -> PdfRevisionRequest {
    PdfRevisionRequest {
        text_fields: vec![fill("group.name", name), fill("code", code)],
        ..Default::default()
    }
}
fn flatten_name() -> PdfFieldFlattenRequest {
    PdfFieldFlattenRequest {
        field_name: "group.name".into(),
        match_index: 0,
    }
}

fn assert_initial_effects(plan: &PdfRevisionPlan) {
    let original = list_form_fields(&open(SOURCE)).unwrap();
    let effects = plan.text_field_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    for (field, name, value, owners) in [
        (8, "group.name", NAME, vec![(9, 3, 14), (10, 4, 14)]),
        (12, "code", CODE, vec![(12, 4, 15)]),
    ] {
        let effect = effects
            .iter()
            .find(|effect| effect.field == reference(field))
            .unwrap();
        assert_eq!(
            effect.before_value,
            original
                .iter()
                .find(|field| field.name == name)
                .unwrap()
                .value
                .as_deref()
                .unwrap()
        );
        assert_eq!(effect.after_value, value);
        assert_eq!(effect.widgets.len(), owners.len());
        for (widget, page, appearance) in owners {
            let widget_effect = effect
                .widgets
                .iter()
                .find(|effect| effect.widget == reference(widget))
                .unwrap();
            assert_eq!(widget_effect.page, reference(page));
            assert_eq!(widget_effect.source_appearance, reference(appearance));
            assert_ne!(
                widget_effect.output_appearance,
                widget_effect.source_appearance
            );
        }
    }
}

fn assert_outputs(
    revision: &PdfRevision,
    values: &[(&str, &str)],
    removed: &[&str],
    order: [u32; 2],
) {
    let original = open(SOURCE);
    let original_fields = list_form_fields(&original).unwrap();
    let removed_widgets = original_fields
        .iter()
        .filter(|field| removed.contains(&field.name.as_str()))
        .flat_map(|field| field.widget_refs.iter().map(|widget| widget.object_number))
        .collect::<Vec<_>>();
    let mut expected_fields = original_fields
        .into_iter()
        .filter(|field| !removed.contains(&field.name.as_str()))
        .collect::<Vec<_>>();
    for (index, field) in expected_fields.iter_mut().enumerate() {
        field.index = index;
        field
            .widget_refs
            .retain(|widget| !removed_widgets.contains(&widget.object_number));
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == field.name) {
            field.value = Some((*value).into());
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
        let source_attachments = read_embedded_attachments(&original).unwrap();
        let attachments = read_embedded_attachments(&reopened).unwrap();
        assert_eq!(attachments, source_attachments);
        for (before, after) in source_attachments.iter().zip(&attachments) {
            assert_eq!(
                read_embedded_attachment_bytes(&reopened, after).unwrap(),
                read_embedded_attachment_bytes(&original, before).unwrap()
            );
        }
        let destinations = read_named_destinations(&reopened).unwrap();
        let mut expected_destinations = read_named_destinations(&original).unwrap();
        for destination in &mut expected_destinations {
            let source_page = [3, 4][destination.page_index];
            destination.page_index = order.iter().position(|page| *page == source_page).unwrap();
        }
        assert_eq!(destinations, expected_destinations);
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
            if removed.is_empty() {
                let source_page = if *page == 3 { 0 } else { 1 };
                assert_eq!(
                    reopened
                        .extract_page_text_spans(position)
                        .unwrap()
                        .spans
                        .iter()
                        .map(|span| &span.text)
                        .collect::<Vec<_>>(),
                    original
                        .extract_page_text_spans(source_page)
                        .unwrap()
                        .spans
                        .iter()
                        .map(|span| &span.text)
                        .collect::<Vec<_>>()
                );
            }
        }
        assert!(
            PdfReferenceIndex::build(&reopened, &context())
                .unwrap()
                .records()
                .iter()
                .all(|edge| !edge.dangling)
        );
        assert_eq!(
            PdfRevision::new(reopened).page_ids().unwrap(),
            order.map(reference)
        );
    }
}

#[test]
fn multiwidget_and_combined_fill_reorder_and_geometry_commit_atomically() {
    let original = open(SOURCE);
    let fields = list_form_fields(&original).unwrap();
    let mut revision = PdfRevision::new(original.clone());
    let before = revision.snapshot();
    let ctx = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![1, 0]),
                page_transforms: vec![PdfPageTransformRequest {
                    page_index: 0,
                    transform: PageTransform {
                        rotation_degrees: Some(-90),
                        media_box: Some([0., 0., 340., 300.]),
                        crop_box: Some([10., 10., 290., 250.]),
                        ..Default::default()
                    },
                }],
                ..request(NAME, CODE)
            },
            &ctx,
        )
        .unwrap();
    assert_initial_effects(&plan);
    assert_eq!(revision.snapshot(), before);
    let geometry = plan.geometry_effects().next().unwrap();
    assert_eq!(geometry.page, reference(3));
    let expected_geometry = geometry.after;
    let receipt = revision.commit(&plan, &ctx).unwrap();
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), before.source());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(revision.page_geometry(1).unwrap(), expected_geometry);
    assert_eq!(revision.page_geometry(1).unwrap().rotation_degrees, 270);
    assert_eq!(list_form_fields(&original).unwrap(), fields);
    assert_outputs(
        &revision,
        &[("group.name", NAME), ("code", CODE)],
        &[],
        [4, 3],
    );
}

#[test]
fn clearing_and_refilling_use_the_committed_value_without_changing_defaults() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let first = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![1, 0]),
                ..request(NAME, CODE)
            },
            &ctx,
        )
        .unwrap();
    revision.commit(&first, &ctx).unwrap();
    let mut before = [NAME, CODE];
    for (index, next) in [["", ""], [NAME, CODE], ["Café", "Ready"]]
        .into_iter()
        .enumerate()
    {
        let plan = revision.plan(request(next[0], next[1]), &ctx).unwrap();
        let effects = plan.text_field_effects().collect::<Vec<_>>();
        for (position, number) in [8, 12].into_iter().enumerate() {
            let effect = effects
                .iter()
                .find(|effect| effect.field == reference(number))
                .unwrap();
            assert_eq!(effect.before_value, before[position]);
            assert_eq!(effect.after_value, next[position]);
        }
        revision.commit(&plan, &ctx).unwrap();
        assert_eq!(revision.snapshot().revision().value(), index as u64 + 2);
        assert_outputs(
            &revision,
            &[("group.name", next[0]), ("code", next[1])],
            &[],
            [4, 3],
        );
        before = next;
    }
}

#[test]
fn filling_then_flattening_preserves_new_text_and_the_editable_sibling() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let plan = revision.plan(request(NAME, CODE), &ctx).unwrap();
    let appearances = plan
        .text_field_effects()
        .find(|effect| effect.field == reference(8))
        .unwrap()
        .widgets
        .iter()
        .map(|effect| (effect.widget, effect.output_appearance))
        .collect::<Vec<_>>();
    revision.commit(&plan, &ctx).unwrap();
    let flattened = revision
        .plan(
            PdfRevisionRequest {
                flatten_fields: vec![flatten_name()],
                reorder_pages: Some(vec![1, 0]),
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    for widget in &flattened.flatten_effects().next().unwrap().widgets {
        assert!(appearances.contains(&(widget.widget, widget.appearance)));
    }
    revision.commit(&flattened, &ctx).unwrap();
    assert_outputs(&revision, &[("code", CODE)], &["group.name"], [4, 3]);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        let reopened = open(output.output.as_bytes());
        for page in 0..2 {
            assert!(
                reopened
                    .extract_page_text_spans(page)
                    .unwrap()
                    .spans
                    .iter()
                    .any(|span| span.text == NAME)
            );
        }
    }
    let later = revision
        .plan(
            PdfRevisionRequest {
                text_fields: vec![fill("code", "Later")],
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    assert_eq!(
        later.text_field_effects().next().unwrap().before_value,
        CODE
    );
    revision.commit(&later, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 3);
    assert_outputs(&revision, &[("code", "Later")], &["group.name"], [4, 3]);
    assert!(revision.plan(request(NAME, CODE), &ctx).is_err());
}

#[test]
fn invalid_fields_values_or_composed_form_operations_do_not_partially_fill() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let ctx = context();
    let mut occurrence = fill("group.name", NAME);
    occurrence.match_index = 1;
    for fields in [
        vec![fill("code", CODE), fill("missing", "value")],
        vec![fill("group.name", NAME), fill("group.name", "Other")],
        vec![fill("group.name", NAME), fill("group.name", NAME)],
        vec![fill("group", "value")],
        vec![fill("", "value")],
        vec![occurrence],
        vec![fill("code", CODE), fill("group.name", &"é".repeat(25))],
        vec![fill("code", CODE), fill("group.name", "Unmapped 漢")],
        vec![fill("code", CODE), fill("group.name", "Line\nbreak")],
    ] {
        assert!(
            revision
                .plan(
                    PdfRevisionRequest {
                        text_fields: fields,
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
    for request in [
        PdfRevisionRequest {
            flatten_fields: vec![flatten_name()],
            ..request(NAME, CODE)
        },
        PdfRevisionRequest {
            checkboxes: vec![CheckboxFieldMutationRequest {
                field_name: "accept".into(),
                checked: false,
                match_index: 0,
            }],
            ..request(NAME, CODE)
        },
    ] {
        assert_eq!(
            revision.plan(request, &ctx).unwrap_err().code,
            PdfErrorCode::UnsupportedFeature
        );
        assert_eq!(revision.snapshot(), before);
    }
    assert_outputs(&revision, &[], &[], [3, 4]);
    // MaxLen is a Unicode character count: 24 mapped é characters fit this
    // fixture's supplied metrics despite needing more than 24 UTF-8 bytes.
    let maximum = "é".repeat(24);
    let toggle = revision
        .plan(
            PdfRevisionRequest {
                checkboxes: vec![CheckboxFieldMutationRequest {
                    field_name: "accept".into(),
                    checked: false,
                    match_index: 0,
                }],
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    revision.commit(&toggle, &ctx).unwrap();
    let valid = revision.plan(request(&maximum, CODE), &ctx).unwrap();
    revision.commit(&valid, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 2);
    assert_outputs(
        &revision,
        &[("group.name", &maximum), ("code", CODE), ("accept", "Off")],
        &[],
        [3, 4],
    );
}

#[test]
fn cancelled_or_exhausted_planning_and_commits_can_retry_without_partial_values() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let ctx = context();
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision
            .plan(request(NAME, CODE), &cancelled)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    for limited in [
        limited_context(0, 512 * 1024 * 1024),
        limited_context(128 * 1024 * 1024, 0),
    ] {
        assert_eq!(
            revision
                .plan(request(NAME, CODE), &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), before);
    }
    let plan = revision.plan(request(NAME, CODE), &ctx).unwrap();
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    let limited = limited_context(128 * 1024 * 1024, plan.changed_objects().count() as u64);
    assert!(revision.commit(&plan, &limited).is_err());
    assert_eq!(revision.snapshot(), before);
    assert_outputs(&revision, &[], &[], [3, 4]);
    revision.commit(&plan, &ctx).unwrap();
    let committed = revision.snapshot();
    assert!(matches!(
        revision.commit(&plan, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(revision.snapshot(), committed);
    assert_outputs(
        &revision,
        &[("group.name", NAME), ("code", CODE)],
        &[],
        [3, 4],
    );
}

#[test]
fn other_sources_and_equal_numbered_divergent_branches_cannot_commit_foreign_fills() {
    let mut left = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let initial = left.plan(request(NAME, CODE), &ctx).unwrap();
    let mut bytes = SOURCE.to_vec();
    let marker = b"/Title (";
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    bytes[offset] = if bytes[offset] == b'O' { b'o' } else { b'O' };
    let mut different = PdfRevision::new(open(&bytes));
    let before = different.snapshot();
    assert!(matches!(
        different.commit(&initial, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(different.snapshot(), before);
    let mut right = left.clone();
    for (revision, value) in [(&mut left, "Left"), (&mut right, "Right")] {
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    text_fields: vec![fill("code", value)],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        revision.commit(&plan, &ctx).unwrap();
    }
    assert_eq!(left.snapshot(), right.snapshot());
    let foreign = left.plan(request(NAME, CODE), &ctx).unwrap();
    let before = right.snapshot();
    assert!(
        matches!(right.commit(&foreign, &ctx), Err(TransactionError::Stage(error)) if error.code == PdfErrorCode::UnsafeRewrite)
    );
    assert_eq!(right.snapshot(), before);
    assert_outputs(&right, &[("code", "Right")], &[], [3, 4]);
    let own = right.plan(request(NAME, CODE), &ctx).unwrap();
    right.commit(&own, &ctx).unwrap();
    assert_outputs(&right, &[("group.name", NAME), ("code", CODE)], &[], [3, 4]);
}

#[test]
fn failed_export_retries_keep_the_committed_values_and_prior_page_order() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![1, 0]),
                ..request(NAME, CODE)
            },
            &ctx,
        )
        .unwrap();
    revision.commit(&plan, &ctx).unwrap();
    let committed = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert_eq!(
            revision.export(mode, &cancelled).unwrap_err().code,
            PdfErrorCode::Cancelled
        );
        let limited = limited_context(128 * 1024 * 1024, 0);
        assert_eq!(
            revision.export(mode, &limited).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), committed);
        let retried = revision.export(mode, &ctx).unwrap();
        let again = revision.export(mode, &ctx).unwrap();
        assert_eq!(retried.output.id(), again.output.id());
        assert_eq!(retried.snapshot, committed);
    }
    assert_outputs(
        &revision,
        &[("group.name", NAME), ("code", CODE)],
        &[],
        [4, 3],
    );
}

#[test]
fn configured_output_limit_does_not_discard_a_fill_or_prevent_later_planning() {
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
    let plan = revision.plan(request(NAME, CODE), &ctx).unwrap();
    revision.commit(&plan, &ctx).unwrap();
    let committed = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        assert_eq!(
            revision.export(mode, &ctx).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), committed);
    }
    let next = revision.plan(request("", "Later"), &ctx).unwrap();
    for effect in next.text_field_effects() {
        assert_eq!(
            effect.before_value,
            if effect.field == reference(8) {
                NAME
            } else {
                CODE
            }
        );
    }
    revision.commit(&next, &ctx).unwrap();
    let verify = revision.plan(request(NAME, CODE), &ctx).unwrap();
    for effect in verify.text_field_effects() {
        assert_eq!(
            effect.before_value,
            if effect.field == reference(8) {
                ""
            } else {
                "Later"
            }
        );
    }
    assert_eq!(revision.snapshot().revision().value(), 2);
}
