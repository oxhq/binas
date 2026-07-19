use std::{collections::BTreeMap, io::Write};

use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode};
use flate2::{Compression, write::ZlibEncoder};

fn deflate(input: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

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

fn xref_stream_pdf(
    xref_filter: bool,
    object_filter: bool,
    widths: &str,
    first_override: Option<usize>,
) -> Vec<u8> {
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
    offsets.insert(
        4,
        object(&mut bytes, 4, &stream_body("", b"BT (xref stream) Tj ET")),
    );

    let header = b"1 0 ";
    let catalog = b"<< /Type /Catalog /Pages 2 0 R >>";
    let object_stream: Vec<u8> = header.iter().chain(catalog).copied().collect();
    let first = first_override.unwrap_or(header.len());
    let object_filter_suffix = if object_filter {
        " /Filter /FlateDecode"
    } else {
        ""
    };
    offsets.insert(
        5,
        object(
            &mut bytes,
            5,
            &stream_body(
                &format!(" /Type /ObjStm /N 1 /First {first}{object_filter_suffix}"),
                &if object_filter {
                    deflate(&object_stream)
                } else {
                    object_stream
                },
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
    let filter = if xref_filter {
        " /Filter [/FlateDecode] /DecodeParms [null]"
    } else {
        ""
    };
    let dictionary = format!(" /Type /XRef /Size 8 /Root 1 0 R /W {widths}{filter}");
    let data = if xref_filter { deflate(&data) } else { data };
    object(&mut bytes, 7, &stream_body(&dictionary, &data));
    bytes.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    bytes
}

fn incremental_xref_stream_pdf() -> Vec<u8> {
    let mut bytes = b"%PDF-1.5\n".to_vec();
    let offsets = vec![
        object(&mut bytes, 1, b"<< /Type /Catalog /Pages 99 0 R >>"),
        object(&mut bytes, 2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        object(
            &mut bytes,
            3,
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
        ),
        object(&mut bytes, 4, &stream_body("", b"BT (newest wins) Tj ET")),
    ];
    let previous = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in &offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(b"trailer\n<< /Size 5 /Root 1 0 R >>\n");

    let header = b"1 0 ";
    let catalog = b"<< /Type /Catalog /Pages 2 0 R >>";
    let object_stream: Vec<u8> = header.iter().chain(catalog).copied().collect();
    let object_stream_offset = object(
        &mut bytes,
        5,
        &stream_body(" /Type /ObjStm /N 1 /First 4", &object_stream),
    );
    let xref_offset = bytes.len();
    let mut data = Vec::new();
    data.extend_from_slice(&row(2, 5, 0));
    data.extend_from_slice(&row(1, object_stream_offset, 0));
    data.extend_from_slice(&row(1, xref_offset, 0));
    let dictionary = format!(
        " /Type /XRef /Size 8 /Root 1 0 R /W [1 4 2] /Index [1 1 5 1 7 1] /Prev {previous}"
    );
    object(&mut bytes, 7, &stream_body(&dictionary, &data));
    bytes.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    bytes
}

#[test]
fn opens_default_index_xref_stream_and_compressed_catalog() {
    let input = xref_stream_pdf(false, false, "[1 4 2]", None);
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let inspect = document.inspect().unwrap();
    assert_eq!(inspect.page_count, 1);
    assert_eq!(inspect.object_count, 6);
    assert_eq!(inspect.xref_revisions, 1);
    assert_eq!(
        document.query_text("xref stream", 0).unwrap().text,
        "xref stream"
    );
}

#[test]
fn newest_xref_stream_entries_override_previous_table_entries() {
    let input = incremental_xref_stream_pdf();
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    assert_eq!(document.inspect().unwrap().xref_revisions, 2);
    assert_eq!(
        document.query_text("newest wins", 0).unwrap().match_index,
        0
    );
}

#[test]
fn xref_and_object_stream_malformed_inputs_fail_closed() {
    let compressed = xref_stream_pdf(true, true, "[1 4 2]", None);
    assert_eq!(
        PdfEngine::default()
            .open(&compressed, OpenOptions::default())
            .unwrap()
            .inspect()
            .unwrap()
            .page_count,
        1
    );

    let mut unsupported = xref_stream_pdf(true, false, "[1 4 2]", None);
    let filter = unsupported
        .windows(b"FlateDecode".len())
        .position(|value| value == b"FlateDecode")
        .unwrap();
    unsupported.splice(
        filter..filter + b"FlateDecode".len(),
        b"DCTDecode".iter().copied(),
    );
    assert_eq!(
        PdfEngine::default()
            .open(&unsupported, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );

    let bad_widths = xref_stream_pdf(false, false, "[1 4]", None);
    assert_eq!(
        PdfEngine::default()
            .open(&bad_widths, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );

    let bad_first = xref_stream_pdf(false, false, "[1 4 2]", Some(999));
    assert_eq!(
        PdfEngine::default()
            .open(&bad_first, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );
}

#[test]
fn xref_size_is_charged_before_entries_are_allocated() {
    let input = xref_stream_pdf(false, false, "[1 4 2]", None);
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_xref_entries: 7,
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
