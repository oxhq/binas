//! Original targeted form-flatten/reorder outputs for the independent oracle.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    OpenOptions, PdfEngine, PdfFieldFlattenRequest, PdfRevision, PdfRevisionRequest, PdfWriterMode,
    list_form_fields,
};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    // Modified for external consumer evidence; fixtures remain frozen.
    let root = std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"));
    let source = std::fs::read(root.join("fixtures/targeted-flatten.pdf"))?;
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
            flatten_fields: vec![PdfFieldFlattenRequest {
                field_name: "group.answer".into(),
                match_index: 0,
            }],
            reorder_pages: Some(vec![1, 0]),
            ..Default::default()
        },
        &context,
    )?;
    assert_eq!(revision.snapshot(), original);
    let effects = plan.flatten_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(
        (effects[0].field.number, effects[0].field.generation),
        (8, 0)
    );
    assert_eq!(effects[0].widgets.len(), 2);
    for (widget, page, matrix) in [
        (9, 3, [2., 0., 0., 1., 16., 143.]),
        (10, 4, [1., 0., 0., 2., 58., 116.]),
    ] {
        let effect = effects[0]
            .widgets
            .iter()
            .find(|effect| effect.widget.number == widget)
            .unwrap();
        assert_eq!(effect.widget.generation, 0);
        assert_eq!((effect.page.number, effect.page.generation), (page, 0));
        assert_eq!(
            (effect.appearance.number, effect.appearance.generation),
            (14, 0)
        );
        assert_eq!(effect.resource_name, b"BinasFlatten1");
        assert_eq!(effect.placement_matrix, matrix);
        assert_eq!(effect.annotation_flags, 4);
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
    // Obtain both checked exports and verify their active field inventories
    // before publishing either required evidence artifact.
    let incremental = revision.export(PdfWriterMode::Incremental, &context)?;
    let compact = revision.export(PdfWriterMode::Compact, &context)?;
    for result in [&incremental, &compact] {
        assert_eq!(result.snapshot, revision.snapshot());
        let reopened =
            PdfEngine::default().open_source(result.output.clone(), OpenOptions::default())?;
        let fields = list_form_fields(&reopened)?;
        assert_eq!(fields.len(), 3);
        assert!(!fields.iter().any(|field| field.name == "group.answer"));
        for (name, value, default_value, object, widget) in [
            ("group.keep", "Keep editing", "Keep editing", 11, 11),
            ("accept", "Yes", "Off", 12, 12),
        ] {
            let field = fields.iter().find(|field| field.name == name).unwrap();
            assert_eq!(field.object_number, Some(object));
            assert_eq!(field.value.as_deref(), Some(value));
            assert_eq!(field.default_value.as_deref(), Some(default_value));
            assert_eq!(
                field
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
        let path = evidence.join(format!("flatten-{mode}.pdf"));
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
