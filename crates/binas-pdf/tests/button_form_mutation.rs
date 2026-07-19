use binas_pdf::{
    ButtonChoiceMutationRequest, CheckboxFieldMutationRequest, OpenOptions, PdfEngine,
    PdfErrorCode, list_form_fields,
};

fn pdf(objects: &[&str]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

fn form_stream() -> &'static str {
    "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length 0 >>\nstream\n\nendstream"
}

fn checkbox() -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Fields [5 0 R] >>",
        "<< /FT /Btn /T (consent) /Kids [6 0 R] /V /Off >>",
        "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /AP << /N << /Off 7 0 R /Yes 8 0 R >> >> /AS /Off >>",
        form_stream(),
        form_stream(),
    ])
}

fn radio(first_state: &str, second_state: &str, extra_appearance: &str) -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        "<< /Fields [5 0 R] >>",
        "<< /FT /Btn /Ff 32768 /T (plan) /Kids [6 0 R 7 0 R] /V /Off >>",
        &format!(
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /AP << /N << /Off 8 0 R /{first_state} 9 0 R >> {extra_appearance} >> /AS /Off >>"
        ),
        &format!(
            "<< /Type /Annot /Subtype /Widget /Parent 5 0 R /AP << /N << /Off 10 0 R /{second_state} 11 0 R >> >> /AS /Off >>"
        ),
        form_stream(),
        form_stream(),
        form_stream(),
        form_stream(),
    ])
}

#[test]
fn checkbox_and_radio_mutations_keep_v_and_widget_as_in_sync() {
    let input = checkbox();
    let checked = open(&input)
        .set_checkbox_field(CheckboxFieldMutationRequest {
            field_name: "consent".into(),
            checked: true,
            match_index: 0,
        })
        .unwrap();
    assert!(checked.verification.passed);
    assert_eq!(checked.report.selected_state, "Yes");
    assert_eq!(
        list_form_fields(&open(&checked.bytes)).unwrap()[0]
            .value
            .as_deref(),
        Some("Yes")
    );
    assert!(
        checked.bytes[input.len()..]
            .windows(b"/AS /Yes".len())
            .any(|value| value == b"/AS /Yes")
    );

    let unchecked = open(&checked.bytes)
        .set_checkbox_field(CheckboxFieldMutationRequest {
            field_name: "consent".into(),
            checked: false,
            match_index: 0,
        })
        .unwrap();
    assert!(unchecked.verification.passed);
    assert_eq!(
        list_form_fields(&open(&unchecked.bytes)).unwrap()[0]
            .value
            .as_deref(),
        Some("Off")
    );

    let input = radio("Basic", "Pro", "");
    let choice = open(&input)
        .set_button_field_choice(ButtonChoiceMutationRequest {
            field_name: "plan".into(),
            state: "Pro".into(),
            match_index: 0,
        })
        .unwrap();
    assert!(choice.verification.passed);
    assert_eq!(choice.report.widgets_affected, 2);
    assert_eq!(
        list_form_fields(&open(&choice.bytes)).unwrap()[0]
            .value
            .as_deref(),
        Some("Pro")
    );
    let appended = &choice.bytes[input.len()..];
    assert!(
        appended
            .windows(b"/AS /Off".len())
            .any(|value| value == b"/AS /Off")
    );
    assert!(
        appended
            .windows(b"/AS /Pro".len())
            .any(|value| value == b"/AS /Pro")
    );
}

#[test]
fn button_mutations_fail_closed_for_non_checkbox_non_radio_and_ambiguous_appearances() {
    let checkbox_error = open(&checkbox())
        .set_button_field_choice(ButtonChoiceMutationRequest {
            field_name: "consent".into(),
            state: "Yes".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(checkbox_error.code, PdfErrorCode::UnsafeRewrite);

    let radio_error = open(&radio("Basic", "Pro", ""))
        .set_checkbox_field(CheckboxFieldMutationRequest {
            field_name: "plan".into(),
            checked: true,
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(radio_error.code, PdfErrorCode::UnsafeRewrite);

    let duplicate_state = open(&radio("Pro", "Pro", ""))
        .set_button_field_choice(ButtonChoiceMutationRequest {
            field_name: "plan".into(),
            state: "Pro".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(duplicate_state.code, PdfErrorCode::UnsafeRewrite);

    let rich_appearance = open(&radio("Basic", "Pro", "/D << /Off 8 0 R /Basic 9 0 R >>"))
        .set_button_field_choice(ButtonChoiceMutationRequest {
            field_name: "plan".into(),
            state: "Pro".into(),
            match_index: 0,
        })
        .unwrap_err();
    assert_eq!(rich_appearance.code, PdfErrorCode::UnsafeRewrite);
}
