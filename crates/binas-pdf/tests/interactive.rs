use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode, list_annotations, list_form_fields,
};

fn pdf(objects: &[&str]) -> Vec<u8> {
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

#[test]
fn lists_inherited_fields_duplicate_names_widgets_and_direct_ref_annotations() {
    let input = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /Annots [<< /Subtype /Text /Rect [0 1 2 3] /Contents (direct) /F 4 >> 9 0 R] >>",
        "<< /Fields [5 0 R 8 0 R 10 0 R] >>",
        "<< /T (person) /FT /Tx /Ff 3 /Kids [6 0 R 7 0 R] >>",
        "<< /T (first) /Subtype /Widget /Parent 5 0 R /Rect [0 0 10 10] /V (Alice) /DV /Default >>",
        "<< /Subtype /Widget /Parent 5 0 R /Rect [10 0 20 10] >>",
        "<< /T (dup) /FT /Btn /V /Yes /DV /Off >>",
        "<< /Type /Annot /Subtype /Link /Rect [4 5 6 7] /Contents <FEFF006C0069006E006B> /F 32 >>",
        "<< /T (dup) /FT /Tx /V (two) >>",
    ]);
    let document = open(&input);

    let fields = list_form_fields(&document).unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["person", "person.first", "dup", "dup"]
    );
    assert_eq!(fields[1].field_type.as_deref(), Some("Tx"));
    assert_eq!(fields[1].flags, Some(3));
    assert_eq!(fields[1].value.as_deref(), Some("Alice"));
    assert_eq!(fields[1].default_value.as_deref(), Some("Default"));
    assert_eq!(fields[0].widget_refs.len(), 2);
    assert_eq!(fields[0].widget_refs[0].object_number, 6);
    assert_eq!(fields[1].widget_refs[0].object_number, 6);
    assert_eq!(fields[2].value.as_deref(), Some("Yes"));

    let annotations = list_annotations(&document).unwrap();
    assert_eq!(annotations.len(), 2);
    assert_eq!(annotations[0].index, 0);
    assert_eq!(annotations[0].page_index, 0);
    assert_eq!(annotations[0].object_number, None);
    assert_eq!(annotations[0].subtype, "Text");
    assert_eq!(annotations[0].rect, [0.0, 1.0, 2.0, 3.0]);
    assert_eq!(annotations[0].contents.as_deref(), Some("direct"));
    assert_eq!(annotations[0].flags, 4);
    assert_eq!(annotations[1].object_number, Some(9));
    assert_eq!(annotations[1].contents.as_deref(), Some("link"));
}

#[test]
fn field_cycles_and_traversal_budgets_are_typed_errors() {
    let cyclic = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Fields [5 0 R] >>",
        "<< /T (cycle) /Kids [5 0 R] >>",
    ]);
    assert_eq!(
        list_form_fields(&open(&cyclic)).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );

    let deep = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Fields [5 0 R] >>",
        "<< /T (a) /Kids [6 0 R] >>",
        "<< /T (b) /Kids [7 0 R] >>",
        "<< /T (c) /Kids [8 0 R] >>",
        "<< /T (d) /Kids [9 0 R] >>",
        "<< /T (e) >>",
    ]);
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_parser_depth: 3,
            ..Limits::default()
        },
    });
    let document = engine.open(&deep, OpenOptions::default()).unwrap();
    assert_eq!(
        list_form_fields(&document).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );

    let repeated = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R >>",
        "<< /Fields [5 0 R 5 0 R 5 0 R 5 0 R 5 0 R 5 0 R] >>",
        "<< /T (a) /Kids [6 0 R 6 0 R 6 0 R 6 0 R 6 0 R 6 0 R 6 0 R 6 0 R 6 0 R 6 0 R] >>",
        "<< /T (b) >>",
    ]);
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_container_items: 50,
            ..Limits::default()
        },
    });
    let document = engine.open(&repeated, OpenOptions::default()).unwrap();
    assert_eq!(
        list_form_fields(&document).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn malformed_annotations_and_total_annotation_budget_are_typed_errors() {
    let malformed = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /Annots [4 0 R] >>",
        "<< /Subtype /Text /Rect [0 1 2] >>",
    ]);
    let error = list_annotations(&open(&malformed)).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
    assert_eq!(error.object, Some((4, 0)));

    let annots = "5 0 R ".repeat(30);
    let many = pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        &format!("<< /Type /Page /Parent 2 0 R /Annots [{annots}] >>"),
        &format!("<< /Type /Page /Parent 2 0 R /Annots [{annots}] >>"),
        "<< /Subtype /Text /Rect [0 0 1 1] >>",
    ]);
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_container_items: 50,
            ..Limits::default()
        },
    });
    let document = engine.open(&many, OpenOptions::default()).unwrap();
    assert_eq!(
        list_annotations(&document).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}
