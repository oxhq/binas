use binas_pdf::{
    FormFieldCreateRequest, FormFieldKind, FormFieldRemoveRequest, OpenOptions, PdfEngine,
    list_form_fields,
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

fn create(bytes: Vec<u8>, name: &str, kind: FormFieldKind, y: f64) -> Vec<u8> {
    open(&bytes)
        .create_form_field(FormFieldCreateRequest {
            name: name.into(),
            page_index: 0,
            rect: [40.0, y, 240.0, y + 24.0],
            kind,
            value: match kind {
                FormFieldKind::Text => "Text value".into(),
                FormFieldKind::Choice => "Two".into(),
                _ => String::new(),
            },
            options: if kind == FormFieldKind::Choice {
                vec!["One".into(), "Two".into()]
            } else {
                Vec::new()
            },
        })
        .unwrap()
        .bytes
}

#[test]
fn creates_every_supported_terminal_field_with_reachable_widget_appearance() {
    let mut bytes = pdf();
    for (index, kind) in [
        FormFieldKind::Text,
        FormFieldKind::Checkbox,
        FormFieldKind::Radio,
        FormFieldKind::Choice,
        FormFieldKind::Signature,
    ]
    .into_iter()
    .enumerate()
    {
        bytes = create(
            bytes,
            &format!("field-{index}"),
            kind,
            700.0 - index as f64 * 40.0,
        );
    }
    let fields = list_form_fields(&open(&bytes)).unwrap();
    assert_eq!(fields.len(), 5);
    assert!(fields.iter().all(|field| field.widget_refs.len() == 1));
    assert_eq!(
        fields
            .iter()
            .map(|field| field.field_type.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("Tx"),
            Some("Btn"),
            Some("Btn"),
            Some("Ch"),
            Some("Sig")
        ]
    );
}

#[test]
fn removes_only_the_selected_duplicate_named_field_and_widget() {
    let bytes = create(pdf(), "duplicate", FormFieldKind::Text, 700.0);
    let bytes = create(bytes, "duplicate", FormFieldKind::Checkbox, 650.0);
    let before = list_form_fields(&open(&bytes)).unwrap();
    let removed_widget = before[1].widget_refs[0].clone();
    let outcome = open(&bytes)
        .remove_form_field(FormFieldRemoveRequest {
            field_name: "duplicate".into(),
            match_index: 1,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    let remaining = list_form_fields(&open(&outcome.bytes)).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].field_type.as_deref(), Some("Tx"));
    assert_ne!(remaining[0].widget_refs[0], removed_widget);
}

#[test]
fn flattens_verified_appearances_into_page_content_and_removes_form_tree() {
    let bytes = create(pdf(), "text", FormFieldKind::Text, 700.0);
    let bytes = create(bytes, "check", FormFieldKind::Checkbox, 650.0);
    let outcome = open(&bytes).flatten_form_fields().unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(outcome.report.appearances_placed, 2);
    assert!(list_form_fields(&open(&outcome.bytes)).unwrap().is_empty());
    assert!(!outcome.bytes.windows(9).any(|value| value == b"/AcroForm"));
    assert!(outcome.bytes.windows(3).any(|value| value == b" Do"));
}
