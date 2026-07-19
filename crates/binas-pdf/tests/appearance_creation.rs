use binas_pdf::{
    AppearanceStatus, EngineConfig, FreeTextAppearanceRequest, Limits, OpenOptions, PdfEngine,
    PdfErrorCode, TextFieldAppearanceRequest, list_annotations, list_form_fields,
};

fn pdf(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
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
            "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn text_field(dr: &str, widget_extra: &str, extra: &[&str], trailer: &str) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Annots [6 0 R] >>".to_vec(),
        format!("<< /DA (/Helv 12 Tf) /DR {dr} /Fields [5 0 R] >>").into_bytes(),
        b"<< /T (name) /FT /Tx /V (Old) /Kids [6 0 R] >>".to_vec(),
        format!(
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [10 20 110 40] {widget_extra} >>"
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    objects.extend(extra.iter().map(|value| value.as_bytes().to_vec()));
    pdf(&objects, trailer)
}

fn free_text(da: &str, annotation_extra: &str, with_appearance: bool) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>".to_vec(),
        format!(
            "<< /Type /Annot /Subtype /FreeText /Rect [0 0 120 24] /DA ({da}) /Contents (Old) {} {annotation_extra} >>",
            if with_appearance { "/AP << /N 6 0 R >>" } else { "" }
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    if with_appearance {
        let stream = b"q\nBT\n/F1 10 Tf\n(Old) Tj\nET\nQ\n";
        let mut appearance = format!(
            "<< /Type /XObject /Subtype /Form /BBox [0 0 120 24] /Resources << /Font << /F1 5 0 R >> >> /Length {} >>\nstream\n",
            stream.len()
        )
        .into_bytes();
        appearance.extend_from_slice(stream);
        appearance.extend_from_slice(b"endstream");
        objects.push(appearance);
    }
    pdf(&objects, "")
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

#[test]
fn creates_missing_text_field_appearance_atomically() {
    let input = text_field("<< /Font << /Helv 7 0 R >> >>", "", &[], "");
    let outcome = open(&input)
        .regenerate_text_field_appearance(TextFieldAppearanceRequest {
            field_name: "name".into(),
            value: "Created".into(),
            match_index: 0,
        })
        .unwrap();
    assert_eq!(outcome.report.appearance_status, AppearanceStatus::Created);
    assert_eq!(outcome.report.appearance_object_number, 8);
    assert!(outcome.verification.passed);
    assert!(outcome.verification.appearance_reachable);
    assert!(outcome.verification.no_dangling_references);
    assert_eq!(
        list_form_fields(&open(&outcome.bytes)).unwrap()[0]
            .value
            .as_deref(),
        Some("Created")
    );
    assert_eq!(open(&outcome.bytes).inspect().unwrap().object_count, 8);
}

#[test]
fn creates_then_updates_free_text_appearance() {
    let input = free_text("/F1 10 Tf", "", false);
    let created = open(&input)
        .regenerate_free_text_appearance(FreeTextAppearanceRequest {
            annotation_index: 0,
            contents: "Created note".into(),
        })
        .unwrap();
    assert_eq!(created.report.appearance_status, AppearanceStatus::Created);
    assert_eq!(created.report.appearance_object_number, 6);
    assert!(created.verification.passed);
    assert_eq!(
        list_annotations(&open(&created.bytes)).unwrap()[0]
            .contents
            .as_deref(),
        Some("Created note")
    );

    let existing = free_text("/F1 10 Tf", "", true);
    let updated = open(&existing)
        .regenerate_free_text_appearance(FreeTextAppearanceRequest {
            annotation_index: 0,
            contents: "Updated note".into(),
        })
        .unwrap();
    assert_eq!(
        updated.report.appearance_status,
        AppearanceStatus::Regenerated
    );
    assert_eq!(updated.report.appearance_object_number, 6);
    assert!(updated.verification.passed);
}

#[test]
fn creation_refuses_missing_fonts_malformed_modes_and_security() {
    let missing_font = text_field("<< /Font << /Other 7 0 R >> >>", "", &[], "");
    assert_eq!(
        open(&missing_font)
            .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                field_name: "name".into(),
                value: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    for input in [
        free_text("/F1 0 Tf", "", false),
        free_text("/F1 10 Tf", "/Rotate 90", false),
        free_text("/F1 10 Tf", "/RC (rich)", false),
    ] {
        assert_eq!(
            open(&input)
                .regenerate_free_text_appearance(FreeTextAppearanceRequest {
                    annotation_index: 0,
                    contents: "x".into(),
                })
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
    }

    let encrypted = text_field(
        "<< /Font << /Helv 7 0 R >> >>",
        "",
        &["<< /Filter /Standard >>"],
        "/Encrypt 8 0 R",
    );
    assert_eq!(
        open(&encrypted)
            .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                field_name: "name".into(),
                value: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
    let signed = text_field(
        "<< /Font << /Helv 7 0 R >> >>",
        "",
        &["<< /Type /Sig /ByteRange [0 1 2 3] >>"],
        "",
    );
    assert_eq!(
        open(&signed)
            .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                field_name: "name".into(),
                value: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}

#[test]
fn new_object_allocation_obeys_xref_limit() {
    let input = free_text("/F1 10 Tf", "", false);
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_xref_entries: 6,
            ..Limits::default()
        },
    })
    .open(&input, OpenOptions::default())
    .unwrap();
    assert_eq!(
        document
            .regenerate_free_text_appearance(FreeTextAppearanceRequest {
                annotation_index: 0,
                contents: "x".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
}
