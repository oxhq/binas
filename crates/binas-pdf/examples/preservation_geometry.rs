//! Original frozen geometry/reorder outputs for the independent geometry oracle.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    OpenOptions, PageTransform, PdfEngine, PdfPageTransformRequest, PdfRevision,
    PdfRevisionRequest, PdfWriterMode,
};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    // Modified for external consumer evidence; fixtures remain frozen.
    let root = std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"));
    let source = std::fs::read(root.join("fixtures/reorder-structures.pdf"))?;
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
        &context,
    )?;
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
    let expected_geometry = effects[0].after;
    let receipt = revision.commit(&plan, &context)?;
    assert_eq!(receipt.before, original);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), original.source());
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(revision.page_geometry(2)?, expected_geometry);
    assert_eq!(
        revision
            .page_ids()?
            .iter()
            .map(|page| page.number)
            .collect::<Vec<_>>(),
        [6, 4, 5]
    );
    assert_eq!(revision.page_geometry(0)?.media_box, [0., 0., 500., 600.]);
    assert_eq!(revision.page_geometry(0)?.rotation_degrees, 270);
    assert_eq!(revision.page_geometry(1)?.media_box, [0., 0., 300., 400.]);
    assert_eq!(revision.page_geometry(1)?.rotation_degrees, 90);
    // Obtain both fully checked exports before publishing either evidence file.
    let incremental = revision.export(PdfWriterMode::Incremental, &context)?;
    let compact = revision.export(PdfWriterMode::Compact, &context)?;
    let evidence = std::env::var_os("BINAS_PRESERVATION_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("evidence"));
    std::fs::create_dir_all(&evidence)?;
    for (mode, result) in [("incremental", incremental), ("compact", compact)] {
        let path = evidence.join(format!("geometry-{mode}.pdf"));
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
