//! Synthetic origin/transform checks only, not glyph layout or rendered appearance.
use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode, TextGeometryConfidence};

fn stream(extra: &str, content: &[u8]) -> Vec<u8> {
    [
        format!("<< /Length {} {extra} >>\nstream\n", content.len()).as_bytes(),
        content,
        b"\nendstream",
    ]
    .concat()
}
fn pdf(page: &[u8], outer: Option<(&str, &[u8])>, inner: Option<(&str, &[u8])>) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /XObject << /Outer 5 0 R /Inner 6 0 R >> /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream("", page),
    ];
    for form in [outer, inner] {
        objects.push(match form {
            Some((extra, content)) => stream(
                &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] {extra}"),
                content,
            ),
            None => b"null".to_vec(),
        });
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

#[test]
fn text_rise_is_transformed_in_text_space_without_font_size_scaling() {
    let bytes = pdf(b"2 0 0 3 10 20 cm BT /F1 12 Tf 0 1 -1 0 30 40 Tm 5 Ts (raised) Tj 0 1 -1 0 30 40 Tm -4 Ts (lowered) Tj ET",None,None);
    let document = PdfEngine::default()
        .open_owned(bytes, OpenOptions::default())
        .unwrap();
    let spans = document.extract_text_spans().unwrap().spans;
    assert_eq!(spans[0].geometry.origin, Some([60.0, 140.0]));
    assert_eq!(spans[1].geometry.origin, Some([78.0, 140.0]));
    assert_eq!(spans[0].geometry.font_size, Some(12.0));
    assert_eq!(
        spans[0].geometry.confidence,
        TextGeometryConfidence::ExactOrigin
    );
    assert_eq!(
        spans[0].geometry.text_matrix,
        Some([0.0, 1.0, -1.0, 0.0, 30.0, 40.0])
    );
}

#[test]
fn nested_forms_compose_noncommuting_transforms_and_text_rise() {
    let bytes = pdf(
        b"2 0 0 3 10 20 cm /Outer Do",
        Some(("/Matrix [0 1 -1 0 4 5]", b"/Inner Do")),
        Some((
            "/Matrix [4 0 0 5 6 7]",
            b"BT /F1 12 Tf 1 0 0 1 8 9 Tm 2 Ts (nested) Tj ET",
        )),
    );
    let document = PdfEngine::default()
        .open_owned(bytes, OpenOptions::default())
        .unwrap();
    let extraction = document.extract_text_spans().unwrap();
    assert!(extraction.warnings.is_empty());
    assert_eq!(extraction.spans.len(), 1);
    let span = &extraction.spans[0];
    assert_eq!(span.object_number, 6);
    assert_eq!(span.text, "nested");
    assert_eq!(span.geometry.origin, Some([-106.0, 149.0]));
}

#[test]
fn matrix_composition_and_rise_overflow_fail_instead_of_reporting_exact_origins() {
    let huge = format!("1{}.0", "0".repeat(200));
    let overflowing_page = format!("{huge} 0 0 {huge} 0 0 cm /Outer Do");
    let matrix = format!("/Matrix [{huge} 0 0 {huge} 0 0]");
    let rise = format!("{huge} 0 0 {huge} 0 0 cm BT {huge} Ts (overflow) Tj ET");
    for bytes in [
        pdf(
            overflowing_page.as_bytes(),
            Some((&matrix, b"BT (nested) Tj ET")),
            None,
        ),
        pdf(rise.as_bytes(), None, None),
    ] {
        let document = PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap();
        let before = document.source_id();
        let error = document.extract_text_spans().unwrap_err();
        assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
        assert!(error.message.contains("not finite"));
        assert_eq!(document.source_id(), before);
    }
}

#[test]
fn null_form_resources_follow_the_existing_absent_entry_fallback() {
    // PDF 32000-1:2008 7.3.9; Adobe PDF Reference 1.4 section3.2.8:
    // https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf
    // A null dictionary value is equivalent to an absent entry. Test direct and
    // resolved indirect null; missing/dangling refs are still separately rejected.
    for resources in ["", "/Resources null", "/Resources 6 0 R"] {
        let bytes = pdf(
            b"/Outer Do",
            Some((resources, b"BT /F1 12 Tf 3 Ts (inherited) Tj ET")),
            None,
        );
        let document = PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap();
        let span = document.extract_text_spans().unwrap().spans.remove(0);
        assert_eq!(span.text, "inherited");
        assert_eq!(span.font_name.as_deref(), Some("F1"));
        assert_eq!(span.geometry.origin, Some([0.0, 3.0]));
    }
    let bytes = pdf(
        b"/Outer Do",
        Some(("/Resources 42", b"BT (bad) Tj ET")),
        None,
    );
    let document = PdfEngine::default()
        .open_owned(bytes, OpenOptions::default())
        .unwrap();
    assert_eq!(
        document.extract_text_spans().unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}
