//! Retained page duplication must create independent editable form occurrences.
//! Exact shared-resource/AP payload and rendering checks belong to the oracle.
use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget,
    transaction::TransactionError,
};
use binas_pdf::{
    EngineConfig, FormWidgetRef, Limits, OpenOptions, PageTransform, PdfDocument,
    PdfDuplicateFieldName, PdfEngine, PdfErrorCode, PdfFieldFlattenRequest,
    PdfPageDuplicateRequest, PdfPageTransformRequest, PdfReferenceIndex, PdfReferenceOrigin,
    PdfReferencePathStep, PdfReferenceTarget, PdfRevision, PdfRevisionPlan, PdfRevisionRequest,
    PdfTextFieldFillRequest, PdfWriterMode, list_annotations, list_form_fields, read_document_info,
    read_embedded_attachment_bytes, read_embedded_attachments, read_named_destinations,
    read_outlines, read_page_labels, read_xmp_metadata,
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
fn rename(field_name: &str, new_field_name: &str) -> PdfDuplicateFieldName {
    PdfDuplicateFieldName {
        field_name: field_name.into(),
        new_field_name: new_field_name.into(),
    }
}
fn duplicate() -> PdfPageDuplicateRequest {
    PdfPageDuplicateRequest {
        page_index: 0,
        // Mapping order does not replace the source page's annotation order.
        fields: vec![
            rename("group.keep", "group.keep_copy"),
            rename("group.name", "group.name_copy"),
        ],
    }
}
fn request() -> PdfRevisionRequest {
    PdfRevisionRequest {
        duplicate_page: Some(duplicate()),
        ..Default::default()
    }
}
fn fill(values: &[(&str, &str)]) -> PdfRevisionRequest {
    PdfRevisionRequest {
        text_fields: values
            .iter()
            .map(|(name, value)| PdfTextFieldFillRequest {
                field_name: (*name).into(),
                value: (*value).into(),
                match_index: 0,
            })
            .collect(),
        ..Default::default()
    }
}

#[derive(Clone, Copy)]
struct Copies {
    page: PdfReferenceTarget,
    name: PdfReferenceTarget,
    name_widget: PdfReferenceTarget,
    keep: PdfReferenceTarget,
    link: PdfReferenceTarget,
}

fn copies(plan: &PdfRevisionPlan) -> Copies {
    let effect = plan.duplicate_effect().unwrap();
    assert_eq!(effect.source_page, reference(3));
    assert_eq!(effect.page_index, 0);
    assert_eq!(effect.inserted_index, 1);
    assert!(![reference(3), reference(4)].contains(&effect.page));
    assert_eq!(effect.page.generation, 0);
    assert_eq!(
        effect
            .fields
            .iter()
            .map(|field| field.new_field_name.as_str())
            .collect::<Vec<_>>(),
        ["group.name_copy", "group.keep_copy"]
    );
    let name = &effect.fields[0];
    let keep = &effect.fields[1];
    assert_eq!(name.source_field, reference(8));
    assert_eq!(name.field_name, "group.name");
    assert_eq!(keep.source_field, reference(11));
    assert_eq!(keep.field_name, "group.keep");
    assert_eq!(name.widgets.len(), 1);
    assert_eq!(name.widgets[0].source_annotation, reference(9));
    assert_eq!(keep.widgets.len(), 1);
    assert_eq!(keep.widgets[0].source_annotation, reference(11));
    assert_eq!(keep.widgets[0].annotation, keep.field);
    assert_ne!(name.widgets[0].annotation, name.field);
    assert_eq!(
        effect
            .annotations
            .iter()
            .map(|annotation| annotation.source_annotation)
            .collect::<Vec<_>>(),
        [reference(9), reference(11), reference(18)]
    );
    assert_eq!(effect.annotations[0].annotation, name.widgets[0].annotation);
    assert_eq!(effect.annotations[1].annotation, keep.field);
    let result = Copies {
        page: effect.page,
        name: name.field,
        name_widget: name.widgets[0].annotation,
        keep: keep.field,
        link: effect.annotations[2].annotation,
    };
    let new = [
        result.page,
        result.name,
        result.name_widget,
        result.keep,
        result.link,
    ];
    assert!(
        new.iter()
            .all(|value| value.generation == 0 && value.number >= 35)
    );
    assert_eq!(
        new.into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        5
    );
    result
}

fn assert_original(revision: &PdfRevision) {
    assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &context()).unwrap();
        let reopened = open(output.output.as_bytes());
        let original = open(SOURCE);
        assert_eq!(
            list_form_fields(&reopened).unwrap(),
            list_form_fields(&original).unwrap()
        );
        assert_eq!(
            list_annotations(&reopened).unwrap(),
            list_annotations(&original).unwrap()
        );
    }
}

