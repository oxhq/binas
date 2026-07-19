use std::io::Write;

use binas_pdf::{
    OpenOptions, PageCompositionPlacement, PageCompositionRequest, PdfEngine, PdfErrorCode,
};
use flate2::{Compression, write::ZlibEncoder};

fn stream(entries: &str, bytes: &[u8]) -> Vec<u8> {
    [
        format!("<< /Length {}{entries} >>\nstream\n", bytes.len()).into_bytes(),
        bytes.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}

fn classic(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R{trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn destination(encrypted: bool) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /MediaBox [0 0 200 200] /Resources << /XObject << /BinasPage 5 0 R >> >> >>".to_vec(),
        stream("", b"BT (TARGET) Tj ET"),
        stream(
            " /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << >>",
            b"0 0 1 1 re f",
        ),
    ];
    let trailer = if encrypted {
        objects.push(b"<< /Filter /Standard >>".to_vec());
        " /Encrypt 6 0 R"
    } else {
        ""
    };
    classic(&objects, trailer)
}

fn source() -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"BT /F0 12 Tf (SOURCE) Tj ET").unwrap();
    let encoded = encoder.finish().unwrap();
    classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F0 5 0 R >> >> >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /MediaBox [0 0 100 100] /CropBox [5 6 90 80] >>".to_vec(),
            stream(" /Filter /FlateDecode", &encoded),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FontDescriptor 6 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 /ItalicAngle 0 /Ascent 718 /Descent -207 /CapHeight 718 /StemV 88 >>"
                .to_vec(),
        ],
        "",
    )
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

fn request(placement: PageCompositionPlacement) -> PageCompositionRequest {
    PageCompositionRequest {
        target_page_index: 0,
        source_page_index: 0,
        transform: [2.0, 0.0, 0.0, 2.0, 10.0, 20.0],
        placement,
    }
}

#[test]
fn composes_filtered_source_with_isolated_remapped_resources() {
    let outcome = open(&destination(false))
        .compose_page(&open(&source()), request(PageCompositionPlacement::Overlay))
        .unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(outcome.report.operation, "compose_page_overlay");
    let text = String::from_utf8_lossy(&outcome.bytes);
    assert!(text.contains("/BinasPage1"));
    assert!(text.contains("/BBox [5.0 6.0 90.0 80.0]"));
    assert!(text.contains("BT /F0 12 Tf (SOURCE) Tj ET"));
    assert!(text.contains("/FontDescriptor 7 0 R"));
    assert!(text.contains("q 2 0 0 2 10 20 cm /BinasPage1 Do Q"));
    assert_eq!(open(&outcome.bytes).inspect().unwrap().page_count, 1);
}

#[test]
fn controls_underlay_and_overlay_content_order() {
    let target = open(&destination(false));
    let source = open(&source());
    let underlay = target
        .compose_page(&source, request(PageCompositionPlacement::Underlay))
        .unwrap();
    let overlay = target
        .compose_page(&source, request(PageCompositionPlacement::Overlay))
        .unwrap();
    let underlay = String::from_utf8_lossy(&underlay.bytes);
    let overlay = String::from_utf8_lossy(&overlay.bytes);
    assert!(underlay.contains("/Contents [9 0 R 4 0 R]"));
    assert!(overlay.contains("/Contents [4 0 R 9 0 R]"));
}

#[test]
fn rejects_invalid_selection_transform_and_security_boundaries() {
    let target = open(&destination(false));
    let source = open(&source());
    let mut invalid = request(PageCompositionPlacement::Overlay);
    invalid.source_page_index = 1;
    assert_eq!(
        target.compose_page(&source, invalid).unwrap_err().code,
        PdfErrorCode::SelectionNotFound
    );
    let mut invalid = request(PageCompositionPlacement::Overlay);
    invalid.transform = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    assert_eq!(
        target.compose_page(&source, invalid).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );
    assert_eq!(
        open(&destination(true))
            .compose_page(&source, request(PageCompositionPlacement::Overlay))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}
