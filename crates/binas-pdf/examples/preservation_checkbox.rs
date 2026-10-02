//! Original frozen checkbox/reorder outputs for the independent checkbox oracle.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    CheckboxFieldMutationRequest, OpenOptions, PdfEngine, PdfRevision, PdfRevisionRequest,
    PdfWriterMode, list_form_fields,
};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    // Modified for external consumer evidence; fixtures remain frozen.
    let root = std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"));
    let source = std::fs::read(root.join("fixtures/checkbox-states.pdf"))?;
    let mut revision =
        PdfRevision::new(PdfEngine::default().open_owned(source, OpenOptions::default())?);
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
        &context,
    )?;
    assert_eq!(revision.snapshot(), original);
    let effects = plan.checkbox_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    for (field, widget, page, checked, before, after) in [
        (5, 5, 3, true, b"Off".as_slice(), b"Yes".as_slice()),
        (7, 8, 4, false, b"Yes".as_slice(), b"Off".as_slice()),
    ] {
        let effect = effects
            .iter()
            .find(|effect| effect.field.number == field)
            .unwrap();
        assert_eq!(effect.field.generation, 0);
        assert_eq!(
            (effect.widget.number, effect.widget.generation),
            (widget, 0)
        );
        assert_eq!((effect.page.number, effect.page.generation), (page, 0));
        assert_eq!(effect.checked, checked);
        assert_eq!(effect.before_value.as_deref(), Some(before));
        assert_eq!(effect.before_appearance_state, before);
        assert_eq!(effect.after_state, after);
    }
    let receipt = revision.commit(&plan, &context)?;
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), original.source());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(
        revision
            .page_ids()?
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [4, 3]
    );
    assert_eq!(
        revision
            .page_labels()?
            .iter()
            .map(|label| label.label.as_str())
            .collect::<Vec<_>>(),
        ["B-1", "A-1"]
    );
    // Obtain both final checked exports and reopen their public field inventory
    // before publishing either required evidence artifact.
    let incremental = revision.export(PdfWriterMode::Incremental, &context)?;
    let compact = revision.export(PdfWriterMode::Compact, &context)?;
    for result in [&incremental, &compact] {
        assert_eq!(result.snapshot, revision.snapshot());
        let reopened =
            PdfEngine::default().open_source(result.output.clone(), OpenOptions::default())?;
        let fields = list_form_fields(&reopened)?;
        for (name, value, default_value, field, widget) in [
            ("accept", "Yes", "Off", 5, 5),
            ("preferences.subscribe", "Off", "Yes", 7, 8),
        ] {
            let actual = fields.iter().find(|actual| actual.name == name).unwrap();
            assert_eq!(actual.object_number, Some(field));
            assert_eq!(actual.value.as_deref(), Some(value));
            assert_eq!(actual.default_value.as_deref(), Some(default_value));
            assert_eq!(
                actual
                    .widget_refs
                    .iter()
                    .map(|widget| widget.object_number)
                    .collect::<Vec<_>>(),
                [widget]
            );
        }
    }
    let evidence = std::env::var_os("BINAS_PRESERVATION_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("evidence"));
    std::fs::create_dir_all(&evidence)?;
    for (mode, result) in [("incremental", incremental), ("compact", compact)] {
        let path = evidence.join(format!("checkbox-{mode}.pdf"));
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