fn assert_outputs(
    revision: &PdfRevision,
    copied: Copies,
    values: &[(&str, &str)],
    flattened: bool,
    order: [PdfReferenceTarget; 3],
) {
    let original = open(SOURCE);
    let mut fields = list_form_fields(&original).unwrap();
    let mut new_fields = Vec::new();
    for (source_name, new_name, object, widget) in [
        (
            "group.name",
            "group.name_copy",
            copied.name,
            copied.name_widget,
        ),
        ("group.keep", "group.keep_copy", copied.keep, copied.keep),
    ] {
        let mut field = fields
            .iter()
            .find(|field| field.name == source_name)
            .unwrap()
            .clone();
        field.name = new_name.into();
        field.object_number = Some(object.number);
        field.object_generation = Some(object.generation);
        field.widget_refs = vec![FormWidgetRef {
            object_number: widget.number,
            object_generation: widget.generation,
        }];
        new_fields.push(field);
    }
    if !flattened {
        fields
            .iter_mut()
            .find(|field| field.name == "group")
            .unwrap()
            .widget_refs
            .push(FormWidgetRef {
                object_number: copied.keep.number,
                object_generation: copied.keep.generation,
            });
        fields.splice(3..3, new_fields);
    }
    for (index, field) in fields.iter_mut().enumerate() {
        field.index = index;
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == field.name) {
            field.value = Some((*value).into());
        }
    }
    let mut annotations = list_annotations(&original).unwrap();
    let mut cloned_annotations = Vec::new();
    for (source, target) in [
        (9, copied.name_widget),
        (11, copied.keep),
        (18, copied.link),
    ] {
        if flattened && source != 18 {
            continue;
        }
        let mut annotation = annotations
            .iter()
            .find(|annotation| annotation.object_number == Some(source))
            .unwrap()
            .clone();
        annotation.object_number = Some(target.number);
        annotation.object_generation = Some(target.generation);
        annotation.page_object_number = copied.page.number;
        annotation.page_object_generation = copied.page.generation;
        cloned_annotations.push(annotation);
    }
    annotations.extend(cloned_annotations);
    annotations.sort_by_key(|annotation| {
        (
            order
                .iter()
                .position(|page| page.number == annotation.page_object_number)
                .unwrap(),
            annotation.index,
        )
    });
    for (index, annotation) in annotations.iter_mut().enumerate() {
        annotation.index = index;
        annotation.page_index = order
            .iter()
            .position(|page| page.number == annotation.page_object_number)
            .unwrap();
    }
    assert_eq!(revision.page_ids().unwrap(), order);
    assert_eq!(revision.annotations().unwrap(), annotations);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &context()).unwrap();
        assert_eq!(output.snapshot, revision.snapshot());
        assert_eq!(output.mode, mode);
        if mode == PdfWriterMode::Incremental {
            assert!(output.output.as_bytes().starts_with(SOURCE));
        }
        let reopened = open(output.output.as_bytes());
        assert_eq!(list_form_fields(&reopened).unwrap(), fields);
        assert_eq!(list_annotations(&reopened).unwrap(), annotations);
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
        assert_eq!(destinations[0].name, "bravo");
        assert_eq!(
            destinations[0].page_index,
            order.iter().position(|page| *page == reference(4)).unwrap()
        );
        let attachments = read_embedded_attachments(&reopened).unwrap();
        let source_attachments = read_embedded_attachments(&original).unwrap();
        assert_eq!(attachments, source_attachments);
        for (before, after) in source_attachments.iter().zip(&attachments) {
            assert_eq!(
                read_embedded_attachment_bytes(&reopened, after).unwrap(),
                read_embedded_attachment_bytes(&original, before).unwrap()
            );
        }
        let labels = read_page_labels(&reopened).unwrap();
        for (index, page) in order.iter().enumerate() {
            assert_eq!(
                labels[index].label,
                if *page == reference(4) { "B-1" } else { "A-1" }
            );
            assert_eq!(
                reopened.page_geometry(index).unwrap(),
                revision.page_geometry(index).unwrap()
            );
            if !(flattened && *page == copied.page) {
                let source_index = usize::from(*page == reference(4));
                assert_eq!(
                    reopened
                        .extract_page_text_spans(index)
                        .unwrap()
                        .spans
                        .iter()
                        .map(|span| &span.text)
                        .collect::<Vec<_>>(),
                    original
                        .extract_page_text_spans(source_index)
                        .unwrap()
                        .spans
                        .iter()
                        .map(|span| &span.text)
                        .collect::<Vec<_>>()
                );
            }
        }
        let references = PdfReferenceIndex::build(&reopened, &context()).unwrap();
        assert!(references.records().iter().all(|edge| !edge.dangling));
        // Duplication keeps an explicit link's target at the original page4.
        assert!(references.records().iter().any(|edge| edge.origin
            == PdfReferenceOrigin::Object(copied.link)
            && edge.target == reference(4)
            && edge.path
                == [
                    PdfReferencePathStep::Key(b"Dest".to_vec()),
                    PdfReferencePathStep::Index(0)
                ]));
        if !flattened {
            for widget in [copied.name_widget, copied.keep] {
                assert!(
                    references
                        .records()
                        .iter()
                        .any(|edge| edge.origin == PdfReferenceOrigin::Object(widget)
                            && edge.target == copied.page
                            && edge.path == [PdfReferencePathStep::Key(b"P".to_vec())])
                );
            }
        }
        assert_eq!(PdfRevision::new(reopened).page_ids().unwrap(), order);
    }
}

