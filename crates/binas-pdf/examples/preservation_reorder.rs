//! Produces both full-reorder writer candidates for qa/preservation/oracle.py.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{OpenOptions, PdfEngine, PdfRevision, PdfRevisionRequest, PdfWriterMode};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    // Modified for external consumer evidence; fixtures remain frozen.
    let root = std::env::var_os("BINAS_PRESERVATION_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../qa/preservation"));
    for (fixture, prefix) in [
        ("reorder-structures", "reordered"),
        ("alpha-boundary", "alpha"),
    ] {
        let input = std::fs::read(root.join(format!("fixtures/{fixture}.pdf")))?;
        let document = PdfEngine::default().open_owned(input, OpenOptions::default())?;
        let mut revision = PdfRevision::new(document);
        let context = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 64 * 1024 * 1024,
                max_work_units: 256 * 1024 * 1024,
            }),
            CancellationToken::new(),
        );
        let plan = revision.plan(
            PdfRevisionRequest {
                reorder_pages: Some(vec![2, 0, 1]),
                ..Default::default()
            },
            &context,
        )?;
        revision.commit(&plan, &context)?;
        let evidence = std::env::var_os("BINAS_PRESERVATION_OUTPUT_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("evidence"));
        std::fs::create_dir_all(&evidence)?;
        for (name, mode) in [
            ("incremental", PdfWriterMode::Incremental),
            ("compact", PdfWriterMode::Compact),
        ] {
            let exported = revision.export(mode, &context)?;
            let path = evidence.join(format!("{prefix}-{name}.pdf"));
            std::fs::write(&path, exported.output.as_bytes())?;
            println!("{}: {} bytes", path.display(), exported.output.len());
        }
    }
    Ok(())
}
