use binas_pdf::{
    BatchTextEditRequest, EncodedImageReplacementRequest, ImageMaskPolicy, OpenOptions,
    PageCompositionPlacement, PageCompositionRequest, PdfEngine, SurgicalTextEditRequest,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = PdfEngine::default();
    let source_bytes = fixture_pdf();
    let source = engine.open(&source_bytes, OpenOptions::default())?;

    assert_eq!(source.inspect()?.page_count, 2);
    assert!(
        source
            .extract_text_spans()?
            .spans
            .iter()
            .any(|span| span.text == "ALPHA")
    );

    let edited = source.batch_text_edit(BatchTextEditRequest {
        edits: vec![SurgicalTextEditRequest {
            old_text: "ALPHA".into(),
            replacement: "OMEGA".into(),
            match_index: 0,
        }],
    })?;
    assert!(edited.verification.passed);

    let edited = engine.open(&edited.bytes, OpenOptions::default())?;
    let composed = edited.compose_page(
        &source,
        PageCompositionRequest {
            target_page_index: 0,
            source_page_index: 1,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            placement: PageCompositionPlacement::Overlay,
        },
    )?;
    assert!(composed.verification.passed);

    let composed = engine.open(&composed.bytes, OpenOptions::default())?;
    let image = composed.replace_image_xobject_encoded(EncodedImageReplacementRequest {
        object_number: 7,
        object_generation: 0,
        encoded_bytes: jpeg(),
        mask_policy: ImageMaskPolicy::Reject,
    })?;
    assert!(image.verification.passed);
    assert_eq!(image.report.width, 2);
    assert_eq!(image.report.height, 1);
    Ok(())
}

fn fixture_pdf() -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /XObject << /Im0 7 0 R >> >> /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 6 0 R >>".to_vec(),
        stream(b"BT (ALPHA) Tj ET"),
        stream(b"BT (BETA) Tj ET"),
        image_stream(),
    ])
}

fn image_stream() -> Vec<u8> {
    let bytes = [1, 2, 3, 4, 5, 6];
    [
        b"<< /Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceRGB /Length 6 >>\nstream\n".as_slice(),
        bytes.as_slice(),
        b"\nendstream".as_slice(),
    ]
    .concat()
}

fn stream(bytes: &[u8]) -> Vec<u8> {
    [
        format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes(),
        bytes.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}

fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn jpeg() -> Vec<u8> {
    vec![
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x02, 0x03, 0x01, 0x11, 0x00,
        0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xda, 0x00, 0x0c, 0x03, 0x01, 0x00, 0x02, 0x00,
        0x03, 0x00, 0x00, 0x3f, 0x00, 0x00, 0xff, 0xd9,
    ]
}