#[test]
fn duplicate_creates_one_page_and_independent_registered_field_occurrences() {
    let original = open(SOURCE);
    let source_fields = list_form_fields(&original).unwrap();
    let mut revision = PdfRevision::new(original.clone());
    let before = revision.snapshot();
    let ctx = context();
    let plan = revision.plan(request(), &ctx).unwrap();
    let copied = copies(&plan);
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    let receipt = revision.commit(&plan, &ctx).unwrap();
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), before.source());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(list_form_fields(&original).unwrap(), source_fields);
    assert_outputs(
        &revision,
        copied,
        &[],
        false,
        [reference(3), copied.page, reference(4)],
    );
}

#[test]
fn fills_on_copied_and_original_fields_do_not_change_each_other() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let plan = revision.plan(request(), &ctx).unwrap();
    let copied = copies(&plan);
    revision.commit(&plan, &ctx).unwrap();
    let copied_values = [
        ("group.name_copy", "Copy name"),
        ("group.keep_copy", "Copy keep"),
    ];
    let copy_fill = revision.plan(fill(&copied_values), &ctx).unwrap();
    for effect in copy_fill.text_field_effects() {
        assert_eq!(effect.widgets.len(), 1);
        assert_eq!(effect.widgets[0].page, copied.page);
    }
    revision.commit(&copy_fill, &ctx).unwrap();
    let order = [reference(3), copied.page, reference(4)];
    assert_outputs(&revision, copied, &copied_values, false, order);
    let originals = [
        ("group.name", "Original name"),
        ("group.keep", "Original keep"),
    ];
    let original_fill = revision.plan(fill(&originals), &ctx).unwrap();
    let name = original_fill
        .text_field_effects()
        .find(|effect| effect.field == reference(8))
        .unwrap();
    assert_eq!(
        name.widgets
            .iter()
            .map(|widget| widget.page)
            .collect::<Vec<_>>(),
        [reference(3), reference(4)]
    );
    revision.commit(&original_fill, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 3);
    assert_outputs(
        &revision,
        copied,
        &[
            copied_values[0],
            copied_values[1],
            originals[0],
            originals[1],
        ],
        false,
        order,
    );
}

