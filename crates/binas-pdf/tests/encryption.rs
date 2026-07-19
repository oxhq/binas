use binas_pdf::{
    OpenOptions, PdfEngine, PdfErrorCode, StandardEncryptionOptions, StandardEncryptionRevision,
    inspect_encryption,
};

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
        b"<< /Title (Sensitive Title) >>".to_vec(),
    ];
    classic(&objects, 1, Some(5), "")
}

fn classic(objects: &[Vec<u8>], root: u32, info: Option<u32>, trailer_extra: &str) -> Vec<u8> {
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
    let info = info.map_or(String::new(), |number| format!(" /Info {number} 0 R"));
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root {root} 0 R{info}{trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn fixed_r2_vector() -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        b"<< /Length 0 >>\nstream\n\nendstream".to_vec(),
        b"null".to_vec(),
        b"<< /Filter /Standard /V 1 /R 2 /O <000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F> /U <F2E39758794025127846C07006961602075FB11D4B5C227E0B27C9A2ABB73E61> /P -44 >>".to_vec(),
    ];
    objects.extend((7..12).map(|_| b"null".to_vec()));
    objects.push(b"<< /Title <089E5E3660EE770D564DE9201BF15209A62290> >>".to_vec());
    classic(
        &objects,
        1,
        Some(12),
        " /Encrypt 6 0 R /ID [<666978747572652D66696C652D696431> <666978747572652D66696C652D696431>]",
    )
}

fn fixed_r3_dictionary(version: i64, length: i64) -> Vec<u8> {
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        b"<< /Length 0 >>\nstream\n\nendstream".to_vec(),
        format!(
            "<< /Filter /Standard /V {version} /R 3 /Length {length} /O <3A59A4C4747915B0DC733CB81E3C81530679739DAC36732902D1C913ED95FF72> /U <EC6652447AA5176E384415220B40A70D0122456A91BAE5134273A6DB134C87C4> /P -4 >>"
        )
        .into_bytes(),
    ];
    classic(
        &objects,
        1,
        None,
        " /Encrypt 5 0 R /ID [<FCE2FE96B7E142B4A0576F61E2E9C441> <FCE2FE96B7E142B4A0576F61E2E9C441>]",
    )
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

#[test]
fn decrypts_the_fixed_revision_two_authoritative_vector() {
    let input = fixed_r2_vector();
    let outcome = open(&input).decrypt_to_plain("user").unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(outcome.report.revision, 2);
    assert_eq!(outcome.report.crypt_filter, "V2");
    assert!(!inspect_encryption(&open(&outcome.bytes)).unwrap().encrypted);
    let expected = b"<536563726574206F626A656374206279746573>";
    assert!(
        outcome
            .bytes
            .windows(expected.len())
            .any(|value| value == expected)
    );
}

#[test]
fn all_standard_revisions_roundtrip_user_and_owner_passwords() {
    for revision in [
        StandardEncryptionRevision::R2Rc4,
        StandardEncryptionRevision::R3Rc4(40),
        StandardEncryptionRevision::R3Rc4(80),
        StandardEncryptionRevision::R3Rc4(128),
        StandardEncryptionRevision::R4Rc4,
        StandardEncryptionRevision::R4AesV2,
        StandardEncryptionRevision::R5Aes256,
        StandardEncryptionRevision::R6Aes256,
    ] {
        let input = pdf("OPEN-SESAME");
        let encrypted = open(&input)
            .encrypt_standard(StandardEncryptionOptions {
                revision,
                user_password: "user-password".into(),
                owner_password: "owner-password".into(),
                permissions: -1028,
            })
            .unwrap();
        assert!(encrypted.verification.passed);
        assert_eq!(
            encrypted.report.crypt_filter,
            match revision {
                StandardEncryptionRevision::R2Rc4
                | StandardEncryptionRevision::R3Rc4(_)
                | StandardEncryptionRevision::R4Rc4 => "V2",
                StandardEncryptionRevision::R4AesV2 => "AESV2",
                StandardEncryptionRevision::R5Aes256 | StandardEncryptionRevision::R6Aes256 =>
                    "AESV3",
            }
        );
        assert!(
            !encrypted
                .bytes
                .windows(11)
                .any(|value| value == b"OPEN-SESAME")
        );
        let metadata = inspect_encryption(&open(&encrypted.bytes)).unwrap();
        assert!(metadata.encrypted);
        assert_eq!(
            metadata.revision,
            Some(match revision {
                StandardEncryptionRevision::R2Rc4 => 2,
                StandardEncryptionRevision::R3Rc4(_) => 3,
                StandardEncryptionRevision::R4Rc4 | StandardEncryptionRevision::R4AesV2 => 4,
                StandardEncryptionRevision::R5Aes256 => 5,
                StandardEncryptionRevision::R6Aes256 => 6,
            })
        );
        for password in ["user-password", "owner-password"] {
            let plain = open(&encrypted.bytes).decrypt_to_plain(password).unwrap();
            assert!(plain.verification.passed);
            assert_eq!(
                open(&plain.bytes)
                    .query_text_all("OPEN-SESAME")
                    .unwrap()
                    .len(),
                1
            );
        }
        let error = open(&encrypted.bytes)
            .decrypt_to_plain("wrong-password")
            .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
        assert!(!error.message.contains("wrong-password"));
    }
}

