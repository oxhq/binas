use binas_pdf::{
    AnnotationContentsMutationRequest, AppearanceStatus, FormValueMutationRequest, OpenOptions,
    PdfEngine, PdfErrorCode, list_annotations, list_form_fields,
};

fn pdf(objects: &[&str], trailer_extra: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
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

fn fixture_with(
    first_field: &str,
    annotation: &str,
    extra_objects: &[&str],
    trailer_extra: &str,
) -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /Annots [7 0 R] >>",
        "<< /NeedAppearances true /Fields [5 0 R 6 0 R] >>",
        first_field,
        "<< /T (dup) /FT /Tx /V (two) >>",
        annotation,
    ];
    objects.extend_from_slice(extra_objects);
    pdf(&objects, trailer_extra)
}

fn fixture(extra_objects: &[&str], trailer_extra: &str) -> Vec<u8> {
    fixture_with(
        "<< /T (dup) /FT /Tx /V (one) >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old) >>",
        extra_objects,
        trailer_extra,
    )
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

#[test]
fn updates_selected_duplicate_text_field_and_text_annotation() {
    let input = fixture(&[], "");
    let form = open(&input)
        .set_form_field_value(FormValueMutationRequest {
            field_name: "dup".into(),
            value: "second updated".into(),
            match_index: 1,
        })
        .unwrap();
    assert!(form.verification.passed);
    assert_eq!(
        form.report.appearance_status,
        AppearanceStatus::ViewerRegenerationRequired
    );
    assert_eq!(
        list_form_fields(&open(&form.bytes)).unwrap()[1]
            .value
            .as_deref(),
        Some("second updated")
    );

    let annotation = open(&form.bytes)
        .set_annotation_contents(AnnotationContentsMutationRequest {
            annotation_index: 0,
            contents: "Nota ✓".into(),
        })
        .unwrap();
    assert!(annotation.verification.passed);
    assert_eq!(
        annotation.report.appearance_status,
        AppearanceStatus::Absent
    );
    assert_eq!(
        list_annotations(&open(&annotation.bytes)).unwrap()[0]
            .contents
            .as_deref(),
        Some("Nota ✓")
    );
}

#[test]
fn selection_is_zero_based_and_missing_matches_are_typed() {
    let input = fixture(&[], "");
    let first = open(&input)
        .set_form_field_value(FormValueMutationRequest {
            field_name: "dup".into(),
            value: "first updated".into(),
            match_index: 0,
        })
        .unwrap();
    let fields = list_form_fields(&open(&first.bytes)).unwrap();
    assert_eq!(fields[0].value.as_deref(), Some("first updated"));
    assert_eq!(fields[1].value.as_deref(), Some("two"));

    let error = open(&input)
        .set_form_field_value(FormValueMutationRequest {
            field_name: "dup".into(),
            value: "nope".into(),
            match_index: 2,
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::SelectionNotFound);
}

#[test]
fn refuses_direct_unsupported_and_appearance_owned_objects() {
    let direct_field = pdf(
        &[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R >>",
            "<< /NeedAppearances true /Fields [<< /T (direct) /FT /Tx >>] >>",
        ],
        "",
    );
    let error = open(&direct_field)
        .set_form_field_value(FormValueMutationRequest {
            field_name: "direct".into(),
            value: "x".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);

    let button = fixture_with(
        "<< /T (dup) /FT /Btn /V (one) >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old) >>",
        &[],
        "",
    );
    let error = open(&button)
        .set_form_field_value(FormValueMutationRequest {
            field_name: "dup".into(),
            value: "x".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);

    let field_ap = fixture_with(
        "<< /T (dup) /FT /Tx /V (one) /AP <<>> >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old) >>",
        &[],
        "",
    );
    assert_eq!(
        open(&field_ap)
            .set_form_field_value(FormValueMutationRequest {
                field_name: "dup".into(),
                value: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    let widget_ap = fixture_with(
        "<< /T (dup) /FT /Tx /V (one) /Kids [8 0 R] >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old) >>",
        &["<< /Subtype /Widget /Parent 5 0 R /AP <<>> >>"],
        "",
    );
    assert_eq!(
        open(&widget_ap)
            .set_form_field_value(FormValueMutationRequest {
                field_name: "dup".into(),
                value: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    let no_regeneration = pdf(
        &[
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R >>",
            "<< /Fields [5 0 R] >>",
            "<< /T (field) /FT /Tx >>",
        ],
        "",
    );
    assert_eq!(
        open(&no_regeneration)
            .set_form_field_value(FormValueMutationRequest {
                field_name: "field".into(),
                value: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    let annotation_ap = fixture_with(
        "<< /T (dup) /FT /Tx /V (one) >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old) /AP <<>> >>",
        &[],
        "",
    );
    assert_eq!(
        open(&annotation_ap)
            .set_annotation_contents(AnnotationContentsMutationRequest {
                annotation_index: 0,
                contents: "x".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    let direct_annotation = pdf(
        &[
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /Annots [<< /Subtype /Text /Rect [0 0 1 1] >>] >>",
        ],
        "",
    );
    assert_eq!(
        open(&direct_annotation)
            .set_annotation_contents(AnnotationContentsMutationRequest {
                annotation_index: 0,
                contents: "x".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}

#[test]
fn malformed_encrypted_and_signed_inputs_fail_closed() {
    let malformed = fixture_with(
        "<< /T (dup) /FT /Tx /V (one) >>",
        "<< /Type /Annot /Subtype 12 /Rect [0 0 10 10] /Contents (old) >>",
        &[],
        "",
    );
    assert_eq!(
        open(&malformed)
            .set_annotation_contents(AnnotationContentsMutationRequest {
                annotation_index: 0,
                contents: "x".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );

    let encrypted = fixture(&["<< /Filter /Standard >>"], "/Encrypt 8 0 R");
    assert_eq!(
        open(&encrypted)
            .set_form_field_value(FormValueMutationRequest {
                field_name: "dup".into(),
                value: "x".into(),
                match_index: 0,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );

    let signed = fixture(
        &["<< /Type /Sig /ByteRange [0 1 2 3] /Contents (signature) >>"],
        "",
    );
    assert_eq!(
        open(&signed)
            .set_annotation_contents(AnnotationContentsMutationRequest {
                annotation_index: 0,
                contents: "x".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}