#[test]
fn copied_fields_can_be_filled_then_flattened_and_reordered_without_touching_originals() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let plan = revision.plan(request(), &ctx).unwrap();
    let copied = copies(&plan);
    revision.commit(&plan, &ctx).unwrap();
    let values = [
        ("group.name_copy", "Copy alpha"),
        ("group.keep_copy", "Copy beta"),
    ];
    let filled = revision.plan(fill(&values), &ctx).unwrap();
    revision.commit(&filled, &ctx).unwrap();
    let flattened = revision
        .plan(
            PdfRevisionRequest {
                flatten_fields: values
                    .iter()
                    .map(|(name, _)| PdfFieldFlattenRequest {
                        field_name: (*name).into(),
                        match_index: 0,
                    })
                    .collect(),
                reorder_pages: Some(vec![2, 1, 0]),
                ..Default::default()
            },
            &ctx,
        )
        .unwrap();
    assert!(
        flattened
            .flatten_effects()
            .flat_map(|effect| &effect.widgets)
            .all(|widget| widget.page == copied.page)
    );
    revision.commit(&flattened, &ctx).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 3);
    assert_outputs(
        &revision,
        copied,
        &[],
        true,
        [reference(4), copied.page, reference(3)],
    );
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        let reopened = open(output.output.as_bytes());
        let text = reopened.extract_page_text_spans(1).unwrap();
        for (_, value) in values {
            assert!(text.spans.iter().any(|span| span.text == value));
        }
    }
    assert!(revision.plan(fill(&values), &ctx).is_err());
    let original = revision
        .plan(fill(&[("group.name", "Still editable")]), &ctx)
        .unwrap();
    revision.commit(&original, &ctx).unwrap();
    assert_outputs(
        &revision,
        copied,
        &[("group.name", "Still editable")],
        true,
        [reference(4), copied.page, reference(3)],
    );
}

#[test]
fn invalid_rename_maps_pages_and_mixed_batches_publish_no_duplicate() {
    let revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let before = revision.snapshot();
    let valid = duplicate();
    for fields in [
        vec![],
        vec![rename("group.name", "group.name_copy")],
        vec![
            rename("group.name", "group.name_copy"),
            rename("group.keep", "group.keep_copy"),
            rename("code", "code_copy"),
        ],
        vec![
            rename("missing", "group.name_copy"),
            rename("group.keep", "group.keep_copy"),
        ],
        vec![
            rename("group.name", "group.name_copy"),
            rename("group.name", "group.other"),
        ],
        vec![
            rename("group.name", "group.same"),
            rename("group.keep", "group.same"),
        ],
        vec![
            rename("group.name", "group.keep"),
            rename("group.keep", "group.keep_copy"),
        ],
        vec![
            rename("group.name", "different.name_copy"),
            rename("group.keep", "group.keep_copy"),
        ],
        vec![
            rename("group.name", "group."),
            rename("group.keep", "group.keep_copy"),
        ],
        vec![
            rename("group.name", "group.extra.name"),
            rename("group.keep", "group.keep_copy"),
        ],
        vec![
            rename("group.name", "group.bad\nname"),
            rename("group.keep", "group.keep_copy"),
        ],
    ] {
        assert!(
            revision
                .plan(
                    PdfRevisionRequest {
                        duplicate_page: Some(PdfPageDuplicateRequest {
                            fields,
                            ..valid.clone()
                        }),
                        ..Default::default()
                    },
                    &ctx
                )
                .is_err()
        );
        assert_eq!(revision.snapshot(), before);
    }
    assert!(
        revision
            .plan(
                PdfRevisionRequest {
                    duplicate_page: Some(PdfPageDuplicateRequest {
                        page_index: 2,
                        ..valid.clone()
                    }),
                    ..Default::default()
                },
                &ctx
            )
            .is_err()
    );
    for mixed in [
        PdfRevisionRequest {
            duplicate_page: Some(valid.clone()),
            reorder_pages: Some(vec![1, 0]),
            ..Default::default()
        },
        PdfRevisionRequest {
            duplicate_page: Some(valid),
            ..fill(&[("group.name", "value")])
        },
    ] {
        assert_eq!(
            revision.plan(mixed, &ctx).unwrap_err().code,
            PdfErrorCode::UnsupportedFeature
        );
        assert_eq!(revision.snapshot(), before);
    }
    assert_original(&revision);
}