#[test]
fn aes256_permissions_tampering_fails_closed() {
    let encrypted = open(&pdf("PERMISSIONS-MUST-AUTHENTICATE"))
        .encrypt_standard(StandardEncryptionOptions {
            revision: StandardEncryptionRevision::R6Aes256,
            user_password: "user-password".into(),
            owner_password: "owner-password".into(),
            permissions: -1028,
        })
        .unwrap();
    let mut tampered = encrypted.bytes;
    let marker = b"/Perms <";
    let offset = tampered
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap()
        + marker.len();
    tampered[offset] = if tampered[offset] == b'0' { b'1' } else { b'0' };
    let error = open(&tampered)
        .decrypt_to_plain("user-password")
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
    assert!(error.message.contains("permissions"));
}

#[test]
fn aes256_requires_the_exact_aesv3_crypt_filter_policy() {
    let encrypted = open(&pdf("CRYPT-FILTER-POLICY"))
        .encrypt_standard(StandardEncryptionOptions {
            revision: StandardEncryptionRevision::R5Aes256,
            user_password: "u".repeat(200),
            owner_password: "owner".into(),
            permissions: -1028,
        })
        .unwrap();
    assert!(
        open(&encrypted.bytes)
            .decrypt_to_plain(&"u".repeat(127))
            .unwrap()
            .verification
            .passed
    );
    let mut wrong_filter = encrypted.bytes;
    let offset = wrong_filter
        .windows(6)
        .position(|window| window == b"AESV3 ")
        .unwrap();
    wrong_filter[offset + 4] = b'2';
    let error = open(&wrong_filter).decrypt_to_plain("u").unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    assert!(error.message.contains("AESV3"));
}

#[test]
fn aes256_saslprep_normalizes_non_ascii_passwords() {
    for revision in [
        StandardEncryptionRevision::R5Aes256,
        StandardEncryptionRevision::R6Aes256,
    ] {
        let encrypted = open(&pdf("UNICODE-PASSWORD"))
            .encrypt_standard(StandardEncryptionOptions {
                revision,
                user_password: "I\u{00ad}X\u{00a0}cafe\u{0301}".into(),
                owner_password: "Ｏｗｎｅｒ".into(),
                permissions: -1028,
            })
            .unwrap();
        for equivalent in ["IX café", "Owner"] {
            assert!(
                open(&encrypted.bytes)
                    .decrypt_to_plain(equivalent)
                    .unwrap()
                    .verification
                    .passed
            );
        }
    }
}

