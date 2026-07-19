use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode, TextGeometryConfidence};

#[test]
fn extracts_page_font_operator_and_best_known_geometry() {
    let content = b"q 2 0 0 2 10 20 cm BT /F1 12 Tf 1 0 0 1 30 40 Tm (one) Tj (two) Tj 0 -15 Td (three) Tj ET Q";
    let input = pdf(content);
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let extraction = document.extract_text_spans().unwrap();
    assert!(extraction.warnings.is_empty());
    assert_eq!(
        extraction
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
    let first = &extraction.spans[0];
    assert_eq!(first.page_index, 0);
    assert_eq!(first.object_number, 5);
    assert_eq!(first.font_name.as_deref(), Some("F1"));
    assert_eq!(first.operator, "Tj");
    assert_eq!(first.geometry.font_size, Some(12.0));
    assert_eq!(first.geometry.origin, Some([70.0, 100.0]));
    assert_eq!(
        first.geometry.confidence,
        TextGeometryConfidence::ExactOrigin
    );
    assert_eq!(
        extraction.spans[1].geometry.confidence,
        TextGeometryConfidence::UnknownAdvance
    );
    assert_eq!(extraction.spans[2].geometry.origin, Some([70.0, 70.0]));
}

#[test]
fn preserves_supported_spans_and_reports_unsupported_streams() {
    let input = include_bytes!("fixtures/pdf/p2-unsupported-nonimage-filter-text.pdf");
    let document = PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap();
    let extraction = document.extract_text_spans().unwrap();
    assert_eq!(extraction.spans.len(), 1);
    assert_eq!(extraction.spans[0].text, "SUPPORTED-P2");
    assert_eq!(extraction.warnings.len(), 1);
}

#[test]
fn extracts_nested_form_text_with_inherited_and_local_resources() {
    let input = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 7 0 R >> /XObject << /Outer 5 0 R >> >> >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream(
            "",
            b"BT /F1 10 Tf (before) Tj ET q 1 0 0 1 10 20 cm /Outer Do Q BT /F1 10 Tf (after) Tj ET",
        ),
        stream(
            " /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Matrix [2 0 0 2 3 4] /Resources << /XObject << /Inner 6 0 R >> >>",
            b"q 1 0 0 1 5 6 cm /Inner Do Q",
        ),
        stream(
            " /Type /XObject /Subtype /Form /BBox [0 0 100 100] /Resources << /Font << /InnerFont 8 0 R >> >>",
            b"BT /InnerFont 10 Tf 1 0 0 1 7 8 Tm (nested) Tj ET",
        ),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".to_vec(),
    ]);
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();

    let extraction = document.extract_text_spans().unwrap();
    assert!(extraction.warnings.is_empty());
    assert_eq!(
        extraction
            .spans
            .iter()
            .map(|span| span.text.as_str())
            .collect::<Vec<_>>(),
        ["before", "nested", "after"]
    );
    let span = &extraction.spans[1];
    assert_eq!(span.text, "nested");
    assert_eq!(span.page_index, 0);
    assert_eq!((span.object_number, span.generation), (6, 0));
    assert_eq!(span.font_name.as_deref(), Some("InnerFont"));
    assert_eq!(span.geometry.user_matrix, [2.0, 0.0, 0.0, 2.0, 23.0, 36.0]);
    assert_eq!(span.geometry.origin, Some([37.0, 52.0]));
    assert_eq!(document.query_text("nested", 0).unwrap().object_number, 6);
}

#[test]
fn rejects_form_xobject_cycles() {
    let input = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /XObject << /Loop 5 0 R >> >> >>"
            .to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream("", b"/Loop Do"),
        stream(
            " /Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources << /XObject << /Loop 5 0 R >> >>",
            b"/Loop Do",
        ),
    ]);
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();

    assert_eq!(
        document.extract_text_spans().unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}

#[test]
fn rejects_malformed_do_invocations() {
    let document = PdfEngine::default()
        .open(&pdf(b"Do"), OpenOptions::default())
        .unwrap();

    assert_eq!(
        document.extract_text_spans().unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}

fn pdf(content: &[u8]) -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources 4 0 R /Contents 5 0 R >>".to_vec(),
        b"<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >>".to_vec(),
        {
            let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            stream.extend_from_slice(content);
            stream.extend_from_slice(b"\nendstream");
            stream
        },
    ];
    classic(&objects)
}

fn stream(entries: &str, content: &[u8]) -> Vec<u8> {
    let mut object = format!("<< /Length {}{entries} >>\nstream\n", content.len()).into_bytes();
    object.extend_from_slice(content);
    object.extend_from_slice(b"\nendstream");
    object
}

fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
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
