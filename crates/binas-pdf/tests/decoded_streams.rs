use binas_core::{CancellationToken, Source};
use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode, StreamObjectRef,
    read_decoded_stream, read_decoded_stream_shared,
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

#[test]
fn repeated_reads_share_payload_and_eviction_keeps_live_handles_and_spent_work() {
    let document = open(&[
        b"<< /Type /Catalog >>",
        b"<< /Length 3 >>\nstream\nraw\nendstream",
    ]);
    let first = read_decoded_stream_shared(&document, stream(2)).unwrap();
    let first_work = document.decode_metrics().work_bytes;
    let clone = document.clone();
    let second = read_decoded_stream_shared(&clone, stream(2)).unwrap();
    assert_eq!(first.as_ptr(), second.as_ptr());
    assert_eq!(clone.decode_metrics().hits, 1);
    assert_eq!(clone.decode_metrics().misses, 1);
    // A hit scans the encoded key and charges one request; it avoids the decoder.
    assert_eq!(clone.decode_metrics().work_bytes, first_work + 4);
    clone.clear_decode_cache();
    assert_eq!(document.decode_metrics().cache_entries, 0);
    assert_eq!(document.decode_metrics().live_decoded_bytes, 3);
    let spent = document.decode_metrics().work_bytes;
    drop(first);
    drop(second);
    assert_eq!(document.decode_metrics().live_decoded_bytes, 0);
    assert_eq!(document.decode_metrics().work_bytes, spent);
    assert_eq!(read_decoded_stream(&document, stream(2)).unwrap(), b"raw");
    assert_eq!(document.decode_metrics().misses, 2);
    assert!(document.decode_metrics().work_bytes > spent);
}

#[test]
fn legacy_reads_clones_and_failed_retries_cannot_reset_lifetime_work() {
    let bytes = pdf_bytes(&[
        b"<< /Type /Catalog >>",
        b"<< /Length 3 >>\nstream\nraw\nendstream",
    ]);
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_cache_bytes: 0,
            max_decode_work_bytes: 24,
            ..Limits::default()
        },
    })
    .open_owned(bytes, OpenOptions::default())
    .unwrap();
    assert_eq!(read_decoded_stream(&document, stream(2)).unwrap(), b"raw");
    assert_eq!(
        read_decoded_stream(&document.clone(), stream(2)).unwrap(),
        b"raw"
    );
    assert_eq!(document.decode_metrics().work_bytes, 22);
    document.clear_decode_cache();
    assert_eq!(
        read_decoded_stream(&document, stream(2)).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(document.decode_metrics().work_bytes, 22);

    let bad = pdf_bytes(&[
        b"<< /Type /Catalog >>",
        b"<< /Length 2 /Filter /ASCIIHexDecode >>\nstream\ng>\nendstream",
    ]);
    let bad = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 10,
            ..Limits::default()
        },
    })
    .open_owned(bad, OpenOptions::default())
    .unwrap();
    assert_eq!(
        read_decoded_stream(&bad, stream(2)).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
    let after_failure = bad.decode_metrics();
    assert!(after_failure.work_bytes > 0);
    assert_eq!(after_failure.cache_entries, 0);
    for _ in 0..5 {
        let _ = read_decoded_stream(&bad.clone(), stream(2));
    }
    assert_eq!(
        read_decoded_stream(&bad, stream(2)).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert!(bad.decode_metrics().work_bytes >= after_failure.work_bytes);
}

#[test]
fn cancelled_reads_leave_source_usable_and_retries_share_the_budget() {
    let bytes = pdf_bytes(&[
        b"<< /Type /Catalog >>",
        b"<< /Length 3 >>\nstream\nraw\nendstream",
    ]);
    let engine = PdfEngine::default();
    let document = engine.open(&bytes, OpenOptions::default()).unwrap();
    let cached = read_decoded_stream_shared(&document, stream(2)).unwrap();
    let before = document.decode_metrics();
    let cancellation = CancellationToken::new();
    let cancelled = document.with_cancellation(cancellation.clone());
    cancellation.cancel();
    assert_eq!(
        read_decoded_stream(&cancelled, stream(2)).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    assert_eq!(document.decode_metrics(), before);
    assert_eq!(cached.as_ref(), b"raw");
    assert_eq!(read_decoded_stream(&document, stream(2)).unwrap(), b"raw");
    assert!(document.decode_metrics().work_bytes > before.work_bytes);
    assert_eq!(
        engine
            .open_source_with_cancellation(
                Source::from_owned(bytes, usize::MAX).unwrap(),
                OpenOptions::default(),
                cancellation
            )
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
}

#[test]
fn live_decoded_limit_includes_handles_after_cache_clear() {
    let bytes = pdf_bytes(&[
        b"<< /Type /Catalog >>",
        b"<< /Length 3 >>\nstream\none\nendstream",
        b"<< /Length 3 >>\nstream\ntwo\nendstream",
    ]);
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_live_bytes: 5,
            ..Limits::default()
        },
    })
    .open_owned(bytes, OpenOptions::default())
    .unwrap();
    let held = read_decoded_stream_shared(&document, stream(2)).unwrap();
    document.clear_decode_cache();
    assert_eq!(
        read_decoded_stream(&document, stream(3)).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(document.decode_metrics().live_decoded_bytes, 3);
    let spent = document.decode_metrics().work_bytes;
    drop(held);
    assert_eq!(read_decoded_stream(&document, stream(3)).unwrap(), b"two");
    assert!(document.decode_metrics().work_bytes > spent);
}
