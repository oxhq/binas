use std::collections::BTreeMap;

use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode};
use md5::{Digest, Md5};
use rc4::{KeyInit, Rc4, StreamCipher, consts::U10};

const PADDING: [u8; 32] = [
    0x28, 0xbf, 0x4e, 0x5e, 0x4e, 0x75, 0x8a, 0x41, 0x64, 0x00, 0x4e, 0x56, 0xff, 0xfa, 0x01, 0x08,
    0x2e, 0x2e, 0x00, 0xb6, 0xd0, 0x68, 0x3e, 0x80, 0x2f, 0x0c, 0xa9, 0xfe, 0x64, 0x53, 0x69, 0x7a,
];

fn rc4(key: &[u8], input: &[u8]) -> Vec<u8> {
    let mut output = input.to_vec();
    let mut cipher = Rc4::<U10>::new_from_slice(key).unwrap();
    cipher.apply_keystream(&mut output);
    output
}

fn object_crypt(file_key: &[u8], number: u32, input: &[u8]) -> Vec<u8> {
    let mut material = file_key.to_vec();
    material.extend_from_slice(&number.to_le_bytes()[..3]);
    material.extend_from_slice(&[0, 0]);
    let digest = Md5::digest(material);
    rc4(&digest[..10], input)
}

fn row(kind: u8, second: usize, third: usize) -> [u8; 7] {
    let second = u32::try_from(second).unwrap().to_be_bytes();
    let third = u16::try_from(third).unwrap().to_be_bytes();
    [
        kind, second[0], second[1], second[2], second[3], third[0], third[1],
    ]
}

fn object(bytes: &mut Vec<u8>, number: u32, body: &[u8]) -> usize {
    let offset = bytes.len();
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
    offset
}

fn encrypted_object_stream_pdf() -> Vec<u8> {
    let file_id = b"fixture-file-id1";
    let owner = (0u8..32).collect::<Vec<_>>();
    let mut padded = b"user".to_vec();
    padded.extend_from_slice(&PADDING[..32 - padded.len()]);
    let mut hash = Md5::new();
    hash.update(&padded);
    hash.update(&owner);
    hash.update((-44i32).to_le_bytes());
    hash.update(file_id);
    let file_key = hash.finalize()[..5].to_vec();

    let mut bytes = b"%PDF-1.5\n".to_vec();
    let mut offsets = BTreeMap::new();
    let content = object_crypt(&file_key, 4, b"BT (encrypted object stream) Tj ET");
    let content_body = [
        format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
        content,
        b"\nendstream".to_vec(),
    ]
    .concat();
    offsets.insert(4, object(&mut bytes, 4, &content_body));
    offsets.insert(
        6,
        object(
            &mut bytes,
            6,
            b"<< /Filter /Standard /V 1 /R 2 /O <000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F> /U <F2E39758794025127846C07006961602075FB11D4B5C227E0B27C9A2ABB73E61> /P -44 >>",
        ),
    );
    let header = b"1 0 2 33 3 74 \n";
    let members = [
        header.as_slice(),
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>",
    ]
    .concat();
    let encrypted_members = object_crypt(&file_key, 7, &members);
    let object_stream = [
        format!(
            "<< /Type /ObjStm /N 3 /First {} /Length {} >>\nstream\n",
            header.len(),
            encrypted_members.len()
        )
        .into_bytes(),
        encrypted_members,
        b"\nendstream".to_vec(),
    ]
    .concat();
    offsets.insert(7, object(&mut bytes, 7, &object_stream));

    let xref_offset = bytes.len();
    offsets.insert(8, xref_offset);
    let mut xref = Vec::new();
    xref.extend_from_slice(&row(0, 0, 65_535));
    for index in 0..3 {
        xref.extend_from_slice(&row(2, 7, index));
    }
    xref.extend_from_slice(&row(1, offsets[&4], 0));
    xref.extend_from_slice(&row(0, 0, 0));
    xref.extend_from_slice(&row(1, offsets[&6], 0));
    xref.extend_from_slice(&row(1, offsets[&7], 0));
    xref.extend_from_slice(&row(1, offsets[&8], 0));
    let dict = format!(
        "<< /Type /XRef /Size 9 /Root 1 0 R /Encrypt 6 0 R /ID [<666978747572652D66696C652D696431> <666978747572652D66696C652D696431>] /W [1 4 2] /Length {} >>\nstream\n",
        xref.len()
    );
    bytes.extend_from_slice(b"8 0 obj\n");
    bytes.extend_from_slice(dict.as_bytes());
    bytes.extend_from_slice(&xref);
    bytes.extend_from_slice(b"\nendstream\nendobj\n");
    bytes.extend_from_slice(format!("startxref\n{xref_offset}\n%%EOF\n").as_bytes());
    bytes
}

#[test]
fn authenticated_open_decrypts_object_streams_before_parsing_members() {
    let input = encrypted_object_stream_pdf();
    let engine = PdfEngine::default();
    assert_eq!(
        engine
            .open(&input, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
    let document = engine
        .open_with_password(&input, "user", OpenOptions::default())
        .unwrap();
    assert_eq!(document.inspect().unwrap().page_count, 1);
    assert_eq!(
        document
            .query_text("encrypted object stream", 0)
            .unwrap()
            .text,
        "encrypted object stream"
    );
    assert_eq!(
        engine
            .open_with_password(&input, "wrong", OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}
