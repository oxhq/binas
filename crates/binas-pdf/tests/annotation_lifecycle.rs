use binas_pdf::{
    AnnotationCreateRequest, AnnotationRemoveRequest, AnnotationSubtype, OpenOptions, PdfEngine,
    PdfErrorCode, list_annotations,
};

fn pdf() -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_vec(),
        b"<< /Length 5 >>\nstream\nBT ET\nendstream".to_vec(),
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

fn create(bytes: Vec<u8>, subtype: AnnotationSubtype, y: f64) -> Vec<u8> {
    let rect = [40.0, y, 180.0, y + 24.0];
    open(&bytes)
        .create_annotation(AnnotationCreateRequest {
            page_index: 0,
            subtype,
            rect,
            contents: format!("{subtype:?}"),
            quad_points: if matches!(
                subtype,
                AnnotationSubtype::Highlight
                    | AnnotationSubtype::Underline
                    | AnnotationSubtype::StrikeOut
            ) {
                vec![
                    45.0,
                    y + 20.0,
                    170.0,
                    y + 20.0,
                    45.0,
                    y + 4.0,
                    170.0,
                    y + 4.0,
                ]
            } else {
                Vec::new()
            },
            uri: if subtype == AnnotationSubtype::Link {
                "https://example.test/".into()
            } else {
                String::new()
            },
        })
        .unwrap()
        .bytes
}

#[test]
fn creates_common_annotation_subtypes_with_reachable_appearances() {
    let subtypes = [
        AnnotationSubtype::Text,
        AnnotationSubtype::FreeText,
        AnnotationSubtype::Square,
        AnnotationSubtype::Circle,
        AnnotationSubtype::Link,
        AnnotationSubtype::Highlight,
        AnnotationSubtype::Underline,
        AnnotationSubtype::StrikeOut,
    ];
    let mut bytes = pdf();
    for (index, subtype) in subtypes.into_iter().enumerate() {
        bytes = create(bytes, subtype, 740.0 - index as f64 * 40.0);
    }
    let annotations = list_annotations(&open(&bytes)).unwrap();
    assert_eq!(annotations.len(), subtypes.len());
    assert_eq!(
        annotations
            .iter()
            .map(|value| value.subtype.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Text",
            "FreeText",
            "Square",
            "Circle",
            "Link",
            "Highlight",
            "Underline",
            "StrikeOut"
        ]
    );
    assert_eq!(
        bytes
            .windows(b"/Type /XObject".len())
            .filter(|value| *value == b"/Type /XObject")
            .count(),
        8
    );
}

#[test]
fn removes_only_the_selected_annotation_and_its_appearance() {
    let bytes = create(pdf(), AnnotationSubtype::FreeText, 700.0);
    let bytes = create(bytes, AnnotationSubtype::Square, 650.0);
    let outcome = open(&bytes)
        .remove_annotation(AnnotationRemoveRequest {
            annotation_index: 0,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    let annotations = list_annotations(&open(&outcome.bytes)).unwrap();
    assert_eq!(annotations.len(), 1);
    assert_eq!(annotations[0].subtype, "Square");
    assert!(
        !outcome
            .bytes
            .windows(b"/Helvetica".len())
            .any(|value| value == b"/Helvetica")
    );
}

#[test]
fn rejects_malformed_markup_and_non_link_uri_without_writing() {
    let error = open(&pdf())
        .create_annotation(AnnotationCreateRequest {
            page_index: 0,
            subtype: AnnotationSubtype::Highlight,
            rect: [0.0, 0.0, 100.0, 20.0],
            contents: String::new(),
            quad_points: vec![0.0, 0.0, 10.0, 10.0],
            uri: String::new(),
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);

    let error = open(&pdf())
        .create_annotation(AnnotationCreateRequest {
            page_index: 0,
            subtype: AnnotationSubtype::Circle,
            rect: [0.0, 0.0, 100.0, 20.0],
            contents: String::new(),
            quad_points: Vec::new(),
            uri: "https://example.test/".into(),
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
}
