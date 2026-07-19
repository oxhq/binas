use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode};

fn pdf(content: &[u8], stream_dict_extra: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Names [null true false 1 -2 3.5 /A#20B (x) <79>] >>"
            .to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        format!(
            "<< /Length {}{} >>\nstream\n",
            content.len(),
            stream_dict_extra
        )
        .into_bytes()
        .into_iter()
        .chain(content.iter().copied())
        .chain(b"\nendstream".iter().copied())
        .collect(),
    ];
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

#[test]
fn opens_inspects_validates_and_queries_literal_and_hex_tj() {
    let input = pdf(
        b"BT (hello\\040world) Tj <68656c6c6f20776f726c64> Tj ET",
        "",
    );
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();

    let inspect = document.inspect().unwrap();
    assert_eq!(inspect.version, "1.7");
    assert_eq!(inspect.object_count, 4);
    assert_eq!(inspect.page_count, 1);
    assert_eq!(inspect.xref_revisions, 1);
    assert_eq!(document.validate().unwrap().page_count, 1);

    let matches = document.query_text_all("hello world").unwrap();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].match_index, 0);
    assert_eq!(matches[1].match_index, 1);
    assert_eq!(document.query_text("hello world", 1).unwrap(), matches[1]);
    assert_eq!(matches[0].object_number, 4);
    assert!(
        matches[0]
            .source_span
            .unwrap()
            .slice(&input)
            .unwrap()
            .starts_with(b"(hello")
    );
}

#[test]
fn malformed_and_over_budget_input_fail_with_typed_errors() {
    let syntax = PdfEngine::default()
        .open(b"not a pdf", OpenOptions::default())
        .unwrap_err();
    assert_eq!(syntax.code, PdfErrorCode::InvalidSyntax);

    let input = pdf(b"(x) Tj", "");
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_input_bytes: input.len() - 1,
            ..Limits::default()
        },
    });
    assert_eq!(
        engine
            .open(&input, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn malformed_filtered_content_fails_instead_of_querying_encoded_bytes() {
    let input = pdf(b"(false positive) Tj", " /Filter /FlateDecode");
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    assert_eq!(
        document.query_text_all("false positive").unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}
