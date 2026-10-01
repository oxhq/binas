use std::sync::Arc;

use binas_core::{Source, Span};
use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode};

const PDF: &[u8] = include_bytes!("fixtures/pdf/uncompressed-direct-length.pdf");

#[test]
fn owned_and_shared_open_retain_the_input_allocation() {
    let engine = PdfEngine::default();
    let owned = PDF.to_vec();
    let pointer = owned.as_ptr();
    let document = engine.open_owned(owned, OpenOptions::default()).unwrap();
    assert_eq!(document.input_source().as_bytes().as_ptr(), pointer);
    assert_eq!(document.input_source().as_bytes(), PDF);
    let clone = document.clone();
    assert!(
        clone
            .input_source()
            .shares_storage_with(document.input_source())
    );
    assert_eq!(clone.inspect().unwrap(), document.inspect().unwrap());

    let shared: Arc<[u8]> = PDF.into();
    let pointer = shared.as_ptr();
    let shared_document = engine
        .open_shared(shared.clone(), OpenOptions::default())
        .unwrap();
    assert_eq!(shared_document.input_source().as_bytes().as_ptr(), pointer);
    // The document and each source-backed encoded payload can retain owners;
    // none of those references copies the caller's byte allocation.
    assert!(Arc::strong_count(&shared) >= 2);
    assert_eq!(shared_document.source_id(), document.source_id());
    drop(shared_document);
    assert_eq!(Arc::strong_count(&shared), 1);
}

#[test]
fn all_input_open_paths_honor_the_configured_limit() {
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_input_bytes: PDF.len() - 1,
            ..Limits::default()
        },
    });
    for error in [
        engine.open(PDF, OpenOptions::default()).unwrap_err(),
        engine.open(PDF, OpenOptions { repair: true }).unwrap_err(),
        engine
            .open_owned(PDF.to_vec(), OpenOptions::default())
            .unwrap_err(),
        engine
            .open_shared(Arc::from(PDF), OpenOptions::default())
            .unwrap_err(),
        engine
            .open_source(
                Source::copy_from_slice(PDF, PDF.len()).unwrap(),
                OpenOptions::default(),
            )
            .unwrap_err(),
        engine
            .open_with_password(PDF, "", OpenOptions::default())
            .unwrap_err(),
        engine
            .decrypt_input_to_plain(PDF, "", OpenOptions::default())
            .unwrap_err(),
        engine
            .inspect_encryption_input(PDF, OpenOptions::default())
            .unwrap_err(),
        engine
            .open_with_public_key(PDF, &[], &[], OpenOptions::default())
            .unwrap_err(),
        engine
            .decrypt_public_key_input(PDF, &[], &[], OpenOptions::default())
            .unwrap_err(),
    ] {
        assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    }
    let exact = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_input_bytes: PDF.len(),
            ..Limits::default()
        },
    });
    exact
        .open_owned(PDF.to_vec(), OpenOptions::default())
        .unwrap();
    exact
        .open_shared(Arc::from(PDF), OpenOptions::default())
        .unwrap();
}

#[test]
fn source_ranges_reject_a_same_length_different_document() {
    let engine = PdfEngine::default();
    let original = engine.open(PDF, OpenOptions::default()).unwrap();
    let mut other = PDF.to_vec();
    // Both headers are legal and equal in length, with identical xref offsets.
    let header = other
        .windows(5)
        .position(|bytes| bytes == b"%PDF-")
        .unwrap();
    other[header + 7] = if other[header + 7] == b'7' {
        b'6'
    } else {
        b'7'
    };
    let other = engine.open_owned(other, OpenOptions::default()).unwrap();
    assert_eq!(original.source_len(), other.source_len());
    assert_ne!(original.source_id(), other.source_id());
    let range = original.source_span(Span::new(0, 8).unwrap()).unwrap();
    assert!(range.resolve(other.input_source()).is_err());
    assert_eq!(range.resolve(original.input_source()).unwrap(), &PDF[..8]);
    assert!(
        original
            .source_span(Span::new(0, PDF.len() as u64 + 1).unwrap())
            .is_err()
    );
}
