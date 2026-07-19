use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode};

fn classic(objects: &[&str]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
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

fn document() -> binas_pdf::PdfDocument {
    let input = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R 6 0 R] /Count 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [1 2 301 402] /CropBox [10 20 290 380] /BleedBox [2 3 300 401] /TrimBox 7 0 R /ArtBox [6 7 296 397] /Rotate 450 >>",
        "<< /Type /Pages /Parent 2 0 R /Kids [5 0 R] /Count 1 /MediaBox [0 0 400 500] /CropBox [10 20 390 480] /BleedBox [11 21 389 479] /TrimBox [12 22 388 478] /ArtBox 8 0 R /Rotate -90 >>",
        "<< /Type /Page /Parent 4 0 R >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>",
        "[4 5 298 399]",
        "[13 23 387 477]",
    ]);
    PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap()
}

#[test]
fn reads_direct_inherited_and_fallback_page_geometry() {
    let document = document();
    assert_eq!(
        document.page_geometry(0).unwrap(),
        binas_pdf::PageGeometry {
            media_box: [1.0, 2.0, 301.0, 402.0],
            crop_box: [10.0, 20.0, 290.0, 380.0],
            bleed_box: Some([2.0, 3.0, 300.0, 401.0]),
            trim_box: Some([4.0, 5.0, 298.0, 399.0]),
            art_box: Some([6.0, 7.0, 296.0, 397.0]),
            rotation_degrees: 90,
        }
    );
    assert_eq!(
        document.page_geometry(1).unwrap(),
        binas_pdf::PageGeometry {
            media_box: [0.0, 0.0, 400.0, 500.0],
            crop_box: [10.0, 20.0, 390.0, 480.0],
            bleed_box: Some([11.0, 21.0, 389.0, 479.0]),
            trim_box: Some([12.0, 22.0, 388.0, 478.0]),
            art_box: Some([13.0, 23.0, 387.0, 477.0]),
            rotation_degrees: 270,
        }
    );
    assert_eq!(
        document.page_geometry(2).unwrap(),
        binas_pdf::PageGeometry {
            media_box: [0.0, 0.0, 200.0, 300.0],
            crop_box: [0.0, 0.0, 200.0, 300.0],
            bleed_box: None,
            trim_box: None,
            art_box: None,
            rotation_degrees: 0,
        }
    );
}

#[test]
fn rejects_malformed_optional_page_boxes_and_references() {
    let malformed_box = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /BleedBox [0 0 100] >>",
    ]);
    let error = PdfEngine::default()
        .open(&malformed_box, OpenOptions::default())
        .unwrap()
        .page_geometry(0)
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);

    let missing_box_reference = classic(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /ArtBox 99 0 R >>",
    ]);
    let error = PdfEngine::default()
        .open(&missing_box_reference, OpenOptions::default())
        .unwrap()
        .page_geometry(0)
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
}

#[test]
fn rejects_an_out_of_range_page_index() {
    assert_eq!(
        document().page_geometry(3).unwrap_err().code,
        PdfErrorCode::SelectionNotFound
    );
}
