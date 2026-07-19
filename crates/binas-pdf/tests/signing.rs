use binas_pdf::{
    DigestMatchStatus, ExternalSignatureFieldOptions, FormFieldCreateRequest, FormFieldKind,
    OpenOptions, PdfEngine, PdfErrorCode, SignatureCryptoStatus, list_form_fields,
};
use cms::{
    cert::{CertificateChoices, IssuerAndSerialNumber},
    content_info::{CmsVersion, ContentInfo},
    signed_data::{
        CertificateSet, DigestAlgorithmIdentifiers, EncapsulatedContentInfo, SignatureValue,
        SignedAttributes, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
    },
};
use der::{
    Any, AnyRef, Encode,
    asn1::{ObjectIdentifier, OctetString, SetOfVec},
};
use rcgen::{CertificateParams, DnType, KeyPair, PKCS_ECDSA_P256_SHA256, SigningKey};
use x509_cert::{Certificate, attr::Attribute, der::Decode};

const ID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ID_SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ID_CONTENT_TYPE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
const ID_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ID_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const ECDSA_WITH_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");

fn pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R >>",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

fn detached_cms(digest: &[u8]) -> Vec<u8> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
    let mut params = CertificateParams::default();
    params
        .distinguished_name
        .push(DnType::CommonName, "Binas External Signer");
    let certificate_der = params.self_signed(&key).unwrap();
    let certificate = Certificate::from_der(certificate_der.der()).unwrap();
    let digest_algorithm = algorithm(ID_SHA256);
    let attributes = SignedAttributes::try_from(vec![
        Attribute {
            oid: ID_CONTENT_TYPE,
            values: SetOfVec::try_from(vec![Any::encode_from(&ID_DATA).unwrap()]).unwrap(),
        },
        Attribute {
            oid: ID_MESSAGE_DIGEST,
            values: SetOfVec::try_from(vec![
                Any::encode_from(&OctetString::new(digest).unwrap()).unwrap(),
            ])
            .unwrap(),
        },
    ])
    .unwrap();
    let signature = key.sign(&attributes.to_der().unwrap()).unwrap();
    let signer = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: certificate.tbs_certificate.issuer.clone(),
            serial_number: certificate.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: digest_algorithm.clone(),
        signed_attrs: Some(attributes),
        signature_algorithm: algorithm(ECDSA_WITH_SHA256),
        signature: SignatureValue::new(signature).unwrap(),
        unsigned_attrs: None,
    };
    let signed_data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: DigestAlgorithmIdentifiers::try_from(vec![digest_algorithm]).unwrap(),
        encap_content_info: EncapsulatedContentInfo {
            econtent_type: ID_DATA,
            econtent: None,
        },
        certificates: Some(
            CertificateSet::try_from(vec![CertificateChoices::Certificate(certificate)]).unwrap(),
        ),
        crls: None,
        signer_infos: SignerInfos::try_from(vec![signer]).unwrap(),
    };
    let signed_der = signed_data.to_der().unwrap();
    ContentInfo {
        content_type: ID_SIGNED_DATA,
        content: Any::from(AnyRef::try_from(signed_der.as_slice()).unwrap()),
    }
    .to_der()
    .unwrap()
}

fn algorithm(oid: ObjectIdentifier) -> x509_cert::spki::AlgorithmIdentifierOwned {
    x509_cert::spki::AlgorithmIdentifierOwned {
        oid,
        parameters: None,
    }
}

#[test]
fn reserves_deterministically_and_applies_only_the_matching_cms() {
    let document = open(&pdf());
    let plan = document.prepare_external_signature(2048).unwrap();
    let repeated = document.prepare_external_signature(2048).unwrap();
    assert_eq!(plan.bytes, repeated.bytes);
    assert_eq!(plan.byte_range, repeated.byte_range);
    assert_eq!(plan.digest_to_sign, repeated.digest_to_sign);
    let fields = list_form_fields(&open(&plan.bytes)).unwrap();
    assert!(fields.iter().any(|field| {
        field.object_number == Some(plan.field_object_number)
            && field.field_type.as_deref() == Some("Sig")
            && !field.widget_refs.is_empty()
    }));

    let descriptor_json = serde_json::to_vec(&plan.descriptor()).unwrap();
    let descriptor = serde_json::from_slice(&descriptor_json).unwrap();
    let reloaded =
        binas_pdf::ExternalSignaturePlan::from_prepared_pdf(plan.bytes.clone(), descriptor)
            .unwrap();
    assert_eq!(reloaded.digest_to_sign, plan.digest_to_sign);
    assert_eq!(reloaded.byte_range, plan.byte_range);

    let cms = detached_cms(&reloaded.digest_to_sign);
    let applied = reloaded.apply_cms(&cms).unwrap();
    assert!(applied.inspection.cms_verified);
    assert!(applied.inspection.covers_current_file);
    assert_eq!(applied.inspection.byte_range, plan.byte_range);
    assert_eq!(
        applied.inspection.cms.digest_status,
        DigestMatchStatus::Match
    );
    assert_eq!(
        applied.inspection.cms.signature_status,
        SignatureCryptoStatus::Valid
    );

    let wrong = detached_cms(&[0; 32]);
    assert_eq!(
        plan.apply_cms(&wrong).unwrap_err().code,
        PdfErrorCode::VerificationFailed
    );
    assert_eq!(
        plan.apply_cms(&vec![1; plan.reserved_cms_bytes + 1])
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    let mut modified = plan.clone();
    modified.bytes[0] ^= 1;
    assert_eq!(
        modified.apply_cms(&cms).unwrap_err().code,
        PdfErrorCode::VerificationFailed
    );
    let mut wrong_descriptor = plan.descriptor();
    wrong_descriptor.digest_to_sign[0] ^= 1;
    assert_eq!(
        binas_pdf::ExternalSignaturePlan::from_prepared_pdf(plan.bytes.clone(), wrong_descriptor,)
            .unwrap_err()
            .code,
        PdfErrorCode::VerificationFailed
    );
}

#[test]
fn reuses_a_uniquely_selected_reachable_signature_field() {
    let created = open(&pdf())
        .create_form_field(FormFieldCreateRequest {
            name: "Approval".into(),
            page_index: 0,
            rect: [10.0, 10.0, 100.0, 40.0],
            kind: FormFieldKind::Signature,
            value: String::new(),
            options: vec![],
        })
        .unwrap();
    let document = open(&created.bytes);
    let existing = list_form_fields(&document).unwrap().remove(0);
    let plan = document
        .prepare_external_signature_with_field(
            2048,
            ExternalSignatureFieldOptions {
                field_name: Some("Approval".into()),
                page_index: 0,
                rect: [0.0; 4],
            },
        )
        .unwrap();
    assert_eq!(Some(plan.field_object_number), existing.object_number);
    let applied = plan.apply_cms(&detached_cms(&plan.digest_to_sign)).unwrap();
    assert!(applied.inspection.cms_verified);
}
