use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode, SurgicalTextEditRequest};

fn pdf(content: &[u8]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn edit(input: &[u8], old_text: &str, replacement: &str, match_index: usize) -> Vec<u8> {
    let document = PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap();
    let outcome = document
        .surgical_text_edit(SurgicalTextEditRequest {
            old_text: old_text.into(),
            replacement: replacement.into(),
            match_index,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(outcome.report.mode, "surgical");
    outcome.bytes
}

#[test]
fn edits_literal_and_selected_duplicate_without_other_byte_changes() {
    let input = pdf(b"BT (hello) Tj (hello) Tj ET");
    let span = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap()
        .query_text("hello", 1)
        .unwrap()
        .source_span
        .unwrap();
    let output = edit(&input, "hello", "world", 1);
    assert_eq!(input.len(), output.len());
    for (index, (before, after)) in input.iter().zip(&output).enumerate() {
        if before != after {
            assert!((span.start() as usize..span.end() as usize).contains(&index));
        }
    }

    let document = PdfEngine::default()
        .open(&output, OpenOptions::default())
        .unwrap();
    assert_eq!(document.query_text_all("hello").unwrap().len(), 1);
    assert_eq!(document.query_text_all("world").unwrap().len(), 1);
}

#[test]
fn edits_hex_tj_without_changing_span_length() {
    let input = pdf(b"BT <68656C6C6F> Tj ET");
    let output = edit(&input, "hello", "world", 0);
    assert!(output.windows(12).any(|bytes| bytes == b"<776F726C64>"));
}

#[test]
fn refuses_a_replacement_that_cannot_preserve_the_token_span() {
    let input = pdf(b"BT (short) Tj ET");
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let error = document
        .surgical_text_edit(SurgicalTextEditRequest {
            old_text: "short".into(),
            replacement: "longer".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
    assert_eq!(document.source_len(), input.len());
}
