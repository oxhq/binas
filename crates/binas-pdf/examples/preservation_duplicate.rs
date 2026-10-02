//! Original page duplication outputs for independent object, field and RGB checks.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    OpenOptions, PdfDuplicateFieldName, PdfEngine, PdfPageDuplicateRequest, PdfRevision,
    PdfRevisionRequest, PdfWriterMode, list_form_fields,
};
use std::{collections::BTreeSet, error::Error, path::Path};

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
            duplicate_page: Some(PdfPageDuplicateRequest {
                page_index: 0,
                fields: [
                    ("group.name", "group.name_copy"),
                    ("group.keep", "group.keep_copy"),
                ]
                .into_iter()
                .map(|(field_name, new_field_name)| PdfDuplicateFieldName {
                    field_name: field_name.into(),
                    new_field_name: new_field_name.into(),
                })
                .collect(),
            }),
            ..Default::default()
        },
        &context,
    )?;
    assert_eq!(revision.snapshot(), original);
    let effect = plan.duplicate_effect().unwrap();
    assert_eq!(
        (effect.source_page.number, effect.source_page.generation),
        (3, 0)
    );
    assert_eq!((effect.page_index, effect.inserted_index), (0, 1));
    assert_eq!(effect.fields.len(), 2);
    assert_eq!(effect.annotations.len(), 3);
    let duplicate_page = effect.page.number;
    let mut clones = BTreeSet::new();
    assert!(duplicate_page > 34);
    assert_eq!(effect.page.generation, 0);
    assert!(clones.insert(duplicate_page));
    for (annotation, source_annotation) in effect.annotations.iter().zip([9, 11, 18]) {
        assert_eq!(
            (
                annotation.source_annotation.number,
                annotation.source_annotation.generation
            ),
            (source_annotation, 0)
        );
        assert!(annotation.annotation.number > 34);
        assert_eq!(annotation.annotation.generation, 0);
        assert!(clones.insert(annotation.annotation.number));
    }
    for (field, (source_field, name, new_name, widget_index)) in effect.fields.iter().zip([
        (8, "group.name", "group.name_copy", 0),
        (11, "group.keep", "group.keep_copy", 1),
    ]) {
        assert_eq!(
            (field.source_field.number, field.source_field.generation),
            (source_field, 0)
        );
        assert_eq!(field.field_name, name);
        assert_eq!(field.new_field_name, new_name);
        assert!(field.field.number > 34);
        assert_eq!(field.field.generation, 0);
        assert_eq!(field.widgets.len(), 1);
        let mapping = &field.widgets[0];
        let annotation = &effect.annotations[widget_index];
        assert_eq!(mapping.source_annotation, annotation.source_annotation);
        assert_eq!(mapping.annotation, annotation.annotation);
        if source_field == 8 {
            assert!(clones.insert(field.field.number));
        } else {
            assert_eq!(field.field, annotation.annotation);
        }
    }
    assert_eq!(clones.len(), 5);
    let copied_name = effect.fields[0].field.number;
    let copied_keep = effect.fields[1].field.number;
    let copied_widget = effect.annotations[0].annotation.number;
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
        [3, duplicate_page, 4]
    );
    assert_eq!(
        revision
            .page_labels()?
            .iter()
            .map(|label| label.label.as_str())
            .collect::<Vec<_>>(),
        ["A-1", "A-1", "B-1"]
    );
    // Both writers must preserve the original fields and create independently
    // registered copied fields before either candidate enters the evidence set.
    let incremental = revision.export(PdfWriterMode::Incremental, &context)?;
    let compact = revision.export(PdfWriterMode::Compact, &context)?;
    for result in [&incremental, &compact] {
        assert_eq!(result.snapshot, revision.snapshot());
        let reopened =
            PdfEngine::default().open_source(result.output.clone(), OpenOptions::default())?;
        let fields = list_form_fields(&reopened)?;
        assert_eq!(fields.len(), 7);
        for (name, value, default, object, widgets) in [
            ("group.name", "Old", "Default name", 8, vec![9, 10]),
            ("group.keep", "Old", "Old", 11, vec![11]),
            ("code", "Start", "Default code", 12, vec![12]),
            ("accept", "Yes", "Off", 13, vec![13]),
            (
                "group.name_copy",
                "Old",
                "Default name",
                copied_name,
                vec![copied_widget],
            ),
            (
                "group.keep_copy",
                "Old",
                "Old",
                copied_keep,
                vec![copied_keep],
            ),
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
        let path = evidence.join(format!("duplicate-{mode}.pdf"));
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
