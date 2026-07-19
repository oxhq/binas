use std::io::Write;

use binas_pdf::{FilteredTextEditRequest, OpenOptions, PdfEngine};
use flate2::{Compression, write::ZlibEncoder};

fn deflate(input: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn hex(input: &[u8]) -> Vec<u8> {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = Vec::with_capacity(input.len() * 2 + 1);
    for byte in input {
        output.push(DIGITS[usize::from(byte >> 4)]);
        output.push(DIGITS[usize::from(byte & 15)]);
    }
    output.push(b'>');
    output
}

fn ascii85(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for chunk in input.chunks(4) {
        let mut padded = [0_u8; 4];
        padded[..chunk.len()].copy_from_slice(chunk);
        let mut value = u32::from_be_bytes(padded);
        let mut digits = [0_u8; 5];
        for digit in digits.iter_mut().rev() {
            *digit = (value % 85) as u8 + b'!';
            value /= 85;
        }
        output.extend_from_slice(&digits[..chunk.len() + 1]);
    }
    output.extend_from_slice(b"~>");
    output
}

fn run_length(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for chunk in input.chunks(128) {
        output.push((chunk.len() - 1) as u8);
        output.extend_from_slice(chunk);
    }
    output.push(128);
    output
}

fn literal_lzw(input: &[u8], early_change: u8) -> Vec<u8> {
    let mut output = Vec::new();
    let mut pending = 0_u32;
    let mut pending_bits = 0_u8;
    let mut next_code = 258_usize;
    let mut width = 9_u8;
    let mut previous = false;
    for code in std::iter::once(256_u16)
        .chain(input.iter().copied().map(u16::from))
        .chain(std::iter::once(257))
    {
        pending = (pending << width) | u32::from(code);
        pending_bits += width;
        while pending_bits >= 8 {
            pending_bits -= 8;
            output.push((pending >> pending_bits) as u8);
            pending &= (1_u32 << pending_bits) - 1;
        }
        if code < 256 {
            if previous && next_code < 4096 {
                next_code += 1;
                if width < 12 && next_code >= (1 << width) - usize::from(early_change) {
                    width += 1;
                }
            }
            previous = true;
        }
    }
    if pending_bits != 0 {
        output.push((pending << (8 - pending_bits)) as u8);
    }
    output
}

fn png_up(input: &[u8], columns: usize) -> Vec<u8> {
    let mut output = Vec::new();
    let mut previous = vec![0_u8; columns];
    for row in input.chunks_exact(columns) {
        output.push(2);
        output.extend(
            row.iter()
                .zip(&previous)
                .map(|(value, up)| value.wrapping_sub(*up)),
        );
        previous.copy_from_slice(row);
    }
    output
}

fn tiff(input: &[u8], columns: usize) -> Vec<u8> {
    let mut output = input.to_vec();
    for row in output.chunks_exact_mut(columns) {
        for index in (1..row.len()).rev() {
            row[index] = row[index].wrapping_sub(row[index - 1]);
        }
    }
    output
}

fn pdf(encoded: &[u8], entries: &str) -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!("<< /Length {} {entries} >>\nstream\n", encoded.len()).into_bytes(),
            encoded.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
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
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn edit(input: &[u8], replacement: &str) {
    let document = PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap();
    let outcome = document
        .filtered_text_edit(FilteredTextEditRequest {
            old_text: "short".into(),
            replacement: replacement.into(),
            match_index: 0,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    assert!(outcome.verification.decoded_stream_verified);
    assert!(outcome.verification.filter_metadata_preserved);
    assert_eq!(
        PdfEngine::default()
            .open(&outcome.bytes, OpenOptions::default())
            .unwrap()
            .query_text_all(replacement)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn edits_ascii_hex_ascii85_run_length_and_lzw_streams() {
    let content = b"BT (short) Tj ET";
    let cases = [
        (hex(content), "/Filter /ASCIIHexDecode"),
        (ascii85(content), "/Filter /ASCII85Decode"),
        (run_length(content), "/Filter /RunLengthDecode"),
        (
            literal_lzw(content, 0),
            "/Filter /LZWDecode /DecodeParms << /EarlyChange 0 >>",
        ),
        (
            literal_lzw(content, 1),
            "/Filter /LZWDecode /DecodeParms << /EarlyChange 1 >>",
        ),
    ];
    for (encoded, entries) in cases {
        edit(&pdf(&encoded, entries), "other");
    }
}

#[test]
fn edits_filter_chains_and_tiff_png_predictors() {
    let content = b"BT (short) Tj ET";
    edit(
        &pdf(
            &hex(&deflate(content)),
            "/Filter [/ASCIIHexDecode /FlateDecode]",
        ),
        "a much longer value",
    );
    edit(
        &pdf(
            &deflate(&tiff(content, 4)),
            "/Filter /FlateDecode /DecodeParms << /Predictor 2 /Columns 4 >>",
        ),
        "other",
    );
    edit(
        &pdf(
            &deflate(&png_up(content, 4)),
            "/Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 4 >>",
        ),
        "other",
    );
}
