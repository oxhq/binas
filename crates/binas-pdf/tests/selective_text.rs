use binas_core::CancellationToken;
use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode};

fn fixture(bad: &[u8], filter: &str, parent_extra: &str, leaf_extra: &str) -> Vec<u8> {
    let stream = |data: &[u8], filter: &str| {
        let mut value = format!("<< /Length {} {filter} >>\nstream\n", data.len()).into_bytes();
        value.extend_from_slice(data);
        value.extend_from_slice(b"\nendstream");
        value
    };
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 100 200] /CropBox [1 2 99 198] /Rotate 90 /Resources << >> {parent_extra} >>").into_bytes(),
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /Contents 6 0 R {leaf_extra} >>").into_bytes(),
        stream(bad, filter),
        stream(b"BT (SAFE) Tj ET", ""),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
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
fn unrelated_malformed_and_expanding_content_is_not_decoded() {
    for (bad, filter, error) in [
        (
            b"G>".as_slice(),
            "/Filter /ASCIIHexDecode",
            PdfErrorCode::InvalidSyntax,
        ),
        (
            &[129, b'x', 128],
            "/Filter /RunLengthDecode",
            PdfErrorCode::ResourceLimit,
        ),
    ] {
        let engine = PdfEngine::new(EngineConfig {
            limits: Limits {
                max_stream_bytes: 64,
                ..Limits::default()
            },
        });
        let document = engine
            .open_owned(fixture(bad, filter, "", ""), OpenOptions::default())
            .unwrap();
        assert_eq!(document.decode_metrics().misses, 0);
        let selected = document.extract_page_text_spans(1).unwrap();
        assert!(selected.warnings.is_empty());
        assert_eq!(selected.spans.len(), 1);
        assert_eq!(selected.spans[0].text, "SAFE");
        assert_eq!(selected.spans[0].page_index, 1);
        assert_eq!(document.decode_metrics().misses, 1);
        assert_eq!(document.extract_page_text_spans(0).unwrap_err().code, error);
        assert_eq!(document.extract_text_spans().unwrap_err().code, error);
        assert_eq!(
            document.extract_page_text_spans(1).unwrap().spans,
            selected.spans
        );
        assert!(document.decode_metrics().hits > 0);
    }
}

#[test]
fn scoped_text_keeps_nested_forms_cmaps_geometry_and_original_indices() {
    let bytes = include_bytes!("../../../qa/preservation/fixtures/reorder-structures.pdf");
    let document = PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap();
    let all = document.extract_text_spans().unwrap();
    assert!(all.warnings.is_empty());
    for page in 0..3 {
        let selected = document.extract_page_text_spans(page).unwrap();
        assert_eq!(
            selected.spans,
            all.spans
                .iter()
                .filter(|span| span.page_index == page)
                .cloned()
                .collect::<Vec<_>>()
        );
        assert!(selected.warnings.is_empty());
    }
}

#[test]
fn invalid_selection_precancel_and_repeated_queries_keep_shared_budget() {
    let input = fixture(b"BAD", "/Filter /UnsupportedCodec", "", "");
    // Budget exactly one complete read under the current work model, including
    // logical text interpretation. Reusing the cache must not buy a second read.
    let probe = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    probe.extract_page_text_spans(1).unwrap();
    let one_read = usize::try_from(probe.decode_metrics().work_bytes).unwrap();
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: one_read,
            ..Limits::default()
        },
    });
    let document = engine.open_owned(input, OpenOptions::default()).unwrap();
    assert_eq!(
        document.extract_page_text_spans(99).unwrap_err().code,
        PdfErrorCode::SelectionNotFound
    );
    assert_eq!(document.decode_metrics().work_bytes, 0);
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        document
            .with_cancellation(token)
            .extract_page_text_spans(1)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    assert_eq!(document.decode_metrics().work_bytes, 0);
    assert_eq!(
        document.extract_page_text_spans(1).unwrap().spans[0].text,
        "SAFE"
    );
    let spent = document.decode_metrics().work_bytes;
    assert_eq!(
        document
            .clone()
            .extract_page_text_spans(1)
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(document.decode_metrics().work_bytes, spent);
}

#[test]
fn optional_boxes_are_direct_only_and_null_inheritable_values_are_absent() {
    let input = fixture(
        b"",
        "",
        "/BleedBox [2 3 98 197] /TrimBox [3 4 97 196] /ArtBox [4 5 96 195]",
        "/MediaBox null /CropBox null /Rotate null /Resources null /BleedBox null /TrimBox [5 6 95 194]",
    );
    let document = PdfEngine::default()
        .open_owned(input, OpenOptions::default())
        .unwrap();
    let first = document.page_geometry(0).unwrap();
    assert_eq!(
        (first.bleed_box, first.trim_box, first.art_box),
        (None, None, None)
    );
    let second = document.page_geometry(1).unwrap();
    assert_eq!(second.media_box, [0., 0., 100., 200.]);
    assert_eq!(second.crop_box, [1., 2., 99., 198.]);
    assert_eq!(second.rotation_degrees, 90);
    assert_eq!(second.bleed_box, None);
    assert_eq!(second.trim_box, Some([5., 6., 95., 194.]));
    assert_eq!(second.art_box, None);
    assert_eq!(
        document.extract_page_text_spans(1).unwrap().spans[0].text,
        "SAFE"
    );
}
