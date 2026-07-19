use std::collections::BTreeMap;

use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode};

fn object(bytes: &mut Vec<u8>, number: u32, body: &[u8]) -> usize {
    let offset = bytes.len();
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
    offset
}

fn stream_body(dict: &str, data: &[u8]) -> Vec<u8> {
    format!("<< /Length {}{dict} >>\nstream\n", data.len())
        .into_bytes()
        .into_iter()
        .chain(data.iter().copied())
        .chain(b"\nendstream".iter().copied())
        .collect()
}

fn row(kind: u8, second: usize, third: usize) -> [u8; 7] {
    let second = u32::try_from(second).unwrap().to_be_bytes();
    let third = u16::try_from(third).unwrap().to_be_bytes();
    [
        kind, second[0], second[1], second[2], second[3], third[0], third[1],
    ]
}

fn object_stream_pdf(object_count: usize, object_value: &[u8]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.5\n".to_vec();
    let mut offsets = BTreeMap::new();
    offsets.insert(
        2,
        object(&mut bytes, 2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
    );
    offsets.insert(
        3,
        object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
        ),
    );
    offsets.insert(4, object(&mut bytes, 4, &stream_body("", b"BT ET")));

    let mut object_stream = b"1 0 ".to_vec();
    object_stream.extend_from_slice(object_value);
    offsets.insert(
        5,
        object(
            &mut bytes,
            5,
            &stream_body(
                &format!(" /Type /ObjStm /N {object_count} /First 4"),
                &object_stream,
            ),
        ),
    );

    let xref_offset = bytes.len();
    offsets.insert(7, xref_offset);
    let mut data = Vec::new();
    data.extend_from_slice(&row(0, 0, 65_535));
    data.extend_from_slice(&row(2, 5, 0));
    for number in 2..=5 {
        data.extend_from_slice(&row(1, offsets[&number], 0));
    }
    data.extend_from_slice(&row(0, 0, 0));
    data.extend_from_slice(&row(1, xref_offset, 0));
    object(
        &mut bytes,
        7,
        &stream_body(" /Type /XRef /Size 8 /Root 1 0 R /W [1 4 2]", &data),
    );
    bytes.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    bytes
}

fn xref_chain(previous: Option<usize>) -> (Vec<u8>, usize) {
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let offset = bytes.len();
    bytes.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 1");
    if let Some(previous) = previous {
        bytes.extend_from_slice(format!(" /Prev {previous}").as_bytes());
    }
    bytes.extend_from_slice(b" >>\n");
    (bytes, offset)
}

fn open_with(input: &[u8], limits: Limits) -> binas_pdf::PdfError {
    PdfEngine::new(EngineConfig { limits })
        .open(input, OpenOptions::default())
        .unwrap_err()
}

#[test]
fn cumulative_decoded_stream_budget_spans_xref_and_object_streams() {
    let input = object_stream_pdf(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let error = open_with(
        &input,
        Limits {
            max_stream_bytes: 64,
            max_total_decoded_bytes: 64,
            ..Limits::default()
        },
    );
    assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    assert!(error.message.contains("decoded stream"));

    PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
}

#[test]
fn prev_cycles_and_revision_depth_fail_closed() {
    let (mut cycle, offset) = xref_chain(None);
    cycle.truncate(cycle.len() - 3);
    cycle.extend_from_slice(format!(" /Prev {offset} >>\nstartxref\n{offset}\n%%EOF\n").as_bytes());
    assert_eq!(
        PdfEngine::default()
            .open(&cycle, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );

    let (mut chain, oldest) = xref_chain(None);
    let newest = chain.len();
    chain.extend_from_slice(b"xref\n0 1\n0000000000 65535 f \ntrailer\n<< /Size 1");
    chain.extend_from_slice(format!(" /Prev {oldest} >>\nstartxref\n{newest}\n%%EOF\n").as_bytes());
    assert_eq!(
        open_with(
            &chain,
            Limits {
                max_xref_revisions: 1,
                ..Limits::default()
            },
        )
        .code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn limits_are_checked_before_stream_and_object_header_allocations() {
    let huge_count = object_stream_pdf(Limits::default().max_objects + 1, b"null");
    assert_eq!(
        PdfEngine::default()
            .open(&huge_count, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );

    let mut input = b"%PDF-1.4\n".to_vec();
    let object_offset = object(
        &mut input,
        1,
        b"<< /Length 100 >>\nstream\nshort\nendstream",
    );
    let xref = input.len();
    input.extend_from_slice(b"xref\n0 2\n0000000000 65535 f \n");
    input.extend_from_slice(format!("{object_offset:010} 00000 n \n").as_bytes());
    input.extend_from_slice(
        format!("trailer\n<< /Size 2 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    assert_eq!(
        open_with(
            &input,
            Limits {
                max_stream_bytes: 8,
                ..Limits::default()
            },
        )
        .code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(
        open_with(
            &input,
            Limits {
                max_container_items: 1,
                ..Limits::default()
            },
        )
        .code,
        PdfErrorCode::ResourceLimit
    );

    let mut repeated = b"%PDF-1.4\n".to_vec();
    let xref = repeated.len();
    repeated.extend_from_slice(
        b"xref\n0 1\n0000000000 65535 f \n0 1\n0000000000 65535 f \n0 1\n0000000000 65535 f \ntrailer\n<< /Size 1 >>\n",
    );
    repeated.extend_from_slice(format!("startxref\n{xref}\n%%EOF\n").as_bytes());
    assert_eq!(
        open_with(
            &repeated,
            Limits {
                max_container_items: 2,
                ..Limits::default()
            },
        )
        .code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn compressed_values_use_parser_depth_limits() {
    let input = object_stream_pdf(1, b"[[[[null]]]]");
    assert_eq!(
        open_with(
            &input,
            Limits {
                max_parser_depth: 2,
                ..Limits::default()
            },
        )
        .code,
        PdfErrorCode::ResourceLimit
    );
}