#[test]
fn cancelled_or_limited_duplicate_plans_and_commits_can_retry_atomically() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let before = revision.snapshot();
    let ctx = context();
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision.plan(request(), &cancelled).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    for limited in [
        limited_context(0, 512 * 1024 * 1024),
        limited_context(128 * 1024 * 1024, 0),
    ] {
        assert_eq!(
            revision.plan(request(), &limited).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), before);
    }
    let plan = revision.plan(request(), &ctx).unwrap();
    let copied = copies(&plan);
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    let exhausted = limited_context(128 * 1024 * 1024, plan.changed_objects().count() as u64);
    assert!(revision.commit(&plan, &exhausted).is_err());
    assert_eq!(revision.snapshot(), before);
    assert_original(&revision);
    revision.commit(&plan, &ctx).unwrap();
    let committed = revision.snapshot();
    assert!(matches!(
        revision.commit(&plan, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(revision.snapshot(), committed);
    assert_outputs(
        &revision,
        copied,
        &[],
        false,
        [reference(3), copied.page, reference(4)],
    );
}

#[test]
fn source_identity_and_divergent_revisions_reject_foreign_duplicate_plans() {
    let mut left = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let plan = left.plan(request(), &ctx).unwrap();
    let mut changed = SOURCE.to_vec();
    let marker = b"Original retained text-field fixture";
    let offset = changed
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    changed[offset] = b'o';
    let mut other = PdfRevision::new(open(&changed));
    let before = other.snapshot();
    assert!(matches!(
        other.commit(&plan, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(other.snapshot(), before);
    let mut right = left.clone();
    for (revision, rotation) in [(&mut left, 90), (&mut right, 180)] {
        let geometry = revision
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
        revision.commit(&geometry, &ctx).unwrap();
    }
    assert_eq!(left.snapshot(), right.snapshot());
    let foreign = left.plan(request(), &ctx).unwrap();
    let before = right.snapshot();
    assert!(
        matches!(right.commit(&foreign, &ctx), Err(TransactionError::Stage(error)) if error.code == PdfErrorCode::UnsafeRewrite)
    );
    assert_eq!(right.snapshot(), before);
    assert_eq!(right.page_geometry(0).unwrap().rotation_degrees, 180);
    let own = right.plan(request(), &ctx).unwrap();
    let copied = copies(&own);
    right.commit(&own, &ctx).unwrap();
    assert_eq!(
        right.page_geometry(1).unwrap(),
        right.page_geometry(0).unwrap()
    );
    assert_outputs(
        &right,
        copied,
        &[],
        false,
        [reference(3), copied.page, reference(4)],
    );
}

#[test]
fn duplication_uses_committed_values_and_geometry_and_export_failures_can_retry() {
    let mut revision = PdfRevision::new(open(SOURCE));
    let ctx = context();
    let original_values = [("group.name", "New name"), ("group.keep", "New keep")];
    let edit = revision
        .plan(
            PdfRevisionRequest {
                page_transforms: vec![PdfPageTransformRequest {
                    page_index: 0,
                    transform: PageTransform {
                        rotation_degrees: Some(90),
                        ..Default::default()
                    },
                }],
                ..fill(&original_values)
            },
            &ctx,
        )
        .unwrap();
    revision.commit(&edit, &ctx).unwrap();
    let plan = revision.plan(request(), &ctx).unwrap();
    let copied = copies(&plan);
    revision.commit(&plan, &ctx).unwrap();
    assert_eq!(
        revision.page_geometry(0).unwrap(),
        revision.page_geometry(1).unwrap()
    );
    let committed = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert_eq!(
            revision.export(mode, &cancelled).unwrap_err().code,
            PdfErrorCode::Cancelled
        );
        assert_eq!(
            revision
                .export(mode, &limited_context(128 * 1024 * 1024, 0))
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), committed);
        assert_eq!(revision.export(mode, &ctx).unwrap().snapshot, committed);
    }
    assert_outputs(
        &revision,
        copied,
        &[
            original_values[0],
            original_values[1],
            ("group.name_copy", "New name"),
            ("group.keep_copy", "New keep"),
        ],
        false,
        [reference(3), copied.page, reference(4)],
    );
}

#[test]
fn configured_output_failure_keeps_the_duplicate_available_for_later_edits() {
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
    let plan = revision.plan(request(), &ctx).unwrap();
    let copied = copies(&plan);
    revision.commit(&plan, &ctx).unwrap();
    let committed = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        assert_eq!(
            revision.export(mode, &ctx).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), committed);
    }
    let edit = revision
        .plan(fill(&[("group.name_copy", "Still here")]), &ctx)
        .unwrap();
    assert_eq!(
        edit.text_field_effects().next().unwrap().before_value,
        "Old"
    );
    revision.commit(&edit, &ctx).unwrap();
    let original = revision
        .plan(fill(&[("group.name", "Original")]), &ctx)
        .unwrap();
    assert_eq!(
        original.text_field_effects().next().unwrap().before_value,
        "Old"
    );
    let updated = revision
        .plan(fill(&[("group.name_copy", "Next")]), &ctx)
        .unwrap();
    assert_eq!(
        updated.text_field_effects().next().unwrap().before_value,
        "Still here"
    );
    assert_eq!(revision.snapshot().revision().value(), 2);
    assert_eq!(
        revision.page_ids().unwrap(),
        [reference(3), copied.page, reference(4)]
    );
}
