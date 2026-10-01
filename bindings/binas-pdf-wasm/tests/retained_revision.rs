//! Browser execution of the shared Rust revision API, not new public JS bindings.
//! Native candidate bytes were independently checked by qa/preservation.
use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget, RevisionId,
    transaction::TransactionError, verification::CheckStatus,
};
use binas_pdf::{
    OpenOptions, PdfEngine, PdfErrorCode, PdfPageTextEditRequest, PdfRevision, PdfRevisionRequest,
    PdfSharedEditMode, PdfWriterMode, SurgicalTextEditRequest, list_form_fields,
    read_document_info, read_embedded_attachment_bytes, read_embedded_attachments,
    read_named_destinations, read_outlines, read_page_labels,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::*;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test_configure!(run_in_browser);

const STRUCTURED: &[u8] =
    include_bytes!("../../../qa/preservation/fixtures/reorder-structures.pdf");
const ALPHA: &[u8] = include_bytes!("../../../qa/preservation/fixtures/alpha-boundary.pdf");
const CONTENT_SEQUENCE: &[u8] =
    include_bytes!("../../../qa/preservation/fixtures/content-sequence.pdf");
const FLATE_STORAGE: &[u8] = include_bytes!("../../../qa/preservation/fixtures/flate-storage.pdf");
const CHECKBOX_STATES: &[u8] =
    include_bytes!("../../../qa/preservation/fixtures/checkbox-states.pdf");
const TARGETED_FLATTEN: &[u8] =
    include_bytes!("../../../qa/preservation/fixtures/targeted-flatten.pdf");
const TEXT_FIELDS: &[u8] = include_bytes!("../../../qa/preservation/fixtures/text-field-fill.pdf");
const LOSSLESS_IMAGES: &[u8] =
    include_bytes!("../../../qa/preservation/fixtures/lossless-images.pdf");

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn page_deletion_remaps_navigation_keeps_fields_usable_and_matches_checked_outputs() {
    use binas_pdf::{PdfExcludedPageTargetPolicy, PdfPageDeleteRequest, PdfTextFieldFillRequest};
    let mut document = revision(TEXT_FIELDS);
    let source = document.snapshot();
    assert!(
        document
            .plan(
                PdfRevisionRequest {
                    delete_page: Some(PdfPageDeleteRequest {
                        page_index: 1,
                        excluded_targets: PdfExcludedPageTargetPolicy::Reject
                    }),
                    ..Default::default()
                },
                &context()
            )
            .is_err()
    );
    assert_eq!(document.snapshot(), source);
    let plan = document
        .plan(
            PdfRevisionRequest {
                delete_page: Some(PdfPageDeleteRequest {
                    page_index: 1,
                    excluded_targets: PdfExcludedPageTargetPolicy::RemapToPage { page_index: 0 },
                }),
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
    let effect = plan.delete_effect().unwrap();
    assert_eq!(effect.page.number, 4);
    assert_eq!(
        effect
            .remaining_pages
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [3]
    );
    assert_eq!(
        effect
            .removed_widgets
            .iter()
            .map(|widget| widget.number)
            .collect::<Vec<_>>(),
        [10, 12, 13]
    );
    assert_eq!(
        effect
            .unregistered_fields
            .iter()
            .map(|field| field.number)
            .collect::<Vec<_>>(),
        [12, 13]
    );
    assert_eq!(effect.remapped_destinations.len(), 3);
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert!(matches!(
        document.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(document.snapshot(), source);
    let receipt = document.commit(&plan, &context()).unwrap();
    assert_eq!(receipt.before, source);
    assert_eq!(receipt.after.revision().value(), 1);
    assert!(matches!(
        document.commit(&plan, &context()),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(
        document
            .page_ids()
            .unwrap()
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [3]
    );
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = document.export(mode, &context()).unwrap();
        let expected: &[u8] = match mode {
            PdfWriterMode::Incremental => {
                include_bytes!("../../../qa/preservation/evidence/delete-incremental.pdf")
            }
            PdfWriterMode::Compact => {
                include_bytes!("../../../qa/preservation/evidence/delete-compact.pdf")
            }
            PdfWriterMode::Surgical => unreachable!(),
        };
        assert_eq!(output.output.as_bytes(), expected);
        assert_eq!(output.snapshot, receipt.after);
        if mode == PdfWriterMode::Incremental {
            assert!(output.output.as_bytes().starts_with(TEXT_FIELDS));
        }
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        assert_eq!(reopened.inspect().unwrap().page_count, 1);
        assert_eq!(reopened.inspect().unwrap().object_count, 34);
        assert_eq!(read_named_destinations(&reopened).unwrap()[0].page_index, 0);
        assert_eq!(read_page_labels(&reopened).unwrap()[0].label, "A-1");
        let fields = list_form_fields(&reopened).unwrap();
        assert_eq!(fields.len(), 3);
        let field = fields
            .iter()
            .find(|field| field.name == "group.name")
            .unwrap();
        assert_eq!(field.value.as_deref(), Some("Old"));
        assert_eq!(field.default_value.as_deref(), Some("Default name"));
        assert_eq!(
            field
                .widget_refs
                .iter()
                .map(|widget| widget.object_number)
                .collect::<Vec<_>>(),
            [9]
        );
    }
    assert!(
        document
            .plan(
                PdfRevisionRequest {
                    delete_page: Some(PdfPageDeleteRequest {
                        page_index: 0,
                        excluded_targets: PdfExcludedPageTargetPolicy::Reject
                    }),
                    ..Default::default()
                },
                &context()
            )
            .is_err()
    );
    assert_eq!(document.snapshot(), receipt.after);
    let fill = document
        .plan(
            PdfRevisionRequest {
                text_fields: vec![PdfTextFieldFillRequest {
                    field_name: "group.name".into(),
                    value: "Café".into(),
                    match_index: 0,
                }],
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
    let effect = fill.text_field_effects().next().unwrap();
    assert_eq!(effect.widgets.len(), 1);
    assert_eq!(effect.widgets[0].widget.number, 9);
    document.commit(&fill, &context()).unwrap();
    assert_eq!(document.snapshot().revision().value(), 2);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = document.export(mode, &context()).unwrap();
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        let name = fields
            .iter()
            .find(|field| field.name == "group.name")
            .unwrap();
        assert_eq!(name.value.as_deref(), Some("Café"));
        assert_eq!(name.default_value.as_deref(), Some("Default name"));
        let keep = fields
            .iter()
            .find(|field| field.name == "group.keep")
            .unwrap();
        assert_eq!(keep.value.as_deref(), Some("Old"));
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn lossless_image_samples_preserve_the_edited_baseline_and_match_checked_outputs() {
    use binas_pdf::PdfLosslessImageSkipReason;
    let mut document = revision(LOSSLESS_IMAGES);
    let source = document.snapshot();
    let edit = document
        .plan(
            PdfRevisionRequest {
                page_text: vec![PdfPageTextEditRequest {
                    page_index: 0,
                    shared: PdfSharedEditMode::Reject,
                    edit: SurgicalTextEditRequest {
                        old_text: "before".into(),
                        replacement: "after".into(),
                        match_index: 0,
                    },
                }],
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
    document.commit(&edit, &context()).unwrap();
    let baseline = document.snapshot();
    assert_eq!(baseline.revision().value(), 1);
    let pages = document.page_ids().unwrap();
    let plan = document.plan_lossless_images(&context()).unwrap();
    assert_eq!(plan.snapshot(), baseline);
    assert_eq!(document.snapshot(), baseline);
    assert_eq!(plan.effects().len(), 2);
    assert_eq!(
        plan.effects()
            .iter()
            .map(|effect| effect.image.number)
            .collect::<Vec<_>>(),
        [7, 8]
    );
    for effect in plan.effects() {
        assert_eq!((effect.width, effect.height), (96, 64));
        assert_eq!((effect.source_predictor, effect.output_predictor), (1, 12));
        assert!(effect.encoded_after < effect.encoded_before);
        assert_eq!(
            effect.decoded_bytes,
            if effect.image.number == 7 {
                18432
            } else {
                6144
            }
        );
    }
    assert!(plan.skips().iter().any(
        |skip| skip.image.number == 13 && skip.reason == PdfLosslessImageSkipReason::NotSmaller
    ));
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert!(matches!(
        document.commit_lossless_images(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(document.snapshot(), baseline);
    let receipt = document
        .commit_lossless_images(&plan, &context())
        .unwrap()
        .unwrap();
    assert_eq!(receipt.before, baseline);
    assert_eq!(receipt.after.revision().value(), 2);
    assert_eq!(document.snapshot().source(), source.source());
    assert_eq!(document.page_ids().unwrap(), pages);
    assert!(matches!(
        document.commit_lossless_images(&plan, &context()),
        Err(TransactionError::StalePlan { .. })
    ));
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = document.export(mode, &context()).unwrap();
        let checked: &[u8] = match mode {
            PdfWriterMode::Incremental => {
                include_bytes!("../../../qa/preservation/evidence/lossless-images-incremental.pdf")
            }
            PdfWriterMode::Compact => {
                include_bytes!("../../../qa/preservation/evidence/lossless-images-compact.pdf")
            }
            PdfWriterMode::Surgical => unreachable!(),
        };
        assert_eq!(output.output.as_bytes(), checked);
        assert_eq!(output.snapshot, receipt.after);
        if mode == PdfWriterMode::Incremental {
            assert!(output.output.as_bytes().starts_with(LOSSLESS_IMAGES));
        }
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        assert_eq!(
            reopened
                .extract_page_text_spans(0)
                .unwrap()
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            ["after"]
        );
        assert_eq!(
            reopened
                .extract_page_text_spans(1)
                .unwrap()
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            ["untouched"]
        );
    }
    let repeated = document.plan_lossless_images(&context()).unwrap();
    assert!(repeated.effects().is_empty());
    assert_eq!(repeated.encoded_bytes_saved(), 0);
    assert!(
        document
            .commit_lossless_images(&repeated, &context())
            .unwrap()
            .is_none()
    );
    assert_eq!(document.snapshot(), receipt.after);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn duplicate_page_keeps_copied_fields_independent_and_matches_checked_outputs() {
    use binas_pdf::{PdfDuplicateFieldName, PdfPageDuplicateRequest, PdfTextFieldFillRequest};
    let mut document = revision(TEXT_FIELDS);
    let original = document.snapshot();
    let pages = document.page_ids().unwrap();
    let plan = document
        .plan(
            PdfRevisionRequest {
                duplicate_page: Some(PdfPageDuplicateRequest {
                    page_index: 0,
                    fields: ["name", "keep"]
                        .map(|name| PdfDuplicateFieldName {
                            field_name: format!("group.{name}"),
                            new_field_name: format!("group.{name}_copy"),
                        })
                        .into(),
                }),
                ..Default::default()
            },
            &context(),
        )
        .unwrap();
    let effect = plan.duplicate_effect().unwrap();
    assert_eq!(effect.source_page, pages[0]);
    assert_eq!(effect.page_index, 0);
    assert_eq!(effect.inserted_index, 1);
    assert_eq!(effect.fields.len(), 2);
    assert_eq!(effect.annotations.len(), 3);
    let fresh_page = effect.page;
    let mut fresh = std::collections::BTreeSet::from([fresh_page.number]);
    for annotation in &effect.annotations {
        assert!(fresh.insert(annotation.annotation.number));
        assert!(annotation.annotation.number > 34);
        assert_ne!(annotation.source_annotation, annotation.annotation);
    }
    for field in &effect.fields {
        assert_eq!(field.widgets.len(), 1);
        assert_ne!(field.field, field.source_field);
    }
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert!(matches!(
        document.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(document.snapshot(), original);
    document.commit(&plan, &context()).unwrap();
    assert_eq!(
        document.page_ids().unwrap(),
        [pages[0], fresh_page, pages[1]]
    );
    assert_eq!(labels(&document), ["A-1", "A-1", "B-1"]);
    assert!(matches!(
        document.commit(&plan, &context()),
        Err(TransactionError::StalePlan { .. })
    ));
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = document.export(mode, &context()).unwrap();
        let checked: &[u8] = match mode {
            PdfWriterMode::Incremental => {
                include_bytes!("../../../qa/preservation/evidence/duplicate-incremental.pdf")
            }
            PdfWriterMode::Compact => {
                include_bytes!("../../../qa/preservation/evidence/duplicate-compact.pdf")
            }
            PdfWriterMode::Surgical => unreachable!(),
        };
        assert_eq!(exported.output.as_bytes(), checked);
        assert_eq!(exported.snapshot, document.snapshot());
        if mode == PdfWriterMode::Incremental {
            assert!(exported.output.as_bytes().starts_with(TEXT_FIELDS));
        }
        let reopened = PdfEngine::default()
            .open_source(exported.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        for (name, default, widgets) in [
            ("group.name", "Default name", 2),
            ("group.name_copy", "Default name", 1),
            ("group.keep", "Old", 1),
            ("group.keep_copy", "Old", 1),
        ] {
            let field = fields.iter().find(|field| field.name == name).unwrap();
            assert_eq!(field.value.as_deref(), Some("Old"));
            assert_eq!(field.default_value.as_deref(), Some(default));
            assert_eq!(field.widget_refs.len(), widgets);
        }
    }
    for (name, value) in [("group.name_copy", "Copied"), ("group.name", "Original")] {
        let fill = document
            .plan(
                PdfRevisionRequest {
                    text_fields: vec![PdfTextFieldFillRequest {
                        field_name: name.into(),
                        value: value.into(),
                        match_index: 0,
                    }],
                    ..Default::default()
                },
                &context(),
            )
            .unwrap();
        document.commit(&fill, &context()).unwrap();
    }
    assert_eq!(document.snapshot().revision().value(), 3);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = document.export(mode, &context()).unwrap();
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        for (name, value, widgets) in [
            ("group.name_copy", "Copied", 1),
            ("group.name", "Original", 2),
            ("group.keep", "Old", 1),
        ] {
            let field = fields.iter().find(|field| field.name == name).unwrap();
            assert_eq!(field.value.as_deref(), Some(value));
            assert_eq!(field.widget_refs.len(), widgets);
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn text_field_fills_keep_shared_appearances_private_and_match_checked_outputs() {
    use binas_pdf::PdfTextFieldFillRequest;
    let mut document = revision(TEXT_FIELDS);
    let original = document.snapshot();
    let operation = context();
    let plan = document
        .plan(
            PdfRevisionRequest {
                text_fields: [("group.name", "Café (A) \\ B"), ("code", "Ready (2)")]
                    .map(|(name, value)| PdfTextFieldFillRequest {
                        field_name: name.into(),
                        value: value.into(),
                        match_index: 0,
                    })
                    .into(),
                reorder_pages: Some(vec![1, 0]),
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    let effects = plan.text_field_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    let mut outputs = std::collections::BTreeSet::new();
    for effect in effects {
        for widget in &effect.widgets {
            assert!(outputs.insert(widget.output_appearance.number));
            assert!(widget.output_appearance.number > 34);
            assert_eq!(widget.font.unwrap().number, 19);
            assert_eq!(widget.cmap.number, 22);
            assert_ne!(widget.source_appearance, widget.output_appearance);
            assert!(widget.ink_bounds[0] >= widget.clip_bounds[0]);
            assert!(widget.ink_bounds[2] <= widget.clip_bounds[2]);
        }
    }
    assert_eq!(outputs.len(), 3);
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert!(matches!(
        document.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(document.snapshot(), original);
    let receipt = document.commit(&plan, &operation).unwrap();
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(labels(&document), ["B-1", "A-1"]);
    assert!(matches!(
        document.commit(&plan, &operation),
        Err(TransactionError::StalePlan { .. })
    ));
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = document.export(mode, &operation).unwrap();
        let checked: &[u8] = match mode {
            PdfWriterMode::Incremental => {
                include_bytes!("../../../qa/preservation/evidence/text-field-incremental.pdf")
            }
            PdfWriterMode::Compact => {
                include_bytes!("../../../qa/preservation/evidence/text-field-compact.pdf")
            }
            PdfWriterMode::Surgical => unreachable!(),
        };
        assert_eq!(exported.output.as_bytes(), checked);
        assert_eq!(exported.snapshot, receipt.after);
        if mode == PdfWriterMode::Incremental {
            assert!(exported.output.as_bytes().starts_with(TEXT_FIELDS));
        }
        let reopened = PdfEngine::default()
            .open_source(exported.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        for (name, value, default, widgets) in [
            ("group.name", "Café (A) \\ B", "Default name", 2),
            ("group.keep", "Old", "Old", 1),
            ("code", "Ready (2)", "Default code", 1),
        ] {
            let field = fields.iter().find(|field| field.name == name).unwrap();
            assert_eq!(field.value.as_deref(), Some(value));
            assert_eq!(field.default_value.as_deref(), Some(default));
            assert_eq!(field.widget_refs.len(), widgets);
        }
    }
    let clear = document
        .plan(
            PdfRevisionRequest {
                text_fields: vec![PdfTextFieldFillRequest {
                    field_name: "code".into(),
                    value: String::new(),
                    match_index: 0,
                }],
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    document.commit(&clear, &operation).unwrap();
    assert_eq!(document.snapshot().revision().value(), 2);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = document.export(mode, &operation).unwrap();
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        assert_eq!(
            fields
                .iter()
                .find(|field| field.name == "code")
                .unwrap()
                .value
                .as_deref(),
            Some("")
        );
        assert_eq!(
            fields
                .iter()
                .find(|field| field.name == "group.keep")
                .unwrap()
                .value
                .as_deref(),
            Some("Old")
        );
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn targeted_flatten_preserves_siblings_and_matches_independent_outputs() {
    use binas_pdf::{CheckboxFieldMutationRequest, PdfFieldFlattenRequest};
    let mut document = revision(TARGETED_FLATTEN);
    let original = document.snapshot();
    let operation = context();
    let plan = document
        .plan(
            PdfRevisionRequest {
                flatten_fields: vec![PdfFieldFlattenRequest {
                    field_name: "group.answer".into(),
                    match_index: 0,
                }],
                reorder_pages: Some(vec![1, 0]),
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    assert_eq!(document.snapshot(), original);
    let effects = plan.flatten_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].field.number, 8);
    assert_eq!(effects[0].widgets.len(), 2);
    for (widget, page, matrix) in [
        (9, 3, [2., 0., 0., 1., 16., 143.]),
        (10, 4, [1., 0., 0., 2., 58., 116.]),
    ] {
        let effect = effects[0]
            .widgets
            .iter()
            .find(|value| value.widget.number == widget)
            .unwrap();
        assert_eq!(effect.page.number, page);
        assert_eq!(effect.appearance.number, 14);
        assert_eq!(effect.resource_name, b"BinasFlatten1");
        assert_eq!(effect.placement_matrix, matrix);
        assert_eq!(effect.annotation_flags, 4);
    }
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert!(matches!(
        document.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(document.snapshot(), original);
    let receipt = document.commit(&plan, &operation).unwrap();
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(labels(&document), ["B-1", "A-1"]);
    assert!(matches!(
        document.commit(&plan, &operation),
        Err(TransactionError::StalePlan { .. })
    ));
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = document.export(mode, &operation).unwrap();
        let checked: &[u8] = match mode {
            PdfWriterMode::Incremental => {
                include_bytes!("../../../qa/preservation/evidence/flatten-incremental.pdf")
            }
            PdfWriterMode::Compact => {
                include_bytes!("../../../qa/preservation/evidence/flatten-compact.pdf")
            }
            PdfWriterMode::Surgical => unreachable!(),
        };
        assert_eq!(exported.output.as_bytes(), checked);
        assert_eq!(exported.snapshot, receipt.after);
        if mode == PdfWriterMode::Incremental {
            assert!(exported.output.as_bytes().starts_with(TARGETED_FLATTEN));
        }
        let reopened = PdfEngine::default()
            .open_source(exported.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        assert_eq!(fields.len(), 3);
        assert!(!fields.iter().any(|field| field.name == "group.answer"));
        for (name, value, default) in [
            ("group.keep", "Keep editing", "Keep editing"),
            ("accept", "Yes", "Off"),
        ] {
            let field = fields.iter().find(|field| field.name == name).unwrap();
            assert_eq!(field.value.as_deref(), Some(value));
            assert_eq!(field.default_value.as_deref(), Some(default));
            assert_eq!(field.widget_refs.len(), 1);
        }
        assert!(
            binas_pdf::list_annotations(&reopened)
                .unwrap()
                .iter()
                .any(|annotation| annotation.object_number == Some(13))
        );
    }
    let toggle = document
        .plan(
            PdfRevisionRequest {
                checkboxes: vec![CheckboxFieldMutationRequest {
                    field_name: "accept".into(),
                    checked: false,
                    match_index: 0,
                }],
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    document.commit(&toggle, &operation).unwrap();
    assert_eq!(document.snapshot().revision().value(), 2);
    assert_eq!(labels(&document), ["B-1", "A-1"]);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = document.export(mode, &operation).unwrap();
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        assert_eq!(
            fields
                .iter()
                .find(|field| field.name == "accept")
                .unwrap()
                .value
                .as_deref(),
            Some("Off")
        );
        assert!(!fields.iter().any(|field| field.name == "group.answer"));
        assert_eq!(
            fields
                .iter()
                .find(|field| field.name == "group.keep")
                .unwrap()
                .value
                .as_deref(),
            Some("Keep editing")
        );
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn checkbox_states_reorder_and_preservation_agree_across_runtimes() {
    use binas_pdf::CheckboxFieldMutationRequest;
    let mut revision = revision(CHECKBOX_STATES);
    let original = revision.snapshot();
    let operation = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                checkboxes: vec![
                    CheckboxFieldMutationRequest {
                        field_name: "accept".into(),
                        checked: true,
                        match_index: 0,
                    },
                    CheckboxFieldMutationRequest {
                        field_name: "preferences.subscribe".into(),
                        checked: false,
                        match_index: 0,
                    },
                ],
                reorder_pages: Some(vec![1, 0]),
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    assert_eq!(revision.snapshot(), original);
    let effects = plan.checkbox_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    for (field, widget, page, before, after) in [
        (5, 5, 3, b"Off".as_slice(), b"Yes".as_slice()),
        (7, 8, 4, b"Yes".as_slice(), b"Off".as_slice()),
    ] {
        let effect = effects
            .iter()
            .find(|effect| effect.field.number == field)
            .unwrap();
        assert_eq!(effect.widget.number, widget);
        assert_eq!(effect.page.number, page);
        assert_eq!(effect.before_value.as_deref(), Some(before));
        assert_eq!(effect.before_appearance_state, before);
        assert_eq!(effect.after_state, after);
    }
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(revision.snapshot(), original);
    let receipt = revision.commit(&plan, &operation).unwrap();
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(labels(&revision), ["B-1", "A-1"]);
    assert!(matches!(
        revision.commit(&plan, &operation),
        Err(TransactionError::StalePlan { .. })
    ));
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = revision.export(mode, &operation).unwrap();
        assert_eq!(exported.snapshot, receipt.after);
        let independently_checked: &[u8] = match mode {
            PdfWriterMode::Incremental => {
                include_bytes!("../../../qa/preservation/evidence/checkbox-incremental.pdf")
            }
            PdfWriterMode::Compact => {
                include_bytes!("../../../qa/preservation/evidence/checkbox-compact.pdf")
            }
            PdfWriterMode::Surgical => unreachable!("only incremental and compact are exercised"),
        };
        assert_eq!(exported.output.as_bytes(), independently_checked);
        if mode == PdfWriterMode::Incremental {
            assert!(exported.output.as_bytes().starts_with(CHECKBOX_STATES));
        }
        let reopened = PdfEngine::default()
            .open_source(exported.output, OpenOptions::default())
            .unwrap();
        let fields = list_form_fields(&reopened).unwrap();
        for (name, state, default) in [
            ("accept", "Yes", "Off"),
            ("preferences.subscribe", "Off", "Yes"),
        ] {
            let field = fields.iter().find(|field| field.name == name).unwrap();
            assert_eq!(field.value.as_deref(), Some(state));
            assert_eq!(field.default_value.as_deref(), Some(default));
            assert_eq!(field.widget_refs.len(), 1);
        }
        let original_doc = PdfEngine::default()
            .open(CHECKBOX_STATES, OpenOptions::default())
            .unwrap();
        assert_eq!(
            read_document_info(&reopened).unwrap(),
            read_document_info(&original_doc).unwrap()
        );
        let attachments = read_embedded_attachments(&reopened).unwrap();
        let before_attachments = read_embedded_attachments(&original_doc).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            read_embedded_attachment_bytes(&reopened, &attachments[0]).unwrap(),
            read_embedded_attachment_bytes(&original_doc, &before_attachments[0]).unwrap()
        );
    }
    assert!(
        revision
            .plan(
                PdfRevisionRequest {
                    checkboxes: vec![CheckboxFieldMutationRequest {
                        field_name: "missing".into(),
                        checked: true,
                        match_index: 0
                    }],
                    ..Default::default()
                },
                &operation
            )
            .is_err()
    );
    assert_eq!(revision.snapshot(), receipt.after);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn absolute_page_geometry_and_reorder_preserve_current_document_structures() {
    use binas_pdf::{PageTransform, PdfPageTransformRequest};
    let mut revision = revision(STRUCTURED);
    let original = revision.snapshot();
    let operation = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                page_transforms: vec![PdfPageTransformRequest {
                    page_index: 1,
                    transform: PageTransform {
                        rotation_degrees: Some(-90),
                        media_box: Some([-10., -20., 320., 420.]),
                        crop_box: Some([20., 30., 280., 370.]),
                        ..Default::default()
                    },
                }],
                reorder_pages: Some(vec![2, 0, 1]),
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    assert_eq!(revision.snapshot(), original);
    let effects = plan.geometry_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].page.number, 5);
    assert_eq!(effects[0].before.rotation_degrees, 90);
    assert_eq!(effects[0].before.media_box, [0., 0., 300., 400.]);
    assert_eq!(effects[0].before.crop_box, [10., 20., 290., 380.]);
    assert_eq!(effects[0].after.rotation_degrees, 270);
    assert_eq!(effects[0].after.media_box, [-10., -20., 320., 420.]);
    assert_eq!(effects[0].after.crop_box, [20., 30., 280., 370.]);
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(revision.snapshot(), original);
    assert_eq!(revision.page_geometry(1).unwrap().rotation_degrees, 90);
    let receipt = revision.commit(&plan, &operation).unwrap();
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), original.source());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(revision.page_geometry(2).unwrap(), effects[0].after);
    assert_eq!(
        revision
            .page_ids()
            .unwrap()
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [6, 4, 5]
    );
    assert_eq!(labels(&revision), ["C-1", "A-1", "B-1"]);
    assert!(matches!(
        revision.commit(&plan, &operation),
        Err(TransactionError::StalePlan { .. })
    ));
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let result = revision.export(mode, &operation).unwrap();
        assert_eq!(result.snapshot, receipt.after);
        assert_eq!(result.mode, mode);
        let expected: &[u8] = match mode {
            PdfWriterMode::Incremental => {
                include_bytes!("../../../qa/preservation/evidence/geometry-incremental.pdf")
            }
            PdfWriterMode::Compact => {
                include_bytes!("../../../qa/preservation/evidence/geometry-compact.pdf")
            }
            PdfWriterMode::Surgical => unreachable!("the geometry fixture tests supported modes"),
        };
        assert_eq!(result.output.as_bytes(), expected);
        if mode == PdfWriterMode::Incremental {
            assert!(result.output.as_bytes().starts_with(STRUCTURED));
        }
        let reopened = PdfEngine::default()
            .open_source(result.output, OpenOptions::default())
            .unwrap();
        for (index, (rotation, media, crop)) in [
            (270, [0., 0., 500., 600.], [30., 40., 470., 560.]),
            (90, [0., 0., 300., 400.], [10., 20., 290., 380.]),
            (270, [-10., -20., 320., 420.], [20., 30., 280., 370.]),
        ]
        .into_iter()
        .enumerate()
        {
            let geometry = reopened.page_geometry(index).unwrap();
            assert_eq!(geometry.rotation_degrees, rotation);
            assert_eq!(geometry.media_box, media);
            assert_eq!(geometry.crop_box, crop);
            assert_eq!(geometry.bleed_box, None);
            assert_eq!(geometry.trim_box, None);
            assert_eq!(geometry.art_box, None);
        }
        for (index, expected) in [["CHARLIE", "A"], ["ALPHA", "A"], ["BRAVO", "A"]]
            .into_iter()
            .enumerate()
        {
            let text = reopened.extract_page_text_spans(index).unwrap();
            assert!(text.warnings.is_empty());
            assert_eq!(
                text.spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
        assert_eq!(
            read_document_info(&reopened).unwrap().title.as_deref(),
            Some("Résumé 東京")
        );
        assert_eq!(read_named_destinations(&reopened).unwrap()[0].page_index, 2);
        assert_eq!(read_outlines(&reopened).unwrap()[0].title, "Résumé 東京");
        let fields = list_form_fields(&reopened).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "customer");
        assert_eq!(fields[0].value.as_deref(), Some("Zoë"));
        let attachments = read_embedded_attachments(&reopened).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            read_embedded_attachment_bytes(&reopened, &attachments[0]).unwrap(),
            b"Original attachment: preserve these exact bytes.\n"
        );
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn flate_recompression_keeps_the_edited_baseline_and_matches_checked_native_output() {
    let mut revision = revision(FLATE_STORAGE);
    let operation = context();
    let original = revision.snapshot();
    let stale = revision.plan_flate_recompression(&operation).unwrap();
    let edit = revision
        .plan(
            scoped_request(0, "before", "after", PdfSharedEditMode::Reject),
            &operation,
        )
        .unwrap();
    revision.commit(&edit, &operation).unwrap();
    let edited = revision.snapshot();
    assert_eq!(edited.revision().value(), 1);
    assert_eq!(edited.source(), original.source());
    assert!(matches!(
        revision.commit_flate_recompression(&stale, &operation),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(revision.snapshot(), edited);
    let baseline = revision.export(PdfWriterMode::Compact, &operation).unwrap();
    assert_eq!(
        baseline.output.as_bytes(),
        include_bytes!("../../../qa/preservation/evidence/flate-edited-baseline.pdf")
    );
    let plan = revision.plan_flate_recompression(&operation).unwrap();
    assert_eq!(plan.snapshot(), edited);
    assert!(plan.census_complete(), "{:?}", plan.census_gaps());
    assert!(
        plan.effects()
            .iter()
            .any(|effect| effect.stream.number == 7)
    );
    assert!(plan.encoded_bytes_saved() > 0);
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision
            .plan_flate_recompression(&cancelled)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    assert!(matches!(
        revision.commit_flate_recompression(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(revision.snapshot(), edited);
    assert_eq!(
        revision.text().unwrap(),
        ["after", "shared", "untouched", "shared"]
    );
    let receipt = revision
        .commit_flate_recompression(&plan, &operation)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.before, edited);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.revision().value(), 2);
    assert_eq!(receipt.after.source(), original.source());
    assert!(matches!(
        revision.commit_flate_recompression(&plan, &operation),
        Err(TransactionError::StalePlan { .. })
    ));
    let repeated = revision.plan_flate_recompression(&operation).unwrap();
    assert!(repeated.effects().is_empty());
    assert_eq!(repeated.encoded_bytes_saved(), 0);
    assert!(
        revision
            .commit_flate_recompression(&repeated, &operation)
            .unwrap()
            .is_none()
    );
    assert_eq!(revision.snapshot(), receipt.after);
    for mode in [PdfWriterMode::Compact, PdfWriterMode::Incremental] {
        let output = revision.export(mode, &operation).unwrap();
        assert_eq!(output.mode, mode);
        assert_eq!(output.snapshot, receipt.after);
        if mode == PdfWriterMode::Compact {
            assert_eq!(
                output.output.as_bytes(),
                include_bytes!("../../../qa/preservation/evidence/flate-compressed.pdf")
            );
            assert!(output.output.len() < baseline.output.len());
        } else {
            assert!(output.output.as_bytes().starts_with(FLATE_STORAGE));
        }
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        assert_eq!(
            reopened
                .extract_text_spans()
                .unwrap()
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            ["after", "shared", "untouched", "shared"]
        );
        let attachments = read_embedded_attachments(&reopened).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(
            read_embedded_attachment_bytes(&reopened, &attachments[0]).unwrap(),
            b"Original attachment: retain all bytes, including NUL:\0\n"
        );
        assert_eq!(read_named_destinations(&reopened).unwrap()[0].page_index, 1);
        assert_eq!(read_outlines(&reopened).unwrap().len(), 1);
    }
}

// Original one-pixel image at two exact scales, plus a singular placement on an
// unrelated page. Full-image-axis DPI is independently calculable as 72/scale.
fn image_preflight_fixture() -> Vec<u8> {
    let content = [
        b"q 1 0 0 1 20 20 cm /Im Do Q q 2 0 0 3 40 60 cm /Im Do Q".as_slice(),
        b"0 0 0 0 10 10 cm /Im Do",
    ];
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] /Resources << /XObject << /Im 7 0 R >> >> >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".to_vec(),
    ];
    for commands in content {
        objects.push(
            [
                format!("<< /Length {} >>\nstream\n", commands.len()).into_bytes(),
                commands.to_vec(),
                b"\nendstream".to_vec(),
            ]
            .concat(),
        );
    }
    objects.push(b"<< /Length 1 /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray >>\nstream\n@\nendstream".to_vec());
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn image_preflight_preserves_threshold_scope_and_unknown_geometry_across_runtimes() {
    use binas_pdf::preflight::{
        ImagePreflightResult, ImagePreflightScope, ImageResolutionProfile, ImageResolutionStatus,
        inspect_revision_images,
    };
    let operation = context();
    let profile = ImageResolutionProfile::from_json(
        br#"{"version":1,"operator":"minimum_image_axis_dpi","unit":"dpi","minimum":30}"#,
        &operation,
    )
    .unwrap();
    assert_eq!(profile, ImageResolutionProfile::new(30.).unwrap());
    assert!(ImageResolutionProfile::from_json(
        br#"{"version":1,"operator":"minimum_image_axis_dpi","unit":"dpi","minimum":30,"cleanup":true}"#,
        &operation,
    ).is_err());
    let mut revision = revision(&image_preflight_fixture());
    let original = revision.snapshot();
    let before = inspect_revision_images(
        &revision,
        &profile,
        ImagePreflightScope::Page(0),
        &operation,
    )
    .unwrap();
    assert_eq!(before.snapshot(), original);
    assert_eq!(before.result(), ImagePreflightResult::Failed);
    assert_eq!(before.page_indices(), [0]);
    assert_eq!(before.coverage().image_placements, 2);
    assert_eq!(before.coverage().checked_placements, 2);
    assert!(before.coverage().census_complete);
    assert!(
        !before
            .scanned_streams()
            .iter()
            .any(|stream| stream.number == 6)
    );
    let placements = before.findings().collect::<Vec<_>>();
    assert_eq!(placements[0].resource.image_dpi, Some([72., 72.]));
    assert_eq!(placements[0].status, ImageResolutionStatus::Passed);
    assert_eq!(placements[0].below_minimum_axes, Some([false, false]));
    assert_eq!(placements[1].resource.image_dpi, Some([36., 24.]));
    assert_eq!(placements[1].status, ImageResolutionStatus::Failed);
    assert_eq!(placements[1].below_minimum_axes, Some([false, true]));
    assert!(placements.iter().all(|finding| {
        finding.resource.target.unwrap().number == 7 && finding.minimum_dpi == 30.
    }));
    let lower = ImageResolutionProfile::new(24.).unwrap();
    assert_eq!(
        inspect_revision_images(&revision, &lower, ImagePreflightScope::Page(0), &operation)
            .unwrap()
            .result(),
        ImagePreflightResult::Passed
    );
    let document =
        inspect_revision_images(&revision, &lower, ImagePreflightScope::Document, &operation)
            .unwrap();
    assert_eq!(document.result(), ImagePreflightResult::Incomplete);
    assert_eq!(document.coverage().image_placements, 3);
    assert_eq!(document.coverage().checked_placements, 2);
    assert!(document.coverage().census_complete);
    let unchecked = document
        .findings()
        .find(|finding| finding.status == ImageResolutionStatus::NotChecked)
        .unwrap();
    assert_eq!(unchecked.resource.page.number, 4);
    assert_eq!(unchecked.resource.image_dpi, None);
    let reorder = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![1, 0]),
                ..Default::default()
            },
            &operation,
        )
        .unwrap();
    let committed = revision.commit(&reorder, &operation).unwrap();
    let after =
        inspect_revision_images(&revision, &lower, ImagePreflightScope::Page(1), &operation)
            .unwrap();
    assert_eq!(after.snapshot(), committed.after);
    assert_eq!(after.result(), ImagePreflightResult::Passed);
    assert!(
        after.findings().all(|finding| {
            finding.resource.page.number == 3 && finding.resource.page_index == 1
        })
    );
    assert_eq!(before.snapshot(), original);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn committed_resource_census_and_incomplete_standards_agree_across_runtimes() {
    use binas_pdf::{
        PdfResourceKind,
        standards::{ConformanceResult, HumanReviewStatus, StandardsReport, target_registry},
    };

    let mut revision = revision(CONTENT_SEQUENCE);
    let operation = context();
    let before = revision.resource_usage(&operation).unwrap();
    assert!(before.complete(), "{:?}", before.gaps);
    let original = revision.snapshot();
    assert_eq!(before.snapshot(), original);
    let mut request = scoped_request(0, "Ω", "Ψ", PdfSharedEditMode::CloneForPage);
    request.reorder_pages = Some(vec![1, 0]);
    let plan = revision.plan(request, &operation).unwrap();
    let receipt = revision.commit(&plan, &operation).unwrap();
    let after = revision.resource_usage(&operation).unwrap();
    // Completeness covers resource names and encoded character traversal only;
    // it does not establish glyph geometry, rendered appearance or conformance.
    assert!(after.complete(), "{:?}", after.gaps);
    assert_eq!(after.snapshot(), receipt.after);
    assert_eq!(after.snapshot(), revision.snapshot());
    assert_eq!(after.snapshot().source(), original.source());
    assert_eq!(after.snapshot().revision().value(), 1);
    assert_eq!(before.snapshot(), original);
    assert!(after.uses.iter().all(|item| match item.page_index {
        0 => item.page.number == 4,
        1 => item.page.number == 3,
        _ => false,
    }));
    for (page, expected) in [
        (0, [(11, b'A'), (11, b'A'), (11, b'A'), (11, b'B')]),
        (1, [(10, b'B'), (10, b'A'), (11, b'A'), (10, b'B')]),
    ] {
        let selected = revision.page_resource_usage(page, &operation).unwrap();
        assert_eq!(selected.snapshot(), after.snapshot());
        assert!(selected.uses.iter().all(|item| item.page_index == page));
        let painted = selected
            .uses
            .iter()
            .filter_map(|item| {
                item.encoded_text.as_ref().map(|text| {
                    assert_eq!(item.kind, PdfResourceKind::Font);
                    assert_eq!(text.len(), 1);
                    (item.target.unwrap().number, text[0])
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(painted, expected);
    }
    assert!(before.uses.iter().any(|item| {
        item.page.number == 3
            && item.page_index == 0
            && item.invocation_path.is_empty()
            && item.encoded_text.as_deref() == Some(b"A")
    }));
    assert!(after.uses.iter().any(|item| {
        item.page.number == 3
            && item.page_index == 1
            && item.kind == PdfResourceKind::Type3Glyph
            && item.target.is_some_and(|glyph| glyph.number == 14)
    }));

    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision.resource_usage(&cancelled).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    assert_eq!(revision.snapshot(), after.snapshot());
    assert!(target_registry().next().is_some());
    for definition in target_registry() {
        let report = StandardsReport::evaluate(
            after.snapshot(),
            definition.target,
            vec![definition.target],
            &[],
            vec![],
            HumanReviewStatus::NotAssessed,
        )
        .unwrap();
        assert_eq!(report.snapshot(), after.snapshot());
        assert_eq!(report.declared_targets(), [definition.target]);
        assert!(report.findings().is_empty());
        assert_eq!(
            report.evaluated_rules_result(),
            ConformanceResult::Incomplete
        );
        assert_eq!(report.conformance(), ConformanceResult::Incomplete);
        assert!(!report.coverage_gaps().is_empty());
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn unicode_operand_clone_and_reorder_preserve_page_and_form_contexts() {
    let mut revision = revision(CONTENT_SEQUENCE);
    let original = revision.snapshot();
    let ctx = context();
    let request = |replacement: &str| PdfRevisionRequest {
        page_text: vec![PdfPageTextEditRequest {
            page_index: 0,
            edit: SurgicalTextEditRequest {
                old_text: "Ω".into(),
                replacement: replacement.into(),
                match_index: 0,
            },
            shared: PdfSharedEditMode::CloneForPage,
        }],
        reorder_pages: Some(vec![1, 0]),
        ..Default::default()
    };
    // The selected font maps A/B to Ω/Ψ; Ю exists only in the other font.
    assert!(revision.plan(request("Ю"), &ctx).is_err());
    assert_eq!(revision.snapshot(), original);
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision.plan(request("Ψ"), &cancelled).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    assert_eq!(revision.snapshot(), original);
    let plan = revision.plan(request("Ψ"), &ctx).unwrap();
    let fonts = plan.font_effects().collect::<Vec<_>>();
    assert_eq!(fonts.len(), 1);
    assert_eq!(fonts[0].page.number, 3);
    assert_eq!(fonts[0].source_stream.number, 21);
    assert_eq!(fonts[0].font.unwrap().number, 10);
    assert_eq!(fonts[0].cmap.number, 15);
    assert_eq!(fonts[0].encoded_bytes, 1);
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(revision.snapshot(), original);
    let receipt = revision.commit(&plan, &ctx).unwrap();
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), original.source());
    assert_eq!(receipt.after.revision().value(), 1);
    let check = receipt
        .evidence
        .checks()
        .iter()
        .find(|check| check.code.code() == "requested_text")
        .unwrap();
    assert_eq!(check.status, CheckStatus::Passed);
    assert_eq!(
        revision
            .page_ids()
            .unwrap()
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [4, 3]
    );
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let result = revision.export(mode, &ctx).unwrap();
        let reopened = PdfEngine::default()
            .open(result.output.as_bytes(), OpenOptions::default())
            .unwrap();
        for (page, expected) in [(0, ["Ж", "Ж", "Ж", "Ю"]), (1, ["Ψ", "Ω", "Ж", "Ψ"])] {
            let extraction = reopened.extract_page_text_spans(page).unwrap();
            assert!(extraction.warnings.is_empty());
            assert_eq!(
                extraction
                    .spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(extraction.spans.iter().all(|span| span.to_unicode));
        }
    }
}
type NativeCase<'a> = (&'a [u8], &'a [&'a str], &'a [u8], &'a [u8]);

fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 64 * 1024 * 1024,
            max_work_units: 256 * 1024 * 1024,
        }),
        CancellationToken::new(),
    )
}

fn revision(bytes: &[u8]) -> PdfRevision {
    PdfRevision::new(
        PdfEngine::default()
            .open(bytes, OpenOptions::default())
            .unwrap(),
    )
}

fn labels(revision: &PdfRevision) -> Vec<String> {
    revision
        .page_labels()
        .unwrap()
        .into_iter()
        .map(|p| p.label)
        .collect()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn structured_reorder_matches_independently_checked_native_candidates() {
    let cases: &[NativeCase<'_>] = &[
        (
            STRUCTURED,
            &["C-1", "A-1", "B-1"],
            include_bytes!("../../../qa/preservation/evidence/reordered-incremental.pdf"),
            include_bytes!("../../../qa/preservation/evidence/reordered-compact.pdf"),
        ),
        (
            ALPHA,
            &["BB", "Z", "AA"],
            include_bytes!("../../../qa/preservation/evidence/alpha-incremental.pdf"),
            include_bytes!("../../../qa/preservation/evidence/alpha-compact.pdf"),
        ),
    ];
    for (source, expected_labels, incremental, compact) in cases {
        let mut revision = revision(source);
        let original = revision.snapshot();
        assert_eq!(original.revision(), RevisionId::INITIAL);
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    reorder_pages: Some(vec![2, 0, 1]),
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        assert_eq!(revision.snapshot(), original);
        let receipt = revision.commit(&plan, &context).unwrap();
        assert_eq!(receipt.before, original);
        assert_eq!(receipt.after, revision.snapshot());
        assert_eq!(revision.snapshot().revision().value(), 1);
        assert_eq!(revision.snapshot().source(), original.source());
        assert_eq!(
            revision
                .page_ids()
                .unwrap()
                .iter()
                .map(|p| p.number)
                .collect::<Vec<_>>(),
            [6, 4, 5]
        );
        assert_eq!(labels(&revision), *expected_labels);
        assert!(matches!(
            revision.commit(&plan, &context),
            Err(TransactionError::StalePlan { .. })
        ));
        assert_eq!(revision.snapshot().revision().value(), 1);
        for (mode, native) in [
            (PdfWriterMode::Incremental, *incremental),
            (PdfWriterMode::Compact, *compact),
        ] {
            let exported = revision.export(mode, &context).unwrap();
            assert_eq!(exported.snapshot, revision.snapshot());
            assert_eq!(
                exported.output.as_bytes(),
                native,
                "frozen native/browser fixture parity"
            );
            let reopened = PdfEngine::default()
                .open_source(exported.output, OpenOptions::default())
                .unwrap();
            assert_eq!(
                read_page_labels(&reopened)
                    .unwrap()
                    .iter()
                    .map(|p| p.label.as_str())
                    .collect::<Vec<_>>(),
                *expected_labels
            );
            assert_eq!(
                read_document_info(&reopened).unwrap().title.as_deref(),
                Some("Résumé 東京")
            );
            let outlines = read_outlines(&reopened).unwrap();
            assert_eq!(outlines.len(), 1);
            assert_eq!(outlines[0].title, "Résumé 東京");
            let destinations = read_named_destinations(&reopened).unwrap();
            assert_eq!(destinations.len(), 1);
            assert_eq!(destinations[0].name, "landing");
            assert_eq!(destinations[0].page_index, 2);
            let fields = list_form_fields(&reopened).unwrap();
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].name, "customer");
            assert_eq!(fields[0].value.as_deref(), Some("Zoë"));
            let attachments = read_embedded_attachments(&reopened).unwrap();
            assert_eq!(attachments.len(), 1);
            assert_eq!(
                read_embedded_attachment_bytes(&reopened, &attachments[0]).unwrap(),
                b"Original attachment: preserve these exact bytes.\n"
            );
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn failed_or_precancelled_work_keeps_the_retained_revision_usable() {
    let mut revision = revision(STRUCTURED);
    let original = revision.snapshot();
    let original_pages = revision.page_ids().unwrap();
    let fresh = context();
    assert!(
        revision
            .plan(
                PdfRevisionRequest {
                    reorder_pages: Some(vec![0, 0, 2]),
                    ..Default::default()
                },
                &fresh
            )
            .is_err()
    );
    let plan = revision
        .plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![2, 0, 1]),
                ..Default::default()
            },
            &fresh,
        )
        .unwrap();
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision
            .plan(
                PdfRevisionRequest {
                    reorder_pages: Some(vec![2, 0, 1]),
                    ..Default::default()
                },
                &cancelled
            )
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    assert!(matches!(
        revision.commit(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(
        revision
            .export(PdfWriterMode::Compact, &cancelled)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    assert_eq!(revision.snapshot(), original);
    assert_eq!(revision.page_ids().unwrap(), original_pages);
    revision.commit(&plan, &fresh).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 1);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn unicode_label_prefix_survives_plan_commit_and_both_writers() {
    let objects: [&[u8]; 5] = [
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 5 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 100 100] /Resources << >> >>",
        b"<< /Type /Page /Parent 2 0 R >>",
        b"<< /Type /Page /Parent 2 0 R >>",
        b"<< /Nums [0 << /S /D /P <FEFF67714EAC> /St 1 >>] >>",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let mut revision = revision(&bytes);
    assert_eq!(labels(&revision), ["東京1", "東京2"]);
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
    revision.commit(&plan, &context).unwrap();
    assert_eq!(labels(&revision), ["東京2", "東京1"]);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &context).unwrap();
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        assert_eq!(
            read_page_labels(&reopened)
                .unwrap()
                .iter()
                .map(|p| p.label.as_str())
                .collect::<Vec<_>>(),
            ["東京2", "東京1"]
        );
    }
}

// Original raw-token control: no font selection or font-fidelity claim.
fn shared_raw_fixture() -> Vec<u8> {
    let objects: [&[u8]; 6] = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 100 100] /Resources << >> >>",
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>",
        b"<< /Length 16 >>\nstream\nBT (hello) Tj ET\nendstream",
        b"<< /Title (Shared scope control) >>",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R /Info 6 0 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    bytes
}

fn scoped_request(
    page_index: usize,
    old: &str,
    replacement: &str,
    shared: binas_pdf::PdfSharedEditMode,
) -> PdfRevisionRequest {
    PdfRevisionRequest {
        page_text: vec![binas_pdf::PdfPageTextEditRequest {
            page_index,
            shared,
            edit: binas_pdf::SurgicalTextEditRequest {
                old_text: old.into(),
                replacement: replacement.into(),
                match_index: 0,
            },
        }],
        ..Default::default()
    }
}

fn check_scoped_exports(revision: &PdfRevision, source: &[u8], expected: [&str; 2]) {
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let result = revision.export(mode, &context()).unwrap();
        assert_eq!(result.mode, mode);
        assert_eq!(result.snapshot, revision.snapshot());
        if mode == PdfWriterMode::Incremental {
            assert!(result.output.as_bytes().starts_with(source));
        }
        let document = PdfEngine::default()
            .open_source(result.output, OpenOptions::default())
            .unwrap();
        assert_eq!(
            read_document_info(&document).unwrap().title.as_deref(),
            Some("Shared scope control")
        );
        for (page, text) in expected.iter().enumerate() {
            assert_eq!(
                document
                    .extract_page_text_spans(page)
                    .unwrap()
                    .spans
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect::<Vec<_>>(),
                [*text]
            );
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn scoped_shared_stream_modes_survive_two_commits_and_both_writers() {
    use binas_pdf::PdfSharedEditMode::{AllPageOwners, CloneForPage, Reject};
    let source = shared_raw_fixture();
    for shared in [CloneForPage, AllPageOwners] {
        let mut document = revision(&source);
        let original = document.snapshot();
        let pages = document.page_ids().unwrap();
        assert!(
            document
                .plan(scoped_request(0, "hello", "alpha", Reject), &context())
                .is_err()
        );
        let plan = document
            .plan(scoped_request(0, "hello", "alpha", shared), &context())
            .unwrap();
        let effects = plan.stream_effects().collect::<Vec<_>>();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].source_stream.number, 5);
        assert_eq!(effects[0].shared, shared);
        if shared == CloneForPage {
            assert_eq!(effects[0].output_stream.number, 7);
            assert_eq!(effects[0].pages, [pages[0]]);
        } else {
            assert_eq!(effects[0].output_stream.number, 5);
            assert_eq!(effects[0].pages, pages);
        }
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert!(matches!(
            document.commit(&plan, &cancelled),
            Err(TransactionError::Context(ContextError::Cancelled))
        ));
        assert_eq!(document.snapshot(), original);
        assert_eq!(document.text().unwrap(), ["hello", "hello"]);
        document.commit(&plan, &context()).unwrap();
        let other = if shared == CloneForPage {
            "hello"
        } else {
            "alpha"
        };
        check_scoped_exports(&document, &source, ["alpha", other]);
        assert!(matches!(
            document.commit(&plan, &context()),
            Err(TransactionError::StalePlan { .. })
        ));
        let later_mode = if shared == CloneForPage {
            Reject
        } else {
            AllPageOwners
        };
        let later = document
            .plan(scoped_request(1, other, "bravo", later_mode), &context())
            .unwrap();
        let effects = later.stream_effects().collect::<Vec<_>>();
        assert_eq!(effects[0].source_stream, effects[0].output_stream);
        assert_eq!(
            effects[0].pages,
            if shared == CloneForPage {
                vec![pages[1]]
            } else {
                pages.clone()
            }
        );
        document.commit(&later, &context()).unwrap();
        assert_eq!(document.snapshot().revision().value(), 2);
        assert_eq!(document.snapshot().source(), original.source());
        assert_eq!(document.page_ids().unwrap(), pages);
        check_scoped_exports(
            &document,
            &source,
            [
                if shared == CloneForPage {
                    "alpha"
                } else {
                    "bravo"
                },
                "bravo",
            ],
        );
    }
}
