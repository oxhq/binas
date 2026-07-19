use std::io::Write;

use binas_pdf::{
    InlineImageColorSpace, InlineImageFilter, InlineImageInventoryEntry,
    InlineImageReplacementRequest, OpenOptions, PdfEngine, PdfErrorCode, list_inline_images,
};
use flate2::{Compression, write::ZlibEncoder};

fn pdf(shared: bool, filtered: bool, signed: bool, encrypted: bool) -> Vec<u8> {
    let content = b"q BI /W 1 /H 1 /BPC 8 /CS /RGB ID\n\x01\x02\x03\nEI Q BT (after) Tj ET";
    let (content, filter) = if filtered {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(content).unwrap();
        (encoder.finish().unwrap(), " /Filter /FlateDecode")
    } else {
        (content.to_vec(), "")
    };
    let second_contents = if shared { "5 0 R" } else { "6 0 R" };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /Contents {second_contents} >>").into_bytes(),
        stream(filter, &content),
        stream("", b"BT (other) Tj ET"),
    ];
    if signed {
        objects.push(b"<< /Type /Sig /ByteRange [0 1 2 3] /Contents (x) >>".to_vec());
    }
    let encrypt = if encrypted {
        objects.push(b"<< /Filter /Standard >>".to_vec());
        format!(" /Encrypt {} 0 R", objects.len())
    } else {
        String::new()
    };
    classic(&objects, &encrypt)
}

fn stream(entries: &str, bytes: &[u8]) -> Vec<u8> {
    [
        format!("<< /Length {}{entries} >>\nstream\n", bytes.len()).into_bytes(),
        bytes.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}

fn classic(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R{trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

fn request(filter: InlineImageFilter, encoded_bytes: Vec<u8>) -> InlineImageReplacementRequest {
    InlineImageReplacementRequest {
        page_index: 0,
        image_index: 0,
        encoded_bytes,
        width: 1,
        height: 1,
        bits_per_component: 8,
        color_space: InlineImageColorSpace::Rgb,
        filter,
    }
}

#[test]
fn lists_direct_unfiltered_inline_image_metadata_without_bytes() {
    let entries = list_inline_images(&open(&pdf(false, false, false, false))).unwrap();

    assert_eq!(
        entries,
        vec![InlineImageInventoryEntry {
            page_index: 0,
            image_index: 0,
            width: 1,
            height: 1,
            color_space: InlineImageColorSpace::Rgb,
            filter: InlineImageFilter::Raw,
            encoded_byte_length: 3,
        }]
    );
}

#[test]
fn inventory_preserves_occurrence_order_and_exact_encoded_lengths() {
    let input = classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
            stream(
                "",
                b"BI /W 1 /H 1 /BPC 8 /CS /G ID x EI BI /W 1 /H 1 /BPC 8 /CS /RGB /F /AHx ID 010203> EI",
            ),
        ],
        "",
    );

    assert_eq!(
        list_inline_images(&open(&input)).unwrap(),
        vec![
            InlineImageInventoryEntry {
                page_index: 0,
                image_index: 0,
                width: 1,
                height: 1,
                color_space: InlineImageColorSpace::Gray,
                filter: InlineImageFilter::Raw,
                encoded_byte_length: 1,
            },
            InlineImageInventoryEntry {
                page_index: 0,
                image_index: 1,
                width: 1,
                height: 1,
                color_space: InlineImageColorSpace::Rgb,
                filter: InlineImageFilter::AsciiHex,
                encoded_byte_length: 7,
            },
        ]
    );
}

#[test]
fn inline_image_inventory_refuses_filtered_array_and_malformed_content() {
    assert_eq!(
        list_inline_images(&open(&pdf(false, true, false, false)))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );

    let array_contents = classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents [4 0 R] >>".to_vec(),
            stream("", b"BI /W 1 /H 1 /BPC 8 /CS /G ID x EI"),
        ],
        "",
    );
    assert_eq!(
        list_inline_images(&open(&array_contents)).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );

    let malformed = classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
            stream("", b"BI /W 1 /H 1 /BPC 8 /CS /G ID x"),
        ],
        "",
    );
    assert_eq!(
        list_inline_images(&open(&malformed)).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn replaces_raw_inline_image_and_preserves_surrounding_content() {
    let outcome = open(&pdf(false, false, false, false))
        .replace_inline_image(request(InlineImageFilter::Raw, vec![9, 8, 7]))
        .unwrap();
    assert!(outcome.verification.passed);
    assert!(outcome.verification.surrounding_bytes_preserved);
    assert_eq!(outcome.report.content_object_number, 5);
    let reopened = open(&outcome.bytes);
    assert_eq!(reopened.query_text_all("after").unwrap().len(), 1);
    assert_eq!(reopened.query_text_all("other").unwrap().len(), 1);
}

#[test]
fn validates_flate_and_asciihex_payloads() {
    let document = open(&pdf(false, false, false, false));
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&[4, 5, 6]).unwrap();
    let flate = document
        .replace_inline_image(request(InlineImageFilter::Flate, encoder.finish().unwrap()))
        .unwrap();
    assert!(flate.verification.encoded_bytes_match);

    let ascii = document
        .replace_inline_image(request(InlineImageFilter::AsciiHex, b"0A141E>".to_vec()))
        .unwrap();
    assert!(ascii.verification.dimensions_match);

    let error = document
        .replace_inline_image(request(InlineImageFilter::Raw, vec![1, 2]))
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&[4, 5, 6]).unwrap();
    let mut ambiguous = encoder.finish().unwrap();
    ambiguous.extend_from_slice(b" EI ");
    assert_eq!(
        document
            .replace_inline_image(request(InlineImageFilter::Flate, ambiguous))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn shared_filtered_and_signed_inputs_fail_closed() {
    assert_eq!(
        open(&pdf(true, false, false, false))
            .replace_inline_image(request(InlineImageFilter::Raw, vec![1, 2, 3]))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    assert_eq!(
        open(&pdf(false, true, false, false))
            .replace_inline_image(request(InlineImageFilter::Raw, vec![1, 2, 3]))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
    assert_eq!(
        open(&pdf(false, false, true, false))
            .replace_inline_image(request(InlineImageFilter::Raw, vec![1, 2, 3]))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    assert_eq!(
        open(&pdf(false, false, false, true))
            .replace_inline_image(request(InlineImageFilter::Raw, vec![1, 2, 3]))
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}
