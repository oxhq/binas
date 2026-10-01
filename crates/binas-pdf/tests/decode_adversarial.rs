use std::io::Write;

use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode, StreamObjectRef,
    read_decoded_stream_shared,
};

fn pdf(stream: &[u8], entries: &str) -> Vec<u8> {
    let mut stream_object =
        format!("<< /Length {} {entries} >>\nstream\n", stream.len()).into_bytes();
    stream_object.extend_from_slice(stream);
    stream_object.extend_from_slice(b"\nendstream");
    assemble(&[b"<< /Type /Catalog >>".to_vec(), stream_object])
}

fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

#[test]
fn inline_image_expansion_spends_the_document_decode_budget() {
    let mut content = b"BI /W 128 /H 100 /BPC 8 /CS /G /F /RL ID ".to_vec();
    for _ in 0..100 {
        content.extend_from_slice(&[129, b'x']);
    }
    content.extend_from_slice(&[128]);
    content.extend_from_slice(b" EI");
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(&content);
    stream.extend_from_slice(b"\nendstream");
    let bytes = assemble(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream,
    ]);
    // Enough for the outer unfiltered content but not its 12,800 image samples.
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: content.len() * 3 + 2,
            ..Limits::default()
        },
    });
    let document = engine.open_owned(bytes, OpenOptions::default()).unwrap();
    assert_eq!(
        document.extract_text_spans().unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn repair_cannot_turn_a_stream_limit_failure_into_a_missing_object() {
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_stream_bytes: 3,
            ..Limits::default()
        },
    });
    let bytes = pdf(b"four", "");
    assert_eq!(
        engine
            .open(&bytes, OpenOptions::default())
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(
        engine
            .open(&bytes, OpenOptions { repair: true })
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn malformed_predictor_types_must_not_decode_using_default_parameters() {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"one").unwrap();
    let stream = encoder.finish().unwrap();
    for key in [
        "Predictor",
        "Colors",
        "Columns",
        "BitsPerComponent",
        "EarlyChange",
    ] {
        for value in ["(invalid)", "true", "1.5", "[]", "-1"] {
            let document = PdfEngine::default()
                .open_owned(
                    pdf(
                        &stream,
                        &format!("/Filter /FlateDecode /DecodeParms << /{key} {value} >>"),
                    ),
                    OpenOptions::default(),
                )
                .unwrap();
            let error = read_decoded_stream_shared(
                &document,
                StreamObjectRef {
                    object_number: 2,
                    object_generation: 0,
                },
            )
            .unwrap_err();
            assert!(
                matches!(
                    error.code,
                    PdfErrorCode::InvalidSyntax
                        | PdfErrorCode::ResourceLimit
                        | PdfErrorCode::UnsupportedFeature
                ),
                "{key} {value}"
            );
            assert_eq!(document.decode_metrics().cache_entries, 0);
        }
    }
    for (key, value) in [
        ("Predictor", "3"),
        ("Colors", "0"),
        ("Columns", "0"),
        ("BitsPerComponent", "33"),
        ("EarlyChange", "2"),
    ] {
        let document = PdfEngine::default()
            .open_owned(
                pdf(
                    &stream,
                    &format!("/Filter /FlateDecode /DecodeParms << /{key} {value} >>"),
                ),
                OpenOptions::default(),
            )
            .unwrap();
        assert!(
            read_decoded_stream_shared(
                &document,
                StreamObjectRef {
                    object_number: 2,
                    object_generation: 0
                }
            )
            .is_err(),
            "{key} {value}"
        );
        assert_eq!(document.decode_metrics().cache_entries, 0);
    }
}

#[test]
fn null_parameter_entries_and_unknown_keys_keep_existing_default_semantics() {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"one").unwrap();
    let stream = encoder.finish().unwrap();
    for declaration in [
        "/Filter /FlateDecode /DecodeParms null",
        "/Filter [/FlateDecode] /DecodeParms [null]",
        "/Filter /FlateDecode /DecodeParms << /PrivateKey (opaque) >>",
        "/Filter /FlateDecode /DecodeParms << /Predictor null /Colors null /Columns null /BitsPerComponent null /EarlyChange null >>",
    ] {
        let document = PdfEngine::default()
            .open_owned(pdf(&stream, declaration), OpenOptions::default())
            .unwrap();
        assert_eq!(
            read_decoded_stream_shared(
                &document,
                StreamObjectRef {
                    object_number: 2,
                    object_generation: 0
                }
            )
            .unwrap()
            .as_ref(),
            b"one"
        );
    }
    let hex: String = stream.iter().map(|byte| format!("{byte:02X}")).collect();
    for params in [
        "[null << /Predictor (wrong) >>]",
        "[null << /Colors true >>]",
    ] {
        let document = PdfEngine::default()
            .open_owned(
                pdf(
                    format!("{hex}>").as_bytes(),
                    &format!("/Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms {params}"),
                ),
                OpenOptions::default(),
            )
            .unwrap();
        let object = StreamObjectRef {
            object_number: 2,
            object_generation: 0,
        };
        let mut previous_work = document.decode_metrics().work_bytes;
        for _ in 0..2 {
            assert_eq!(
                read_decoded_stream_shared(&document, object)
                    .unwrap_err()
                    .code,
                PdfErrorCode::InvalidSyntax
            );
            assert_eq!(document.decode_metrics().cache_entries, 0);
            assert!(document.decode_metrics().work_bytes > previous_work);
            previous_work = document.decode_metrics().work_bytes;
        }
    }
}
