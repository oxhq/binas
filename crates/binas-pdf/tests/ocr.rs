use binas_pdf::{
    OcrParseLimits, OcrTextBox, OcrTextLayerRequest, OpenOptions, PdfEngine, PdfErrorCode,
    StandardEncryptionOptions, StandardEncryptionRevision, parse_alto_xml, parse_ocr_json,
};

fn pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>",
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

fn dynamic_xfa_pdf() -> Vec<u8> {
    let config = b"<config><dynamicRender>required</dynamicRender></config>";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /XFA [(config) 5 0 R] >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", config.len()).into_bytes(),
            config.to_vec(),
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
    bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn request() -> OcrTextLayerRequest {
    OcrTextLayerRequest {
        page_index: 0,
        source_width: 1200.0,
        source_height: 1600.0,
        boxes: vec![OcrTextBox {
            text: "Selectable OCR".into(),
            x: 120.0,
            y: 160.0,
            width: 240.0,
            height: 80.0,
        }],
    }
}

#[test]
fn plans_transforms_and_applies_an_invisible_selectable_layer() {
    let document = open(&pdf());
    let plan = document.plan_ocr_text_layer(request()).unwrap();
    assert_eq!(plan.boxes[0].x, 60.0);
    assert_eq!(plan.boxes[0].y, 680.0);
    assert_eq!(plan.boxes[0].width, 120.0);
    assert_eq!(plan.boxes[0].height, 40.0);
    let outcome = document.apply_ocr_text_layer(&plan).unwrap();
    assert!(outcome.verification.passed);
    assert!(outcome.verification.text_selectable);
    assert!(outcome.verification.no_dangling_references);
    assert!(outcome.bytes.windows(4).any(|window| window == b"3 Tr"));
    assert_eq!(
        open(&outcome.bytes)
            .query_text_all("Selectable OCR")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn parses_bounded_json_and_alto_without_recognition() {
    let limits = OcrParseLimits {
        max_input_bytes: 4096,
        max_boxes: 4,
        max_text_bytes: 64,
    };
    let json = serde_json::to_vec(&request()).unwrap();
    assert_eq!(parse_ocr_json(&json, limits).unwrap(), request());

    let alto = br#"<alto><Layout><Page WIDTH="1200" HEIGHT="1600"><PrintSpace><TextBlock><TextLine><String CONTENT="Hello &amp; OCR" HPOS="10" VPOS="20" WIDTH="30" HEIGHT="40"/></TextLine></TextBlock></PrintSpace></Page></Layout></alto>"#;
    let pages = parse_alto_xml(alto, limits).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].boxes[0].text, "Hello & OCR");
    assert_eq!(pages[0].boxes[0].x, 10.0);

    assert_eq!(
        parse_ocr_json(
            &json,
            OcrParseLimits {
                max_input_bytes: 1,
                ..limits
            }
        )
        .unwrap_err()
        .code,
        PdfErrorCode::ResourceLimit
    );
    let too_many = br#"<alto><Page WIDTH="1" HEIGHT="1"><String CONTENT="a" HPOS="0" VPOS="0" WIDTH="1" HEIGHT="1"/><String CONTENT="b" HPOS="0" VPOS="0" WIDTH="1" HEIGHT="1"/></Page></alto>"#;
    assert_eq!(
        parse_alto_xml(
            too_many,
            OcrParseLimits {
                max_boxes: 1,
                ..limits
            }
        )
        .unwrap_err()
        .code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn refuses_encrypted_signed_and_mismatched_plan_inputs() {
    let document = open(&pdf());
    let plan = document.plan_ocr_text_layer(request()).unwrap();
    let mut changed = pdf();
    changed[7] = b'6';
    assert_eq!(
        open(&changed).apply_ocr_text_layer(&plan).unwrap_err().code,
        PdfErrorCode::VerificationFailed
    );

    let encrypted = document
        .encrypt_standard(StandardEncryptionOptions {
            revision: StandardEncryptionRevision::R4AesV2,
            user_password: "user".into(),
            owner_password: "owner".into(),
            permissions: -4,
        })
        .unwrap();
    assert_eq!(
        open(&encrypted.bytes)
            .plan_ocr_text_layer(request())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    let signed = document.prepare_external_signature(1024).unwrap();
    assert_eq!(
        open(&signed.bytes)
            .plan_ocr_text_layer(request())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    assert_eq!(
        open(&dynamic_xfa_pdf())
            .plan_ocr_text_layer(request())
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}
