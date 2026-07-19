use binas_pdf::{
    AppearanceStatus, OpenOptions, PdfEngine, PdfErrorCode, TextFieldAppearanceRequest,
    list_form_fields,
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

#[allow(clippy::too_many_arguments)]
fn fixture(
    da: &str,
    field_kids: &str,
    field_extra: &str,
    widget_extra: &str,
    appearance_extra: &str,
    resources: &str,
    extra_objects: &[&str],
    trailer_extra: &str,
) -> Vec<u8> {
    let old_appearance = b"q\nBT\n/Helv 12 Tf\n(Old) Tj\nET\nQ\n";
    let mut appearance = format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 100 20] /Resources {resources} /Length {} {appearance_extra} >>\nstream\n",
        old_appearance.len()
    )
    .into_bytes();
    appearance.extend_from_slice(old_appearance);
    appearance.extend_from_slice(b"endstream");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Annots [6 0 R] >>".to_vec(),
        format!("<< /DA ({da}) /Fields [5 0 R] >>").into_bytes(),
        format!("<< /T (name) /FT /Tx /V (Old) /Kids [{field_kids}] {field_extra} >>")
            .into_bytes(),
        format!(
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /Rect [0 0 100 20] /AP << /N 7 0 R >> {widget_extra} >>"
        )
        .into_bytes(),
        appearance,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ];
    objects.extend(extra_objects.iter().map(|value| value.as_bytes().to_vec()));
    pdf(&objects, trailer_extra)
}

fn normal() -> Vec<u8> {
    fixture(
        "/Helv 12 Tf 0 g",
        "6 0 R",
        "",
        "",
        "",
        "<< /Font << /Helv 8 0 R >> >>",
        &[],
        "",
    )
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

#[test]
fn atomically_updates_field_and_existing_appearance_stream() {
    let input = normal();
    let outcome = open(&input)
        .regenerate_text_field_appearance(TextFieldAppearanceRequest {
            field_name: "name".into(),
            value: r"New (safe) \ value".into(),
            match_index: 0,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(
        outcome.report.appearance_status,
        AppearanceStatus::Regenerated
    );
    assert_eq!(outcome.report.font_name, "Helv");
    assert_eq!(outcome.report.font_size, 12.0);
    assert!(outcome.bytes.starts_with(&input));
    assert_eq!(
        list_form_fields(&open(&outcome.bytes)).unwrap()[0]
            .value
            .as_deref(),
        Some(r"New (safe) \ value")
    );
    assert_eq!(open(&outcome.bytes).inspect().unwrap().xref_revisions, 2);
}

#[test]
fn refuses_autosize_rich_text_rotation_and_multiple_widgets() {
    let cases = [
        fixture(
            "/Helv 0 Tf",
            "6 0 R",
            "",
            "",
            "",
            "<< /Font << /Helv 8 0 R >> >>",
            &[],
            "",
        ),
        fixture(
            "/Helv 12 Tf",
            "6 0 R",
            "/RV (rich)",
            "",
            "",
            "<< /Font << /Helv 8 0 R >> >>",
            &[],
            "",
        ),
        fixture(
            "/Helv 12 Tf",
            "6 0 R",
            "",
            "/MK << /R 90 >>",
            "",
            "<< /Font << /Helv 8 0 R >> >>",
            &[],
            "",
        ),
        fixture(
            "/Helv 12 Tf",
            "6 0 R 9 0 R",
            "",
            "",
            "",
            "<< /Font << /Helv 8 0 R >> >>",
            &["<< /Subtype /Widget /Parent 5 0 R /Rect [0 0 10 10] >>"],
            "",
        ),
    ];
    for input in cases {
        let error = open(&input)
            .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                field_name: "name".into(),
                value: "new".into(),
                match_index: 0,
            })
            .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
    }
}

#[test]
fn refuses_filters_missing_fonts_transforms_and_non_ascii() {
    let cases = [
        fixture(
            "/Helv 12 Tf",
            "6 0 R",
            "",
            "",
            "/Filter /FlateDecode",
            "<< /Font << /Helv 8 0 R >> >>",
            &[],
            "",
        ),
        fixture(
            "/Helv 12 Tf",
            "6 0 R",
            "",
            "",
            "",
            "<< /Font << /Other 8 0 R >> >>",
            &[],
            "",
        ),
        fixture(
            "/Helv 12 Tf",
            "6 0 R",
            "",
            "",
            "/Matrix [0 1 -1 0 0 0]",
            "<< /Font << /Helv 8 0 R >> >>",
            &[],
            "",
        ),
    ];
    for input in cases {
        assert_eq!(
            open(&input)
                .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                    field_name: "name".into(),
                    value: "new".into(),
                    match_index: 0,
                })
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
    }
    assert_eq!(
        open(&normal())
            .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                field_name: "name".into(),
                value: "café".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}

#[test]
fn signed_and_encrypted_inputs_fail_closed() {
    let signed = fixture(
        "/Helv 12 Tf",
        "6 0 R",
        "",
        "",
        "",
        "<< /Font << /Helv 8 0 R >> >>",
        &["<< /Type /Sig /ByteRange [0 1 2 3] >>"],
        "",
    );
    let encrypted = fixture(
        "/Helv 12 Tf",
        "6 0 R",
        "",
        "",
        "",
        "<< /Font << /Helv 8 0 R >> >>",
        &["<< /Filter /Standard >>"],
        "/Encrypt 9 0 R",
    );
    for input in [signed, encrypted] {
        assert_eq!(
            open(&input)
                .regenerate_text_field_appearance(TextFieldAppearanceRequest {
                    field_name: "name".into(),
                    value: "new".into(),
                    match_index: 0,
                })
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
    }
}
