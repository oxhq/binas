use std::io::Write;

use binas_pdf::{
    EngineConfig, FontTextEditRequest, IncrementalTextEditRequest, Limits, OpenOptions, PdfEngine,
    PdfErrorCode, SurgicalTextEditRequest,
};
use flate2::{Compression, write::ZlibEncoder};

fn flate(input: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn stream(data: &[u8], filtered: bool) -> Vec<u8> {
    let data = if filtered { flate(data) } else { data.to_vec() };
    format!(
        "<< /Length {}{} >>\nstream\n",
        data.len(),
        if filtered {
            " /Filter /FlateDecode"
        } else {
            ""
        }
    )
    .into_bytes()
    .into_iter()
    .chain(data)
    .chain(b"\nendstream".iter().copied())
    .collect()
}

fn pdf(content: &[u8], cmap: Option<&[u8]>, filtered: bool, shared: bool) -> Vec<u8> {
    let kids = if shared { "[3 0 R 8 0 R]" } else { "[3 0 R]" };
    let count = if shared { 2 } else { 1 };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids {kids} /Count {count} /Resources 5 0 R >>").into_bytes(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream(content, filtered),
        b"<< /Font << /F1 6 0 R >> >>".to_vec(),
        if cmap.is_some() {
            b"<< /Type /Font /Subtype /Type0 /ToUnicode 7 0 R >>".to_vec()
        } else {
            b"<< /Type /Font /Subtype /Type1 >>".to_vec()
        },
    ];
    if let Some(cmap) = cmap {
        objects.push(stream(cmap, filtered));
    } else {
        objects.push(b"null".to_vec());
    }
    if shared {
        objects.push(b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec());
    }

    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn unicode_cmap() -> &'static [u8] {
    b"2 begincodespacerange <00> <ff> <0100> <01ff> endcodespacerange 5 beginbfchar <0102> <03a9> <03> <4e2d> <04> <4e2d> <0105> <d83dde00> <06> <0058> endbfchar"
}

fn edit(
    document: &binas_pdf::PdfDocument,
    replacement: &str,
) -> Result<binas_pdf::FontEditOutcome, binas_pdf::PdfError> {
    document.font_text_edit(FontTextEditRequest {
        old_text: "\u{3a9}".into(),
        replacement: replacement.into(),
        match_index: 0,
    })
}

#[test]
fn edits_unicode_with_deterministic_variable_width_codes() {
    let input = pdf(
        b"BT /F1 12 Tf <0102> Tj ET",
        Some(unicode_cmap()),
        false,
        false,
    );
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    assert_eq!(
        document
            .surgical_text_edit(SurgicalTextEditRequest {
                old_text: "\u{3a9}".into(),
                replacement: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    assert_eq!(
        document
            .incremental_text_edit(IncrementalTextEditRequest {
                old_text: "\u{3a9}".into(),
                replacement: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    let outcome = edit(&document, "\u{4e2d}\u{1f600}").unwrap();

    assert!(outcome.verification.passed);
    assert!(outcome.bytes.starts_with(&input));
    assert_eq!(outcome.report.encoded_glyph_bytes, 3);
    let rewritten = PdfEngine::default()
        .open(&outcome.bytes, OpenOptions::default())
        .unwrap();
    let found = rewritten.query_text("\u{4e2d}\u{1f600}", 0).unwrap();
    assert_eq!(
        found.source_span.unwrap().slice(&outcome.bytes),
        Some(b"<030105>".as_slice())
    );
}

#[test]
fn edits_filtered_unicode_and_preserves_the_filter() {
    let input = pdf(
        b"BT /F1 12 Tf <0102> Tj ET",
        Some(unicode_cmap()),
        true,
        false,
    );
    let outcome = edit(
        &PdfEngine::default()
            .open(&input, OpenOptions::default())
            .unwrap(),
        "\u{4e2d}\u{1f600}",
    )
    .unwrap();

    assert!(outcome.verification.passed);
    assert!(
        PdfEngine::default()
            .open(&outcome.bytes, OpenOptions::default())
            .unwrap()
            .query_text("\u{4e2d}\u{1f600}", 0)
            .unwrap()
            .source_span
            .is_none()
    );
}

#[test]
fn refuses_ambiguous_missing_unmapped_and_shared_edits() {
    let ambiguous_cmap = b"1 begincodespacerange <00> <ff> endcodespacerange 4 beginbfchar <01> <03a9> <02> <0041> <03> <0042> <04> <00410042> endbfchar";
    let ambiguous = pdf(
        b"BT /F1 12 Tf <01> Tj ET",
        Some(ambiguous_cmap),
        false,
        false,
    );
    let document = PdfEngine::default()
        .open(&ambiguous, OpenOptions::default())
        .unwrap();
    assert_eq!(
        edit(&document, "AB").unwrap_err().code,
        PdfErrorCode::UnsafeRewrite
    );
    assert_eq!(
        edit(&document, "C").unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );

    let unmapped = pdf(b"BT /F1 12 Tf (old) Tj ET", None, false, false);
    let error = PdfEngine::default()
        .open(&unmapped, OpenOptions::default())
        .unwrap()
        .font_text_edit(FontTextEditRequest {
            old_text: "old".into(),
            replacement: "new".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);

    let shared = pdf(
        b"BT /F1 12 Tf <0102> Tj ET",
        Some(unicode_cmap()),
        false,
        true,
    );
    assert_eq!(
        edit(
            &PdfEngine::default()
                .open(&shared, OpenOptions::default())
                .unwrap(),
            "X"
        )
        .unwrap_err()
        .code,
        PdfErrorCode::UnsafeRewrite
    );
}

#[test]
fn cumulative_content_and_cmap_decode_budget_is_enforced() {
    let content = b"BT /F1 12 Tf <0102> Tj ET";
    let input = pdf(content, Some(unicode_cmap()), true, false);
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_total_decoded_bytes: content.len() + unicode_cmap().len() - 1,
            ..Limits::default()
        },
    });
    assert_eq!(
        edit(&engine.open(&input, OpenOptions::default()).unwrap(), "X")
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
}
