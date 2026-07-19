use binas_pdf::{IncrementalTextEditRequest, OpenOptions, PdfEngine, PdfErrorCode};

fn pdf(content: &[u8], indirect_length: bool, trailer_extra: &str, signed: bool) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let length = if indirect_length {
        "/Length 5 0 R".to_owned()
    } else {
        format!("/Length {}", content.len())
    };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!("<< {length} >>\nstream\n").into_bytes(),
            content.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];
    if indirect_length {
        objects.push(content.len().to_string().into_bytes());
    } else if signed || !trailer_extra.is_empty() {
        objects.push(if signed {
            b"<< /Type /Sig /ByteRange [0 1 2 3] >>".to_vec()
        } else {
            b"<< >>".to_vec()
        });
    }
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn apply(input: &[u8], old_text: &str, replacement: &str) -> Vec<u8> {
    let document = PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap();
    let outcome = document
        .incremental_text_edit(IncrementalTextEditRequest {
            old_text: old_text.into(),
            replacement: replacement.into(),
            match_index: 0,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    assert!(outcome.verification.prefix_preserved);
    assert_eq!(outcome.report.original_bytes, input.len());
    outcome.bytes
}

#[test]
fn appends_length_changing_literal_revision_and_preserves_prefix() {
    let input = pdf(b"BT (short) Tj ET", false, "", false);
    let output = apply(&input, "short", "a much longer value");
    assert_eq!(&output[..input.len()], input.as_slice());

    let document = PdfEngine::default()
        .open(&output, OpenOptions::default())
        .unwrap();
    assert_eq!(document.inspect().unwrap().xref_revisions, 2);
    assert_eq!(
        document
            .query_text_all("a much longer value")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn appends_length_changing_hex_revision() {
    let input = pdf(b"BT <6F6C64> Tj ET", false, "", false);
    let output = apply(&input, "old", "replacement");
    assert!(
        output
            .windows(24)
            .any(|value| value == b"<7265706C6163656D656E74>")
    );
}

#[test]
fn appends_classic_revision_after_an_xref_stream() {
    let input = include_bytes!("fixtures/pdf/p4-xref-stream-preserve-text.pdf");
    let output = apply(input, "08-15-2024", "May 5, 2026");
    let document = PdfEngine::default()
        .open(&output, OpenOptions::default())
        .unwrap();
    assert_eq!(document.inspect().unwrap().xref_revisions, 2);
    assert_eq!(document.query_text_all("May 5, 2026").unwrap().len(), 1);
}

#[test]
fn refuses_indirect_length_encryption_and_signatures() {
    let indirect = pdf(b"BT (old) Tj ET", true, "", false);
    let document = PdfEngine::default()
        .open(&indirect, OpenOptions::default())
        .unwrap();
    assert_eq!(document.query_text_all("old").unwrap().len(), 1);
    let error = document
        .incremental_text_edit(IncrementalTextEditRequest {
            old_text: "old".into(),
            replacement: "new text".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
    assert!(
        error.message.contains("direct /Length"),
        "{}",
        error.message
    );

    for (input, expected) in [
        (
            pdf(b"(old) Tj", false, "/Encrypt 5 0 R", false),
            "encrypted PDFs",
        ),
        (pdf(b"(old) Tj", false, "", true), "signed PDFs"),
    ] {
        let document = PdfEngine::default()
            .open(&input, OpenOptions::default())
            .unwrap();
        let error = document
            .incremental_text_edit(IncrementalTextEditRequest {
                old_text: "old".into(),
                replacement: "new text".into(),
                match_index: 0,
            })
            .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
        assert!(error.message.contains(expected), "{}", error.message);
    }
}
