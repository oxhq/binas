use binas_pdf::{
    BatchTextEditRequest, OpenOptions, PdfEngine, PdfErrorCode, SurgicalTextEditRequest,
};

fn pdf(content: &[u8]) -> Vec<u8> {
    make_pdf(content, false)
}

fn make_pdf(content: &[u8], signed: bool) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];
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
        format!(
            "trailer\n<< /Size 5 /Root 1 0 R{} >>\nstartxref\n{xref}\n%%EOF\n",
            if signed { " /ByteRange [0 1 2 3]" } else { "" }
        )
        .as_bytes(),
    );
    bytes
}

fn edit(old_text: &str, replacement: &str, match_index: usize) -> SurgicalTextEditRequest {
    SurgicalTextEditRequest {
        old_text: old_text.into(),
        replacement: replacement.into(),
        match_index,
    }
}

#[test]
fn plans_and_applies_multiple_edits_with_one_output() {
    let input = pdf(b"BT (alpha) Tj (bravo) Tj (alpha) Tj ET");
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let plan = document
        .plan_batch_text_edits(BatchTextEditRequest {
            edits: vec![edit("bravo", "BRAVO", 0), edit("alpha", "omega", 1)],
        })
        .unwrap();
    assert_eq!(plan.edits.len(), 2);

    let outcome = document.apply_batch_text_edits(plan).unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(outcome.report.edit_count, 2);
    assert_eq!(outcome.bytes.len(), input.len());
    let rewritten = PdfEngine::default()
        .open(&outcome.bytes, OpenOptions::default())
        .unwrap();
    assert_eq!(rewritten.query_text_all("alpha").unwrap().len(), 1);
    assert_eq!(rewritten.query_text_all("BRAVO").unwrap().len(), 1);
    assert_eq!(rewritten.query_text_all("omega").unwrap().len(), 1);
}

#[test]
fn refuses_overlapping_edits_before_writing() {
    let input = pdf(b"BT (same) Tj ET");
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let error = document
        .plan_batch_text_edits(BatchTextEditRequest {
            edits: vec![edit("same", "left", 0), edit("same", "rght", 0)],
        })
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsafeRewrite);
    assert!(error.message.contains("overlap"));

    let mut forged = document
        .plan_batch_text_edits(BatchTextEditRequest {
            edits: vec![edit("same", "left", 0)],
        })
        .unwrap();
    forged.edits.push(forged.edits[0].clone());
    assert_eq!(
        document.apply_batch_text_edits(forged).unwrap_err().code,
        PdfErrorCode::UnsafeRewrite
    );
}

#[test]
fn binds_plans_to_the_source_and_refuses_signed_documents() {
    let input = pdf(b"BT (alpha) Tj ET");
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let plan = document
        .plan_batch_text_edits(BatchTextEditRequest {
            edits: vec![edit("alpha", "omega", 0)],
        })
        .unwrap();
    let mut other = input.clone();
    other[5] = b'2';
    let other = PdfEngine::default()
        .open(&other, OpenOptions::default())
        .unwrap();
    assert_eq!(
        other.apply_batch_text_edits(plan).unwrap_err().code,
        PdfErrorCode::UnsafeRewrite
    );

    let signed = make_pdf(b"BT (alpha) Tj ET", true);
    let signed = PdfEngine::default()
        .open(&signed, OpenOptions::default())
        .unwrap();
    assert_eq!(
        signed
            .plan_batch_text_edits(BatchTextEditRequest {
                edits: vec![edit("alpha", "omega", 0)],
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}
