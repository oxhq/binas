use binas_pdf::{
    OpenOptions, PdfEngine, PdfErrorCode, PublicKeyEncryptionMethod, PublicKeyEncryptionOptions,
    inspect_encryption,
};
use rcgen::{CertificateParams, DnType, KeyPair, PKCS_RSA_SHA256};
use rsa::{
    RsaPrivateKey,
    pkcs8::{EncodePrivateKey, LineEnding},
    rand_core::OsRng,
};

fn make_certificate(common_name: &str) -> (Vec<u8>, Vec<u8>) {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let private_key = private.to_pkcs8_der().unwrap().as_bytes().to_vec();
    let pem = private.to_pkcs8_pem(LineEnding::LF).unwrap();
    let key = KeyPair::from_pem_and_sign_algo(&pem, &PKCS_RSA_SHA256).unwrap();
    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    let certificate = params.self_signed(&key).unwrap();
    (certificate.der().to_vec(), private_key)
}

fn pdf(text: &str) -> Vec<u8> {
    let content = format!("BT ({text}) Tj ET").into_bytes();
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content,
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

fn open(bytes: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap()
}

#[test]
fn adobe_pubsec_rc4_aesv2_and_aesv3_roundtrip_with_the_authorized_recipient() {
    let (certificate, private_key) = make_certificate("Authorized Recipient");
    for method in [
        PublicKeyEncryptionMethod::Rc4,
        PublicKeyEncryptionMethod::AesV2,
        PublicKeyEncryptionMethod::AesV3,
    ] {
        let encrypted = open(&pdf("RECIPIENT-ONLY"))
            .encrypt_public_key(PublicKeyEncryptionOptions {
                method,
                recipient_certificates_der: vec![certificate.clone()],
                permissions: -1028,
            })
            .unwrap();
        assert!(encrypted.verification.passed);
        assert!(
            !encrypted
                .bytes
                .windows(14)
                .any(|value| value == b"RECIPIENT-ONLY")
        );
        let encrypted_document = open(&encrypted.bytes);
        let metadata = inspect_encryption(&encrypted_document).unwrap();
        assert_eq!(metadata.filter.as_deref(), Some("Adobe.PubSec"));
        assert_eq!(metadata.sub_filter.as_deref(), Some("adbe.pkcs7.s5"));
        let (version, bits) = match method {
            PublicKeyEncryptionMethod::Rc4 | PublicKeyEncryptionMethod::AesV2 => (4, 128),
            PublicKeyEncryptionMethod::AesV3 => (5, 256),
        };
        assert_eq!(metadata.version, Some(version));
        assert_eq!(metadata.key_length_bits, Some(bits));
        assert_eq!(encrypted.report.revision, version);

        let decrypted = encrypted_document
            .decrypt_public_key(&certificate, &private_key)
            .unwrap();
        assert_eq!(decrypted.permissions, -1028);
        assert!(decrypted.verification.passed);
        assert_eq!(
            open(&decrypted.bytes)
                .query_text_all("RECIPIENT-ONLY")
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn wrong_recipient_and_tampered_cms_fail_without_secret_material() {
    let (certificate, private_key) = make_certificate("Authorized Recipient");
    let (wrong_certificate, wrong_private_key) = make_certificate("Wrong Recipient");
    let encrypted = open(&pdf("SECRET"))
        .encrypt_public_key(PublicKeyEncryptionOptions {
            method: PublicKeyEncryptionMethod::AesV2,
            recipient_certificates_der: vec![certificate.clone()],
            permissions: -44,
        })
        .unwrap();
    let error = open(&encrypted.bytes)
        .decrypt_public_key(&wrong_certificate, &wrong_private_key)
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
    assert!(!error.message.contains("Wrong Recipient"));
    assert!(!error.message.contains(&hex(&wrong_private_key[..16])));

    let mut tampered = encrypted.bytes;
    let marker = b"/Recipients [<";
    let start = tampered
        .windows(marker.len())
        .position(|value| value == marker)
        .unwrap()
        + marker.len();
    let end = tampered[start..]
        .iter()
        .position(|byte| *byte == b'>')
        .unwrap()
        + start;
    tampered[end - 2] = if tampered[end - 2] == b'0' {
        b'1'
    } else {
        b'0'
    };
    let error = open(&tampered)
        .decrypt_public_key(&certificate, &private_key)
        .unwrap_err();
    assert!(matches!(
        error.code,
        PdfErrorCode::InvalidSyntax | PdfErrorCode::UnsafeRewrite
    ));
}

#[test]
fn legacy_s3_and_s4_top_level_recipient_arrays_decrypt() {
    let (certificate, private_key) = make_certificate("Legacy Recipient");
    let encrypted = open(&pdf("LEGACY-PUBSEC"))
        .encrypt_public_key(PublicKeyEncryptionOptions {
            method: PublicKeyEncryptionMethod::Rc4,
            recipient_certificates_der: vec![certificate.clone()],
            permissions: -44,
        })
        .unwrap();

    for sub_filter in ["adbe.pkcs7.s3", "adbe.pkcs7.s4"] {
        let legacy = legacy_public_key_revision(&encrypted.bytes, sub_filter);
        let decrypted = PdfEngine::default()
            .decrypt_public_key_input(&legacy, &certificate, &private_key, OpenOptions::default())
            .unwrap();
        assert_eq!(decrypted.report.revision, 2);
        assert_eq!(decrypted.report.crypt_filter, "V2");
        assert_eq!(
            open(&decrypted.bytes)
                .query_text_all("LEGACY-PUBSEC")
                .unwrap()
                .len(),
            1
        );
    }
}

fn legacy_public_key_revision(input: &[u8], sub_filter: &str) -> Vec<u8> {
    let marker = b"/Recipients [<";
    let recipient_start = input
        .windows(marker.len())
        .position(|value| value == marker)
        .unwrap()
        + marker.len();
    let recipient_end = input[recipient_start..]
        .iter()
        .position(|byte| *byte == b'>')
        .unwrap()
        + recipient_start;
    let recipient = &input[recipient_start..recipient_end];
    let startxref = b"startxref\n";
    let old_xref_start = input
        .windows(startxref.len())
        .rposition(|value| value == startxref)
        .unwrap()
        + startxref.len();
    let old_xref_end = input[old_xref_start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap()
        + old_xref_start;
    let old_xref = std::str::from_utf8(&input[old_xref_start..old_xref_end])
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let mut output = input.to_vec();
    let object_offset = output.len();
    output.extend_from_slice(
        format!(
            "\n5 0 obj\n<< /Filter /Adobe.PubSec /SubFilter /{sub_filter} /V 2 /Length 128 /Recipients [<{}>] >>\nendobj\n",
            std::str::from_utf8(recipient).unwrap()
        )
        .as_bytes(),
    );
    let xref = output.len();
    output.extend_from_slice(
        format!(
            "xref\n5 1\n{object_offset:010} 00000 n \ntrailer\n<< /Size 6 /Root 1 0 R /Encrypt 5 0 R /Prev {old_xref} >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .as_bytes(),
    );
    output
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
