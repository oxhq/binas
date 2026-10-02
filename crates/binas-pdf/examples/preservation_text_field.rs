//! Original text-field fill/reorder outputs for independent object and RGB checks.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    OpenOptions, PdfEngine, PdfRevision, PdfRevisionRequest, PdfTextFieldFillRequest,
    PdfWriterMode, list_form_fields,
};
use std::{collections::BTreeSet, error::Error, path::Path};

const NAME: &str = r"Café (A) \ B";
const CODE: &str = "Ready (2)";

fn main() -> Result<(), Box<dyn Error>> {
    // Modified for external consumer evidence; fixtures remain frozen.
    let root = std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"));
    let source = std::fs::read(root.join("fixtures/text-field-fill.pdf"))?;
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
            text_fields: [("group.name", NAME), ("code", CODE)]
                .into_iter()
                .map(|(field_name, value)| PdfTextFieldFillRequest {
                    field_name: field_name.into(),
                    value: value.into(),
                    match_index: 0,
                })
                .collect(),
            reorder_pages: Some(vec![1, 0]),
            ..Default::default()
        },
        &context,
    )?;
    assert_eq!(revision.snapshot(), original);
    let effects = plan.text_field_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    let mut clones = BTreeSet::new();
    for (field, before, after, appearances, expected_widgets, ink, clip) in [
        (
            8,
            "Old",
            NAME,
            14,
            vec![(9, 3), (10, 4)],
            [5.5, 7.5, 78.5, 19.],
            [2., 2., 178., 30.],
        ),
        (
            12,
            "Start",
            CODE,
            15,
            vec![(12, 4)],
            [5.5, 7.5, 60.5, 19.],
            [2., 2., 148., 30.],
        ),
    ] {
        let effect = effects
            .iter()
            .find(|effect| effect.field.number == field)
            .unwrap();
        assert_eq!(effect.field.generation, 0);
        assert_eq!(effect.before_value, before);
        assert_eq!(effect.after_value, after);
        assert_eq!(effect.widgets.len(), expected_widgets.len());
        for (widget, page) in expected_widgets {
            let effect = effect
                .widgets
                .iter()
                .find(|effect| effect.widget.number == widget)
                .unwrap();
            assert_eq!(effect.widget.generation, 0);
            assert_eq!((effect.page.number, effect.page.generation), (page, 0));
            assert_eq!(
                (
                    effect.source_appearance.number,
                    effect.source_appearance.generation
                ),
                (appearances, 0)
            );
            assert!(effect.output_appearance.number > 34);
            assert_eq!(effect.output_appearance.generation, 0);
            assert!(clones.insert(effect.output_appearance.number));
            assert_eq!(
                effect.font.map(|font| (font.number, font.generation)),
                Some((19, 0))
            );
            assert_eq!((effect.cmap.number, effect.cmap.generation), (22, 0));
            assert_eq!(effect.font_size, 10.);
            assert_eq!(effect.original_advance, before.len() as f64 * 6.);
            assert_eq!(
                effect.replacement_advance,
                after.chars().count() as f64 * 6.
            );
            assert_eq!(effect.ink_bounds, ink);
            assert_eq!(effect.clip_bounds, clip);
            assert_eq!(effect.available_width, clip[2] - 6.);
            assert_eq!(effect.encoded_bytes, after.chars().count());
        }
    }
    assert_eq!(clones.len(), 3);
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
    // Publish only after both checked writers reopen with all original fields,
    // requested values and untouched defaults/unselected values.
    let incremental = revision.export(PdfWriterMode::Incremental, &context)?;
    let compact = revision.export(PdfWriterMode::Compact, &context)?;
    for result in [&incremental, &compact] {
        assert_eq!(result.snapshot, revision.snapshot());
        let reopened =
            PdfEngine::default().open_source(result.output.clone(), OpenOptions::default())?;
        let fields = list_form_fields(&reopened)?;
        assert_eq!(fields.len(), 5);
        for (name, value, default, object, widgets) in [
            ("group.name", NAME, "Default name", 8, vec![9, 10]),
            ("group.keep", "Old", "Old", 11, vec![11]),
            ("code", CODE, "Default code", 12, vec![12]),
            ("accept", "Yes", "Off", 13, vec![13]),
        ] {
            let field = fields.iter().find(|field| field.name == name).unwrap();
            assert_eq!(field.object_number, Some(object));
            assert_eq!(field.value.as_deref(), Some(value));
            assert_eq!(field.default_value.as_deref(), Some(default));
            assert_eq!(
                field
                    .widget_refs
                    .iter()
                    .map(|widget| widget.object_number)
                    .collect::<Vec<_>>(),
                widgets
            );
        }
    }
    let evidence = std::env::var_os("BINAS_PRESERVATION_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("evidence"));
    std::fs::create_dir_all(&evidence)?;
    for (mode, result) in [("incremental", incremental), ("compact", compact)] {
        let path = evidence.join(format!("text-field-{mode}.pdf"));
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
