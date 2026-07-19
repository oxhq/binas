use binas_pdf::{OpenOptions, OverlayStampRequest, PdfEngine, PdfErrorCode, TextOverlayRequest};

fn pdf(signed: bool, encrypted: bool) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources << /XObject << /BinasOverlay 7 0 R >> >> >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R /MediaBox [0 0 100 100] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R /MediaBox [0 0 100 100] >>".to_vec(),
        stream("", b"BT (A) Tj ET"),
        stream("", b"BT (B) Tj ET"),
        stream(
            " /Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 1 1] /Resources << >>",
            b"0 0 1 1 re f",
        ),
    ];
    if signed {
        objects.push(b"<< /Type /Sig /ByteRange [0 1 2 3] /Contents (x) >>".to_vec());
    }
    let encrypt_ref = if encrypted {
        objects.push(b"<< /Filter /Standard >>".to_vec());
        format!(" /Encrypt {} 0 R", objects.len())
    } else {
        String::new()
    };
    classic(&objects, &encrypt_ref)
}

fn stream(entries: &str, bytes: &[u8]) -> Vec<u8> {
    [
        format!("<< /Length {}{entries} >>\nstream\n", bytes.len()).into_bytes(),
        bytes.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}

fn classic(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
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
            "trailer\n<< /Size {} /Root 1 0 R{trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
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

fn request() -> OverlayStampRequest {
    OverlayStampRequest {
        page_indices: vec![0, 1],
        form_content: b"0 0 10 10 re 1 0 0 rg f".to_vec(),
        bbox: [0.0, 0.0, 10.0, 10.0],
        transform: [2.0, 0.0, 0.0, 3.0, 12.0, 24.0],
        opacity: Some(0.5),
    }
}

#[test]
fn places_one_shared_form_on_selected_pages_with_deterministic_names() {
    let outcome = open(&pdf(false, false))
        .place_overlay_stamp(request())
        .unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(outcome.report.pages_stamped, 2);
    assert_eq!(
        outcome.report.resource_names,
        vec!["BinasOverlay1", "BinasOverlay1"]
    );
    assert!(
        outcome
            .bytes
            .windows(b"/BinasOverlay1 Do Q".len())
            .any(|window| window == b"/BinasOverlay1 Do Q")
    );
    assert!(
        outcome
            .bytes
            .windows(b"/GS0 gs".len())
            .any(|window| window == b"/GS0 gs")
    );
    let reopened = open(&outcome.bytes);
    assert_eq!(reopened.inspect().unwrap().page_count, 2);
    assert_eq!(reopened.query_text_all("A").unwrap().len(), 1);
    assert_eq!(reopened.query_text_all("B").unwrap().len(), 1);
}

#[test]
fn places_selectable_text_with_a_built_in_font() {
    let outcome = open(&pdf(false, false))
        .place_text_overlay(TextOverlayRequest {
            page_index: 0,
            text: "APPROVED".into(),
            x: 12.0,
            y: 24.0,
            font_size: 12.0,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    assert!(outcome.verification.font_resource_matches);
    assert!(outcome.verification.text_selectable);
    assert!(
        outcome
            .bytes
            .windows(b"/BaseFont /Helvetica".len())
            .any(|window| window == b"/BaseFont /Helvetica")
    );
    let reopened = open(&outcome.bytes);
    assert_eq!(reopened.inspect().unwrap().page_count, 2);
    let approved = reopened.query_text("APPROVED", 0).unwrap();
    assert_eq!(approved.text, "APPROVED");
    assert_eq!(approved.object_number, outcome.report.form_object_number);
}

#[test]
fn validates_bbox_matrix_opacity_selection_and_resource_free_content() {
    let document = open(&pdf(false, false));
    let mut invalid = request();
    invalid.opacity = Some(1.1);
    assert_eq!(
        document.place_overlay_stamp(invalid).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );
    let mut invalid = request();
    invalid.transform = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    assert_eq!(
        document.place_overlay_stamp(invalid).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );
    let mut invalid = request();
    invalid.form_content = b"/Other Do".to_vec();
    assert_eq!(
        document.place_overlay_stamp(invalid).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );
    let mut invalid = request();
    invalid.page_indices = vec![0, 0];
    assert_eq!(
        document.place_overlay_stamp(invalid).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn signed_and_encrypted_inputs_fail_closed() {
    for input in [pdf(true, false), pdf(false, true)] {
        assert_eq!(
            open(&input)
                .place_overlay_stamp(request())
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
    }
}
