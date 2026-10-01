use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode};

// Original minimal fixtures; every supplied part is a separate indirect stream.
fn fixture(pages: &[Vec<(&[u8], &str)>]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Count {} /Kids [{}] >>",
            pages.len(),
            (0..pages.len())
                .map(|index| format!("{} 0 R", index + 3))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .into_bytes(),
    ];
    let mut streams = Vec::new();
    for parts in pages {
        let mut refs = Vec::new();
        for (bytes, filter) in parts {
            refs.push(format!("{} 0 R", 3 + pages.len() + streams.len()));
            let mut stream =
                format!("<< /Length {}{filter} >>\nstream\n", bytes.len()).into_bytes();
            stream.extend_from_slice(bytes);
            stream.extend_from_slice(b"\nendstream");
            streams.push(stream);
        }
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> /F2 << /Type /Font /Subtype /Type1 /BaseFont /Courier >> >> >> /Contents [{}] >>", refs.join(" ")).into_bytes());
    }
    objects.extend(streams);
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn canonical_preserves_cross_stream_state_and_physical_operand_identity() {
    let bytes = fixture(&[vec![
        (b"q BT /F1 12 Tf (before)", ""),
        (b"Tj [(array)]", ""),
        (b"TJ ET Q BT /F2 10 Tf (after)", ""),
        (b"Tj ET", ""),
    ]]);
    let engine = PdfEngine::default();
    let document = engine.open_owned(bytes, OpenOptions::default()).unwrap();
    let old = document.extract_text_spans().unwrap();
    let result = document.canonicalize().unwrap();
    assert!(result.verification.text_queries_available);
    assert!(result.verification.text_query_semantics_unchanged);
    let reopened = engine
        .open_owned(result.bytes.clone(), OpenOptions::default())
        .unwrap();
    let new = reopened.extract_text_spans().unwrap();
    let inventory = |spans: &binas_pdf::TextExtraction| {
        spans
            .spans
            .iter()
            .map(|item| {
                (
                    item.page_index,
                    item.object_number,
                    item.generation,
                    item.text.clone(),
                    item.font_name.clone(),
                    item.decoded_span,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(inventory(&old), inventory(&new));
    assert_eq!(
        new.spans
            .iter()
            .map(|item| (item.object_number, item.text.as_str()))
            .collect::<Vec<_>>(),
        [(4, "before"), (5, "array"), (6, "after")]
    );
    assert_eq!(reopened.canonicalize().unwrap().bytes, result.bytes);
}

#[test]
fn canonical_finishes_each_page_and_rejects_incomplete_tokens() {
    let engine = PdfEngine::default();
    for bytes in [
        fixture(&[vec![(b"BT", "")], vec![(b"ET", "")]]),
        fixture(&[vec![(b"q", "")]]),
        fixture(&[vec![(b"BT (split", ""), (b"string) Tj ET", "")]]),
        fixture(&[vec![(b"BT (pending)", "")]]),
    ] {
        let document = engine.open_owned(bytes, OpenOptions::default()).unwrap();
        assert_eq!(
            document.canonicalize().unwrap_err().code,
            PdfErrorCode::InvalidSyntax
        );
    }
}

#[test]
fn canonical_does_not_demote_unknown_graphics_state_to_unavailable_text() {
    let engine = PdfEngine::default();
    for bytes in [
        fixture(&[vec![(b"/GS1", ""), (b"gs BT (unknown-font) Tj ET", "")]]),
        fixture(&[
            vec![(b"712051>", " /Filter /ASCIIHexDecode")],
            vec![(b"/GS1 gs", "")],
        ]),
    ] {
        let document = engine.open_owned(bytes, OpenOptions::default()).unwrap();
        assert_eq!(
            document.canonicalize().unwrap_err().code,
            PdfErrorCode::UnsupportedFeature
        );
    }
}

#[test]
fn canonical_retains_explicit_filtered_unavailable_and_null_filter_is_absent() {
    let engine = PdfEngine::default();
    let filtered = engine
        .open_owned(
            fixture(&[vec![(b"712051>", " /Filter /ASCIIHexDecode")]]),
            OpenOptions::default(),
        )
        .unwrap();
    let output = filtered.canonicalize().unwrap();
    assert!(!output.verification.text_queries_available);
    let raw = engine
        .open_owned(
            fixture(&[vec![(b"BT (raw) Tj ET", " /Filter null")]]),
            OpenOptions::default(),
        )
        .unwrap();
    assert!(
        raw.canonicalize()
            .unwrap()
            .verification
            .text_queries_available
    );
}
