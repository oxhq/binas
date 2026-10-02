//! Logical page deletion with explicit local destination remapping.
//!
//! Original detached objects and incremental history remain stored. This
//! example does not perform secure erasure or redaction.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    OpenOptions, PdfDocument, PdfEngine, PdfExcludedPageTargetPolicy, PdfPageDeleteRequest,
    PdfReferencePathStep, PdfReferenceTarget, PdfRevision, PdfRevisionRequest,
    PdfTextFieldFillRequest, PdfWriterMode, list_form_fields,
};
use std::{error::Error, path::Path};

const FILLED_NAME: &str = r"Café (A) \ B";

const fn target(number: u32) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number,
        generation: 0,
    }
}

fn check_fields(document: &PdfDocument, name_value: &str) -> Result<(), Box<dyn Error>> {
    let fields = list_form_fields(document)?;
    assert_eq!(fields.len(), 3);
    assert!(fields.iter().any(|field| field.name == "group"));
    for (name, value, default, object, widget) in [
        ("group.name", name_value, "Default name", 8, 9),
        ("group.keep", "Old", "Old", 11, 11),
    ] {
        let field = fields.iter().find(|field| field.name == name).unwrap();
        assert_eq!(field.object_number, Some(object));
        assert_eq!(field.value.as_deref(), Some(value));
        assert_eq!(field.default_value.as_deref(), Some(default));
        assert_eq!(
            field
                .widget_refs
                .iter()
                .map(|widget| (widget.object_number, widget.object_generation))
                .collect::<Vec<_>>(),
            [(widget, 0)]
        );
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    // Modified for external consumer evidence; fixtures remain frozen.
    let root = std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"));
    let source = std::fs::read(root.join("fixtures/text-field-fill.pdf"))?;
    let mut revision =
        PdfRevision::new(PdfEngine::default().open(&source, OpenOptions::default())?);
    let original = revision.snapshot();
    let context = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 64 * 1024 * 1024,
            max_work_units: 256 * 1024 * 1024,
        }),
        CancellationToken::new(),
    );
    let plan = revision.plan(
        PdfRevisionRequest {
            delete_page: Some(PdfPageDeleteRequest {
                page_index: 1,
                excluded_targets: PdfExcludedPageTargetPolicy::RemapToPage { page_index: 0 },
            }),
            ..Default::default()
        },
        &context,
    )?;
    assert_eq!(revision.snapshot(), original);
    let effect = plan.delete_effect().unwrap();
    assert_eq!(effect.page, target(4));
    assert_eq!(effect.page_index, 1);
    assert_eq!(effect.remaining_pages, [target(3)]);
    assert_eq!(effect.removed_widgets, [target(10), target(12), target(13)]);
    assert_eq!(effect.unregistered_fields, [target(12), target(13)]);
    assert_eq!(effect.remapped_destinations.len(), 3);
    for (object, path) in [
        (
            18,
            vec![
                PdfReferencePathStep::Key(b"Dest".to_vec()),
                PdfReferencePathStep::Index(0),
            ],
        ),
        (
            26,
            vec![
                PdfReferencePathStep::Key(b"Dest".to_vec()),
                PdfReferencePathStep::Index(0),
            ],
        ),
        (
            27,
            vec![
                PdfReferencePathStep::Key(b"Dests".to_vec()),
                PdfReferencePathStep::Key(b"Names".to_vec()),
                PdfReferencePathStep::Index(1),
                PdfReferencePathStep::Index(0),
            ],
        ),
    ] {
        let remap = effect
            .remapped_destinations
            .iter()
            .find(|remap| remap.object == target(object))
            .unwrap();
        assert_eq!(remap.path, path);
        assert_eq!(remap.from, target(4));
        assert_eq!(remap.to, target(3));
    }
    let receipt = revision.commit(&plan, &context)?;
    let deleted = revision.snapshot();
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after, deleted);
    assert_eq!(deleted.source(), original.source());
    assert_eq!(deleted.revision().value(), 1);
    assert_eq!(revision.page_ids()?, [target(3)]);
    assert_eq!(
        revision
            .page_labels()?
            .iter()
            .map(|label| label.label.as_str())
            .collect::<Vec<_>>(),
        ["A-1"]
    );

    // Check both deletion outputs before running the independent next edit.
    // The oracle receives these exact deletion-only bytes.
    let incremental = revision.export(PdfWriterMode::Incremental, &context)?;
    let compact = revision.export(PdfWriterMode::Compact, &context)?;
    assert!(incremental.output.as_bytes().starts_with(&source));
    for result in [&incremental, &compact] {
        assert_eq!(result.snapshot, deleted);
        let reopened =
            PdfEngine::default().open_source(result.output.clone(), OpenOptions::default())?;
        check_fields(&reopened, "Old")?;
        assert_eq!(PdfRevision::new(reopened).page_ids()?, [target(3)]);
    }

    // The retained widget remains editable, with its default and unrelated
    // field intact. The excluded widget must not receive a new appearance.
    let fill = revision.plan(
        PdfRevisionRequest {
            text_fields: vec![PdfTextFieldFillRequest {
                field_name: "group.name".into(),
                value: FILLED_NAME.into(),
                match_index: 0,
            }],
            ..Default::default()
        },
        &context,
    )?;
    assert_eq!(revision.snapshot(), deleted);
    let fill_effects = fill.text_field_effects().collect::<Vec<_>>();
    assert_eq!(fill_effects.len(), 1);
    let fill_effect = fill_effects[0];
    assert_eq!(fill_effect.field, target(8));
    assert_eq!(fill_effect.before_value, "Old");
    assert_eq!(fill_effect.after_value, FILLED_NAME);
    assert_eq!(fill_effect.widgets.len(), 1);
    assert_eq!(fill_effect.widgets[0].widget, target(9));
    assert_eq!(fill_effect.widgets[0].page, target(3));
    assert_eq!(fill_effect.widgets[0].source_appearance, target(14));
    assert!(fill_effect.widgets[0].output_appearance.number > 34);
    let fill_receipt = revision.commit(&fill, &context)?;
    assert_eq!(fill_receipt.before, deleted);
    assert_eq!(fill_receipt.after, revision.snapshot());
    assert_eq!(fill_receipt.after.revision().value(), 2);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let result = revision.export(mode, &context)?;
        assert_eq!(result.snapshot, revision.snapshot());
        let reopened = PdfEngine::default().open_source(result.output, OpenOptions::default())?;
        check_fields(&reopened, FILLED_NAME)?;
        assert_eq!(PdfRevision::new(reopened).page_ids()?, [target(3)]);
    }

    let evidence = std::env::var_os("BINAS_PRESERVATION_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("evidence"));
    std::fs::create_dir_all(&evidence)?;
    for (mode, result) in [("incremental", incremental), ("compact", compact)] {
        let path = evidence.join(format!("delete-{mode}.pdf"));
        std::fs::write(&path, result.output.as_bytes())?;
        println!(
            "{}: {} bytes, output {:?}",
            path.display(),
            result.output.len(),
            result.output.id()
        );
    }
    Ok(())
}
