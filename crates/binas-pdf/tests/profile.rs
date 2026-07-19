use binas_pdf::{
    CapabilityDecision, OpenOptions, PdfEngine, StandardEncryptionOptions,
    StandardEncryptionRevision,
};

fn pdf() -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 7 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Annots [5 0 R 6 0 R] >>".to_vec(),
        b"null".to_vec(),
        b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (SECRET-FIELD) /Rect [0 0 10 10] /P 3 0 R >>"
            .to_vec(),
        b"<< /Type /Annot /Subtype /Text /Rect [10 10 20 20] /Contents (SECRET-ANNOTATION) >>"
            .to_vec(),
        b"<< /Fields [5 0 R] >>".to_vec(),
        b"<< /Filter [/FlateDecode /ASCII85Decode] >>".to_vec(),
    ];
    classic(&objects, 9)
}

fn plain_pdf() -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
    ];
    classic(&objects, 4)
}

fn dynamic_xfa_pdf() -> Vec<u8> {
    let config = b"<config><dynamicRender>required</dynamicRender></config>";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /XFA [(config) 5 0 R] >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", config.len()).into_bytes(),
            config.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];
    classic(&objects, 6)
}

fn classic(objects: &[Vec<u8>], size: usize) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

#[test]
fn reports_stable_bounded_secret_free_capabilities() {
    let profile = open(&pdf()).capability_profile().unwrap();
    assert_eq!(profile.profile_version, 1);
    assert_eq!(profile.page_count, 1);
    assert_eq!(profile.form_field_count, 1);
    assert_eq!(profile.annotation_count, 2);
    assert_eq!(
        profile.filter_names,
        vec!["ASCII85Decode".to_owned(), "FlateDecode".to_owned()]
    );
    assert_eq!(
        profile.operation("canonicalize").unwrap().decision,
        CapabilityDecision::Conditional
    );
    assert_eq!(
        profile.operation("external_signing").unwrap().decision,
        CapabilityDecision::Supported
    );
    let json = serde_json::to_string(&profile).unwrap();
    assert!(!json.contains("SECRET-FIELD"));
    assert!(!json.contains("SECRET-ANNOTATION"));
}

#[test]
fn reports_encryption_signatures_and_dynamic_xfa_as_operation_boundaries() {
    let document = open(&plain_pdf());
    let encrypted = document
        .encrypt_standard(StandardEncryptionOptions {
            revision: StandardEncryptionRevision::R6Aes256,
            user_password: "SECRET-PASSWORD".into(),
            owner_password: "SECRET-OWNER".into(),
            permissions: -4,
        })
        .unwrap();
    let encrypted_profile = open(&encrypted.bytes).capability_profile().unwrap();
    assert_eq!(
        encrypted_profile
            .operation("external_signing")
            .unwrap()
            .decision,
        CapabilityDecision::Refused
    );
    assert_eq!(
        encrypted_profile.operation("decrypt").unwrap().decision,
        CapabilityDecision::Supported
    );
    let json = serde_json::to_string(&encrypted_profile).unwrap();
    assert!(!json.contains("SECRET-PASSWORD"));
    assert!(!json.contains("SECRET-OWNER"));

    let signed = document.prepare_external_signature(1024).unwrap();
    let signed_profile = open(&signed.bytes).capability_profile().unwrap();
    assert_eq!(signed_profile.signature_count, 1);
    assert_eq!(
        signed_profile.operation("canonicalize").unwrap().decision,
        CapabilityDecision::Refused
    );

    let xfa_profile = open(&dynamic_xfa_pdf()).capability_profile().unwrap();
    assert!(xfa_profile.xfa_dynamic);
    assert_eq!(
        xfa_profile.operation("ocr_text_layer").unwrap().decision,
        CapabilityDecision::Refused
    );
    assert_eq!(
        xfa_profile.operation("form_lifecycle").unwrap().decision,
        CapabilityDecision::Refused
    );
}
