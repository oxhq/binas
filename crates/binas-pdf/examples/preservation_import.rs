//! Produces public page-assembly candidates for the independent import oracle.
use binas_pdf::{OpenOptions, PdfEngine};
use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    // Modified for external consumer evidence; fixtures remain frozen.
    let source = std::env::var_os("BINAS_IMPORT_INPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| crate_root.join("tests/fixtures/preservation"));
    let output = std::env::var_os("BINAS_PRESERVATION_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| crate_root.join("../../qa/preservation/evidence"));
    std::fs::create_dir_all(&output)?;
    let engine = PdfEngine::default();
    let shared = engine.open_owned(
        std::fs::read(source.join("type3-shared.pdf"))?,
        OpenOptions::default(),
    )?;
    let separate = engine.open_owned(
        std::fs::read(source.join("type3-separate.pdf"))?,
        OpenOptions::default(),
    )?;
    for (name, document) in [("shared", &shared), ("separate", &separate)] {
        let candidate = document.extract_pages(&[1, 0, 1])?;
        std::fs::write(
            output.join(format!("import-{name}-duplicate.pdf")),
            candidate.bytes,
        )?;
    }
    let merged = shared.merge_pages(&[&separate])?;
    std::fs::write(output.join("import-merged.pdf"), &merged.bytes)?;
    let merged = engine.open_owned(merged.bytes, OpenOptions::default())?;
    let extracted = merged.extract_pages(&[3, 0, 2, 1, 3])?;
    std::fs::write(output.join("import-merged-extracted.pdf"), extracted.bytes)?;
    println!("Wrote four assembly candidates to {}", output.display());
    Ok(())
}
