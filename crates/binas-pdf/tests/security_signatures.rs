use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode, inspect_encryption, inspect_signatures};

fn pdf(objects: &[String], trailer_extra: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
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

fn signature_pdf() -> Vec<u8> {
    let placeholder = "9999999999";
    let objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R >>".into(),
        format!(
            "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached \
             /Name (Signer) /M (D:20260710000000Z) \
             /ByteRange [0 {placeholder} {placeholder} {placeholder}] \
             /Contents <{}> >>",
            "00".repeat(64)
        ),
    ];
    let mut bytes = pdf(&objects, "");
    let contents_marker = b"/Contents <";
    let marker = bytes
        .windows(contents_marker.len())
        .position(|value| value == contents_marker)
        .unwrap();
    let gap_start = marker + b"/Contents ".len();
    let gap_end = gap_start
        + bytes[gap_start..]
            .iter()
            .position(|byte| *byte == b'>')
            .unwrap()
        + 1;
    let ranges = [gap_start, gap_end, bytes.len() - gap_end];
    let mut search = 0usize;
    for value in ranges {
        let position = bytes[search..]
            .windows(placeholder.len())
            .position(|item| item == placeholder.as_bytes())
            .map(|position| position + search)
            .unwrap();
        bytes[position..position + placeholder.len()]
            .copy_from_slice(format!("{value:010}").as_bytes());
        search = position + placeholder.len();
    }
    bytes
}

#[test]
fn reports_standard_encryption_dictionary_without_claiming_decryption() {
    let plain = pdf(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R >>".into(),
        ],
        "",
    );
    assert!(!inspect_encryption(&open(&plain)).unwrap().encrypted);

    let encrypted = pdf(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R >>".into(),
            "<< /Filter /Standard /V 4 /R 4 /Length 128 /P -4 /EncryptMetadata false /StmF /StdCF /StrF /StdCF >>".into(),
        ],
        " /Encrypt 4 0 R",
    );
    let metadata = inspect_encryption(&open(&encrypted)).unwrap();
    assert!(metadata.encrypted);
    assert_eq!(metadata.filter.as_deref(), Some("Standard"));
    assert_eq!(metadata.revision, Some(4));
    assert_eq!(metadata.permissions, Some(-4));
    assert_eq!(metadata.encrypt_metadata, Some(false));
}

#[test]
fn validates_byte_ranges_and_hashes_only_signed_bytes() {
    let input = signature_pdf();
    let signature = inspect_signatures(&open(&input)).unwrap().remove(0);
    assert!(signature.covers_current_file);
    assert_eq!(signature.later_bytes, 0);
    assert_eq!(signature.contents_bytes, 0);
    assert_eq!(signature.signed_bytes_sha256.len(), 64);
    assert!(!signature.cms_verified);

    let mut gap_changed = input.clone();
    gap_changed[usize::try_from(signature.gap_start).unwrap() + 1] = b'1';
    let same = inspect_signatures(&open(&gap_changed)).unwrap().remove(0);
    assert_eq!(same.signed_bytes_sha256, signature.signed_bytes_sha256);

    let mut covered_changed = input.clone();
    covered_changed[5] = b'2';
    let changed = inspect_signatures(&open(&covered_changed))
        .unwrap()
        .remove(0);
    assert_ne!(changed.signed_bytes_sha256, signature.signed_bytes_sha256);
}

#[test]
fn malformed_or_overlapping_byte_ranges_fail_closed() {
    let mut input = signature_pdf();
    let marker = b"/ByteRange [0 ";
    let start = input
        .windows(marker.len())
        .position(|value| value == marker)
        .unwrap()
        + marker.len();
    let second_start = start + 11;
    input[second_start..second_start + 10].copy_from_slice(b"0000000001");
    assert_eq!(
        inspect_signatures(&open(&input)).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}
