use std::{collections::BTreeMap, io::Write};

use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode};
use flate2::{Compression, write::ZlibEncoder};

fn stream(dictionary: &str, bytes: &[u8]) -> Vec<u8> {
    format!("<< /Length {}{dictionary} >>\nstream\n", bytes.len())
        .into_bytes()
        .into_iter()
        .chain(bytes.iter().copied())
        .chain(b"\nendstream".iter().copied())
        .collect()
}

fn classic(objects: &[(u32, u16, Vec<u8>)], trailer: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = BTreeMap::new();
    for (number, generation, body) in objects {
        offsets.insert(*number, (bytes.len(), *generation));
        bytes.extend_from_slice(format!("{number} {generation} obj\n").as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let size = usize::try_from(*offsets.keys().next_back().unwrap()).unwrap() + 1;
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for number in 1..size {
        match offsets.get(&u32::try_from(number).unwrap()) {
            Some((offset, generation)) => {
                bytes.extend_from_slice(format!("{offset:010} {generation:05} n \n").as_bytes())
            }
            None => bytes.extend_from_slice(b"0000000000 00000 f \n"),
        }
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R{trailer} >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    bytes
}

fn basic(extra_objects: Vec<(u32, u16, Vec<u8>)>, trailer: &str) -> Vec<u8> {
    let mut objects = vec![
        (1, 0, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
        (2, 0, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
        (
            3,
            0,
            b"<< /Type /Page /Parent 2 0 R /Contents 4 2 R >>".to_vec(),
        ),
        (4, 2, stream("", b"BT (canonical text) Tj ET")),
    ];
    objects.extend(extra_objects);
    classic(&objects, trailer)
}

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

fn row(kind: u8, second: usize, third: usize) -> [u8; 7] {
    let second = u32::try_from(second).unwrap().to_be_bytes();
    let third = u16::try_from(third).unwrap().to_be_bytes();
    [
        kind, second[0], second[1], second[2], second[3], third[0], third[1],
    ]
}

fn compressed_pdf() -> Vec<u8> {
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
        object(&mut bytes, 4, &stream("", b"BT (compressed) Tj ET")),
    );
    let packed = b"1 0 << /Type /Catalog /Pages 2 0 R >>";
    offsets.insert(
        5,
        object(
            &mut bytes,
            5,
            &stream(" /Type /ObjStm /N 1 /First 4", packed),
        ),
    );
    let xref_offset = bytes.len();
    offsets.insert(7, xref_offset);
    let mut rows = Vec::new();
    rows.extend_from_slice(&row(0, 0, 65_535));
    rows.extend_from_slice(&row(2, 5, 0));
    for number in 2..=5 {
        rows.extend_from_slice(&row(1, offsets[&number], 0));
    }
    rows.extend_from_slice(&row(0, 0, 0));
    rows.extend_from_slice(&row(1, xref_offset, 0));
    object(
        &mut bytes,
        7,
        &stream(" /Type /XRef /Size 8 /Root 1 0 R /W [1 4 2]", &rows),
    );
    bytes.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    bytes
}

#[test]
fn canonicalizes_classic_deterministically_and_preserves_identity_and_semantics() {
    let input = basic(
        vec![(
            5,
            0,
            b"<< /Values [null true false -2 1.5 0.00000000000000000000000000000000000000001 /A#20B (raw)] >>".to_vec(),
        )],
        " /Info 5 0 R /ID [<0102> <0304>]",
    );
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let first = document.canonicalize().unwrap();
    let second = document.canonicalize().unwrap();
    assert_eq!(first.bytes, second.bytes);
    assert!(first.verification.passed);
    assert!(first.verification.text_queries_available);
    assert!(first.bytes.windows(7).any(|value| value == b"4 2 obj"));
    assert!(first.bytes.windows(11).any(|value| value == b"/Info 5 0 R"));
    assert!(!first.bytes.windows(5).any(|value| value == b"/Prev"));
    assert!(!first.bytes.windows(8).any(|value| value == b"/XRefStm"));
    assert_eq!(
        PdfEngine::default()
            .open(&first.bytes, OpenOptions::default())
            .unwrap()
            .query_text_all("canonical text")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn expands_object_stream_members_into_a_classic_xref() {
    let output = PdfEngine::default()
        .open(&compressed_pdf(), OpenOptions::default())
        .unwrap()
        .canonicalize()
        .unwrap()
        .bytes;
    assert!(output.windows(7).any(|value| value == b"1 0 obj"));
    assert!(output.windows(5).any(|value| value == b"xref\n"));
    assert_eq!(
        PdfEngine::default()
            .open(&output, OpenOptions::default())
            .unwrap()
            .query_text_all("compressed")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn filtered_stream_bytes_pass_through_and_output_limit_is_enforced() {
    let encoded = deflate(b"BT (filtered) Tj ET");
    let input = {
        let objects = vec![
            (1, 0, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
            (2, 0, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
            (
                3,
                0,
                b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
            ),
            (4, 0, stream(" /Filter /FlateDecode", &encoded)),
        ];
        classic(&objects, "")
    };
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let output = document.canonicalize().unwrap();
    assert!(!output.verification.text_queries_available);
    assert!(
        output
            .bytes
            .windows(encoded.len())
            .any(|value| value == encoded)
    );

    let limited = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_output_bytes: 32,
            ..Limits::default()
        },
    })
    .open(&input, OpenOptions::default())
    .unwrap();
    assert_eq!(
        limited.canonicalize().unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn encryption_and_signatures_are_refused_before_rewrite() {
    let encrypted = basic(
        vec![(5, 0, b"<< /Filter /Standard >>".to_vec())],
        " /Encrypt 5 0 R",
    );
    assert_eq!(
        PdfEngine::default()
            .open(&encrypted, OpenOptions::default())
            .unwrap()
            .canonicalize()
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    let signed = basic(
        vec![(5, 0, b"<< /Type /Sig /ByteRange [0 1 2 3] >>".to_vec())],
        "",
    );
    assert_eq!(
        PdfEngine::default()
            .open(&signed, OpenOptions::default())
            .unwrap()
            .canonicalize()
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}
