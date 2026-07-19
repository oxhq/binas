use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode, StreamMutationRequest};

fn pdf() -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        b"<< /Length 14 >>\nstream\nBT (old) Tj ET\nendstream".to_vec(),
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

#[test]
fn mutates_and_verifies_a_selected_stream() {
    let document = PdfEngine::default()
        .open(&pdf(), OpenOptions::default())
        .unwrap();
    let outcome = document
        .mutate_stream(StreamMutationRequest {
            object_number: 4,
            object_generation: 0,
            decoded_bytes: b"BT (replacement) Tj ET".to_vec(),
        })
        .unwrap();
    assert!(outcome.verification.passed);
    let reopened = PdfEngine::default()
        .open(&outcome.bytes, OpenOptions::default())
        .unwrap();
    assert_eq!(
        reopened.query_text("replacement", 0).unwrap().text,
        "replacement"
    );
}

#[test]
fn stream_mutation_rejects_missing_and_non_stream_objects() {
    let document = PdfEngine::default()
        .open(&pdf(), OpenOptions::default())
        .unwrap();
    for object_number in [1, 99] {
        let error = document
            .mutate_stream(StreamMutationRequest {
                object_number,
                object_generation: 0,
                decoded_bytes: Vec::new(),
            })
            .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::SelectionNotFound);
    }
}