#[test]
fn aes256_saslprep_rejects_prohibited_and_unassigned_input() {
    for password in ["control\u{0007}", "unassigned\u{0221}"] {
        let error = open(&pdf("INVALID-PASSWORD"))
            .encrypt_standard(StandardEncryptionOptions {
                revision: StandardEncryptionRevision::R6Aes256,
                user_password: password.into(),
                owner_password: "owner".into(),
                permissions: -1028,
            })
            .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
        assert_eq!(error.message, "R5/R6 password is not valid SASLprep input");
        assert!(!error.message.contains(password));
    }

    let encrypted = open(&pdf("INVALID-DECRYPT-PASSWORD"))
        .encrypt_standard(StandardEncryptionOptions {
            revision: StandardEncryptionRevision::R6Aes256,
            user_password: "valid".into(),
            owner_password: "owner".into(),
            permissions: -1028,
        })
        .unwrap();
    let error = open(&encrypted.bytes)
        .decrypt_to_plain("control\u{0007}")
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
}

#[test]
fn unsupported_revisions_and_malformed_credentials_fail_closed() {
    let mut unsupported = fixed_r2_vector();
    let offset = unsupported
        .windows(4)
        .position(|value| value == b"/R 2")
        .unwrap();
    unsupported[offset + 3] = b'3';
    let error = open(&unsupported).decrypt_to_plain("user").unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    assert!(error.message.contains("R=3"));

    let mut malformed = fixed_r2_vector();
    let marker = b"/U <";
    let start = malformed
        .windows(marker.len())
        .position(|value| value == marker)
        .unwrap()
        + marker.len();
    malformed[start] = if malformed[start] == b'0' { b'1' } else { b'0' };
    let error = open(&malformed).decrypt_to_plain("user").unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
}

#[test]
fn r3_rejects_invalid_rc4_key_lengths() {
    for bits in [0, 39, 41, 127, 129, u16::MAX] {
        let error = open(&pdf("INVALID-R3-LENGTH"))
            .encrypt_standard(StandardEncryptionOptions {
                revision: StandardEncryptionRevision::R3Rc4(bits),
                user_password: "user".into(),
                owner_password: "owner".into(),
                permissions: -4,
            })
            .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
        assert!(error.message.contains("40 to 128 bits"));
    }
    for input in [fixed_r3_dictionary(2, 39), fixed_r3_dictionary(2, 136)] {
        let error = open(&input).decrypt_to_plain("asdfzxcv").unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
        assert!(error.message.contains("40 to 128 bits"));
    }
    let error = open(&fixed_r3_dictionary(3, 128))
        .decrypt_to_plain("asdfzxcv")
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    assert!(error.message.contains("R=3 with V=3"));
}

#[test]
fn changes_passwords_without_changing_revision_method_or_permissions() {
    let revisions = [
        StandardEncryptionRevision::R2Rc4,
        StandardEncryptionRevision::R3Rc4(128),
        StandardEncryptionRevision::R4Rc4,
        StandardEncryptionRevision::R4AesV2,
        StandardEncryptionRevision::R5Aes256,
        StandardEncryptionRevision::R6Aes256,
    ];
    for revision in revisions {
        let encrypted = open(&pdf("PASSWORD-CHANGE"))
            .encrypt_standard(StandardEncryptionOptions {
                revision,
                user_password: "old-user".into(),
                owner_password: "old-owner".into(),
                permissions: -1028,
            })
            .unwrap();
        assert_eq!(
            open(&encrypted.bytes)
                .change_standard_passwords("wrong", "new-user", "new-owner")
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
        let changed = open(&encrypted.bytes)
            .change_standard_passwords("old-owner", "new-user", "new-owner")
            .unwrap();
        assert_eq!(changed.report.operation, "change_standard_passwords");
        assert_eq!(changed.report.revision, encrypted.report.revision);
        assert_eq!(changed.report.crypt_filter, encrypted.report.crypt_filter);
        assert_eq!(changed.report.permissions, encrypted.report.permissions);
        let changed = open(&changed.bytes);
        assert!(
            changed
                .decrypt_to_plain("new-user")
                .unwrap()
                .verification
                .passed
        );
        assert!(
            changed
                .decrypt_to_plain("new-owner")
                .unwrap()
                .verification
                .passed
        );
        for obsolete in ["old-user", "old-owner", "wrong"] {
            assert_eq!(
                changed.decrypt_to_plain(obsolete).unwrap_err().code,
                PdfErrorCode::UnsafeRewrite
            );
        }
    }
}
