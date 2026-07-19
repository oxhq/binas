use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode};

#[test]
fn repairs_a_broken_xref_without_weakening_strict_open() {
    let input = include_bytes!("fixtures/pdf/graph-traversal-catalog.pdf");
    let engine = PdfEngine::default();
    assert_eq!(
        engine.open(input, OpenOptions::default()).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
    let repaired = engine.open(input, OpenOptions { repair: true }).unwrap();
    assert_eq!(repaired.inspect().unwrap().page_count, 3);
    assert_eq!(repaired.query_text_all("Page 1").unwrap().len(), 1);
    assert_eq!(repaired.query_text_all("Page 3").unwrap().len(), 1);
}

#[test]
fn repair_stays_bounded_and_refuses_missing_structure() {
    let error = PdfEngine::default()
        .open(
            b"%PDF-1.7\ntrailer\n<< /Size 1 >>\n",
            OpenOptions { repair: true },
        )
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
}
