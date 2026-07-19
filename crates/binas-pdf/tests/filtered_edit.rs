use std::io::Write;

use binas_pdf::{
    EngineConfig, FilteredTextEditRequest, Limits, OpenOptions, PdfEngine, PdfErrorCode,
};
use flate2::{Compression, write::ZlibEncoder};

fn deflate(input: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn pdf(content: &[u8], stream_entries: &str, shared: bool, signed: bool) -> Vec<u8> {
    let encoded = deflate(content);
    let kids = if shared { "[3 0 R 5 0 R]" } else { "[3 0 R]" };
    let count = if shared { 2 } else { 1 };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!("<< /Type /Pages /Kids {kids} /Count {count} >>").into_bytes(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!(
                "<< /Length {} /Filter /FlateDecode{stream_entries} >>\nstream\n",
                encoded.len()
            )
            .into_bytes(),
            encoded,
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];
    if shared {
        objects.push(b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec());
    } else if signed {
        objects.push(b"<< /Type /Sig /ByteRange [0 1 2 3] >>".to_vec());
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

fn request() -> FilteredTextEditRequest {
    FilteredTextEditRequest {
        old_text: "short".into(),
        replacement: "a much longer value".into(),
        match_index: 0,
    }
}

#[test]
fn replaces_length_changing_flate_text_in_an_incremental_revision() {
    let input = pdf(b"BT (short) Tj ET", "", false, false);
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    assert!(
        document
            .query_text("short", 0)
            .unwrap()
            .source_span
            .is_none()
    );
    let outcome = document.filtered_text_edit(request()).unwrap();
    assert_eq!(&outcome.bytes[..input.len()], input.as_slice());
    assert!(outcome.verification.passed);
    assert!(outcome.verification.decoded_stream_verified);
    assert_eq!(outcome.report.mode, "filtered_incremental");
    let rewritten = PdfEngine::default()
        .open(&outcome.bytes, OpenOptions::default())
        .unwrap();
    assert_eq!(rewritten.inspect().unwrap().xref_revisions, 2);
    assert_eq!(
        rewritten
            .query_text_all("a much longer value")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn replaces_a_filtered_hex_text_token() {
    let input = pdf(b"BT <73686F7274> Tj ET", "", false, false);
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let outcome = document.filtered_text_edit(request()).unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(
        PdfEngine::default()
            .open(&outcome.bytes, OpenOptions::default())
            .unwrap()
            .query_text_all("a much longer value")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn refuses_unsupported_predictors_shared_streams_and_signatures() {
    for (input, code, message) in [
        (
            pdf(
                b"BT (short) Tj ET",
                " /DecodeParms << /Predictor 9 >>",
                false,
                false,
            ),
            PdfErrorCode::UnsupportedFeature,
            "predictor 9",
        ),
        (
            pdf(b"BT (short) Tj ET", "", true, false),
            PdfErrorCode::UnsafeRewrite,
            "unambiguous",
        ),
        (
            pdf(b"BT (short) Tj ET", "", false, true),
            PdfErrorCode::UnsafeRewrite,
            "signed PDFs",
        ),
    ] {
        let document = PdfEngine::default()
            .open(&input, OpenOptions::default())
            .unwrap();
        let error = document.filtered_text_edit(request()).unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.message.contains(message), "{}", error.message);
    }
}

#[test]
fn enforces_output_limit_before_returning_bytes() {
    let input = pdf(b"BT (short) Tj ET", "", false, false);
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_output_bytes: input.len(),
            ..Limits::default()
        },
    })
    .open(&input, OpenOptions::default())
    .unwrap();
    assert_eq!(
        document.filtered_text_edit(request()).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}
