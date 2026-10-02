//! Exact image predictor outputs after a required edit, independently checked.
//! Stream payload savings and final serialized sizes are distinct measurements.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    ImageColorSpace, OpenOptions, PdfEngine, PdfLosslessImageSkipReason, PdfPageTextEditRequest,
    PdfRevision, PdfRevisionRequest, PdfSharedEditMode, PdfWriterMode, SurgicalTextEditRequest,
};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    // Modified for external consumer evidence; fixtures remain frozen.
    let root = std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"));
    let source = std::fs::read(root.join("fixtures/lossless-images.pdf"))?;
    let mut revision =
        PdfRevision::new(PdfEngine::default().open_owned(source, OpenOptions::default())?);
    let context = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 64 * 1024 * 1024,
            max_work_units: 256 * 1024 * 1024,
        }),
        CancellationToken::new(),
    );
    let edit = revision.plan(
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
        &context,
    )?;
    revision.commit(&edit, &context)?;
    assert_eq!(revision.snapshot().revision().value(), 1);
    assert_eq!(revision.text()?, ["after", "untouched"]);
    let baseline = revision.export(PdfWriterMode::Compact, &context)?;
    let edited = revision.snapshot();
    let plan = revision.plan_lossless_images(&context)?;
    assert_eq!(revision.snapshot(), edited);
    assert_eq!(plan.snapshot(), edited);
    assert!(plan.census_complete(), "{:?}", plan.census_gaps());
    assert_eq!(plan.effects().len(), 2);
    for (number, color_space, sample_bytes, encoded_before) in [
        (7, ImageColorSpace::DeviceRgb, 18_432, 18_448),
        (8, ImageColorSpace::DeviceGray, 6_144, 583),
    ] {
        let effect = plan
            .effects()
            .iter()
            .find(|effect| effect.image.number == number)
            .unwrap();
        assert_eq!(effect.image.generation, 0);
        assert_eq!((effect.width, effect.height), (96, 64));
        assert_eq!(effect.color_space, color_space);
        assert_eq!((effect.source_predictor, effect.output_predictor), (1, 12));
        assert_eq!(effect.decoded_bytes, sample_bytes);
        assert_eq!(effect.encoded_before, encoded_before);
        assert!(effect.encoded_after < effect.encoded_before);
        println!("image sample/storage effect: {effect:?}");
    }
    for (number, reason) in [
        (9, PdfLosslessImageSkipReason::UnsupportedFilter),
        (13, PdfLosslessImageSkipReason::NotSmaller),
    ] {
        let skip = plan
            .skips()
            .iter()
            .find(|skip| skip.image.number == number)
            .unwrap();
        assert_eq!(skip.image.generation, 0);
        assert_eq!(skip.reason, reason);
    }
    let encoded_bytes_saved = plan.encoded_bytes_saved();
    assert_eq!(
        encoded_bytes_saved,
        plan.effects()
            .iter()
            .map(|effect| (effect.encoded_before - effect.encoded_after) as u64)
            .sum::<u64>()
    );
    let receipt = revision
        .commit_lossless_images(&plan, &context)?
        .ok_or("fixture must produce a nonempty lossless image commit")?;
    assert_eq!(receipt.before, edited);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), edited.source());
    assert_eq!(receipt.after.revision().value(), 2);
    assert_eq!(revision.text()?, ["after", "untouched"]);
    let incremental = revision.export(PdfWriterMode::Incremental, &context)?;
    let compact = revision.export(PdfWriterMode::Compact, &context)?;
    assert!(compact.output.len() < baseline.output.len());
    let optimized = revision.snapshot();
    let repeat = revision.plan_lossless_images(&context)?;
    assert_eq!(repeat.snapshot(), optimized);
    assert!(repeat.effects().is_empty());
    assert_eq!(repeat.encoded_bytes_saved(), 0);
    for number in [7, 8, 13] {
        assert_eq!(
            repeat
                .skips()
                .iter()
                .find(|skip| skip.image.number == number)
                .unwrap()
                .reason,
            PdfLosslessImageSkipReason::NotSmaller
        );
    }
    assert!(
        revision
            .commit_lossless_images(&repeat, &context)?
            .is_none()
    );
    assert_eq!(revision.snapshot(), optimized);
    assert_eq!(
        revision
            .export(PdfWriterMode::Compact, &context)?
            .output
            .as_bytes(),
        compact.output.as_bytes()
    );
    for result in [&incremental, &compact] {
        assert_eq!(result.snapshot, optimized);
        let reopened = PdfRevision::new(
            PdfEngine::default().open_source(result.output.clone(), OpenOptions::default())?,
        );
        assert_eq!(reopened.text()?, ["after", "untouched"]);
        assert_eq!(
            reopened
                .page_ids()?
                .iter()
                .map(|page| page.number)
                .collect::<Vec<_>>(),
            [3, 4]
        );
        assert_eq!(
            reopened
                .page_labels()?
                .iter()
                .map(|label| label.label.as_str())
                .collect::<Vec<_>>(),
            ["A-1", "B-1"]
        );
    }
    // Publish only real engine exports after the required edit, both writers,
    // strict reopen and a repeated no-op have all passed their local assertions.
    let evidence = std::env::var_os("BINAS_PRESERVATION_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join("evidence"));
    std::fs::create_dir_all(&evidence)?;
    for (name, result) in [
        ("edited-baseline", baseline),
        ("incremental", incremental),
        ("compact", compact),
    ] {
        let path = evidence.join(format!("lossless-images-{name}.pdf"));
        std::fs::write(&path, result.output.as_bytes())?;
        println!(
            "{}: {} bytes, output {:?}, encoded_bytes_saved={encoded_bytes_saved}",
            path.display(),
            result.output.len(),
            result.output.id()
        );
    }
    Ok(())
}
