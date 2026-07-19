use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode, StreamObjectRef,
    read_decoded_stream,
};

fn pdf_bytes(objects: &[&[u8]]) -> Vec<u8> {
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

fn open(objects: &[&[u8]]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(&pdf_bytes(objects), OpenOptions::default())
        .unwrap()
}

fn stream(object_number: u32) -> StreamObjectRef {
    StreamObjectRef {
        object_number,
        object_generation: 0,
    }
}

#[test]
fn reads_raw_and_filtered_streams_by_stable_reference() {
    let document = open(&[
        b"<< /Type /Catalog >>",
        b"<< /Length 3 >>\nstream\nraw\nendstream",
        b"<< /Length 11 /Filter /ASCIIHexDecode >>\nstream\n48656c6c6f>\nendstream",
    ]);

    assert_eq!(read_decoded_stream(&document, stream(2)).unwrap(), b"raw");
    assert_eq!(read_decoded_stream(&document, stream(3)).unwrap(), b"Hello");
}

#[test]
fn decoded_stream_reader_fails_closed() {
    let document = open(&[
        b"<< /Type /Catalog >>",
        b"<< /Length 4 /Filter /DCTDecode >>\nstream\njpeg\nendstream",
        b"<< /Length 1 /Filter [/FlateDecode 7] >>\nstream\nA\nendstream",
    ]);

    for (reference, code) in [
        (stream(99), PdfErrorCode::SelectionNotFound),
        (stream(1), PdfErrorCode::SelectionNotFound),
        (stream(2), PdfErrorCode::UnsupportedFeature),
        (stream(3), PdfErrorCode::InvalidSyntax),
    ] {
        let error = read_decoded_stream(&document, reference).unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.object, Some((reference.object_number, 0)));
    }

    let encoded = [252, b'A', 128];
    let expanded = [
        b"<< /Length 3 /Filter /RunLengthDecode >>\nstream\n".as_slice(),
        encoded.as_slice(),
        b"\nendstream".as_slice(),
    ]
    .concat();
    let limits = Limits {
        max_stream_bytes: 8,
        max_total_decoded_bytes: 4,
        ..Limits::default()
    };
    let limited = PdfEngine::new(EngineConfig { limits })
        .open(
            &pdf_bytes(&[b"<< /Type /Catalog >>", expanded.as_slice()]),
            OpenOptions::default(),
        )
        .unwrap();
    let error = read_decoded_stream(&limited, stream(2)).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    assert_eq!(error.object, Some((2, 0)));
}
