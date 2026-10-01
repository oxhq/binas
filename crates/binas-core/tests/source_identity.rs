use std::sync::Arc;

use binas_core::{
    Span,
    identity::{RevisionId, SnapshotId, SourceId},
    source::{PayloadProvenance, Source, SourceError, SourceSpan},
};

#[test]
fn content_identity_rejects_same_size_replacement() {
    let original = Source::copy_from_slice(b"abc", 3).unwrap();
    let replacement = Source::copy_from_slice(b"abd", 3).unwrap();
    assert_ne!(original.id(), replacement.id());
    assert_eq!(
        original.id().digest(),
        &[
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ]
    );
    let range = original.qualified_span(Span::new(0, 3).unwrap()).unwrap();
    assert!(matches!(
        range.resolve(&replacement),
        Err(SourceError::SourceMismatch { .. })
    ));
    assert_ne!(
        SnapshotId::new(original.id(), RevisionId::INITIAL),
        SnapshotId::new(replacement.id(), RevisionId::INITIAL)
    );
    let expected = SnapshotId::new(original.id(), RevisionId::INITIAL);
    let actual = SnapshotId::new(replacement.id(), RevisionId::INITIAL);
    let mismatch = expected.ensure_current(actual).unwrap_err();
    assert_eq!(mismatch.expected, expected);
    assert_eq!(mismatch.actual, actual);
    expected.ensure_current(expected).unwrap();
}

#[test]
fn ranges_check_source_and_boundaries() {
    let source = Source::copy_from_slice(b"abc", 3).unwrap();
    let range = source.qualified_span(Span::new(1, 3).unwrap()).unwrap();
    assert_eq!(range.resolve(&source).unwrap(), b"bc");
    assert_eq!(range.source(), source.id());
    assert_eq!(range.span(), Span::new(1, 3).unwrap());
    assert_eq!(
        source
            .qualified_span(Span::new(3, 3).unwrap())
            .unwrap()
            .resolve(&source)
            .unwrap(),
        b""
    );
    assert!(matches!(
        source.qualified_span(Span::new(3, 4).unwrap()),
        Err(SourceError::RangeOutOfBounds { .. })
    ));
    assert!(
        SourceSpan::new(source.id(), Span::new(0, u64::MAX).unwrap())
            .resolve(&source)
            .is_err()
    );
}

#[test]
fn source_storage_preserves_owned_and_shared_payload_allocations() {
    let mut owned = Vec::with_capacity(1024);
    owned.extend_from_slice(b"payload");
    let pointer = owned.as_ptr();
    let source = Source::from_owned(owned, 7).unwrap();
    assert_eq!(source.as_bytes().as_ptr(), pointer);
    let cloned = source.clone();
    assert!(source.shares_storage_with(&cloned));
    assert_eq!(source.as_bytes().as_ptr(), cloned.as_bytes().as_ptr());
    assert_eq!(source.id(), cloned.id());
    let same_content = Source::copy_from_slice(b"payload", 7).unwrap();
    assert_eq!(source.id(), same_content.id());
    assert!(!source.shares_storage_with(&same_content));
    let bytes: Arc<[u8]> = Arc::from(b"payload".as_slice());
    let pointer = bytes.as_ptr();
    let shared = Source::from_shared(bytes.clone(), 7).unwrap();
    let shared_again = Source::from_shared(bytes, 7).unwrap();
    assert_eq!(shared.as_bytes().as_ptr(), pointer);
    assert!(shared.shares_storage_with(&shared_again));
    drop(source);
    assert_eq!(cloned.as_bytes(), b"payload");
}

#[test]
fn constructors_reject_oversize_inputs_and_accept_empty() {
    assert!(matches!(
        Source::copy_from_slice(b"abcd", 3),
        Err(SourceError::InputTooLarge {
            actual: 4,
            maximum: 3
        })
    ));
    assert!(Source::from_owned(vec![0; 4], 3).is_err());
    assert!(Source::from_shared(Arc::from([0u8; 4]), 3).is_err());
    let empty = Source::from_owned(Vec::new(), 0).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert!(empty.shares_storage_with(&empty.clone()));
    assert!(!empty.shares_storage_with(&Source::from_owned(Vec::new(), 0).unwrap()));
}

#[test]
fn revisions_never_wrap_and_snapshots_bind_revision() {
    let source = SourceId::from_sha256([7; 32]);
    let first = RevisionId::INITIAL;
    let next = first.checked_next().unwrap();
    assert_eq!(next.value(), 1);
    assert!(RevisionId::new(u64::MAX).checked_next().is_none());
    let snapshot = SnapshotId::new(source, next);
    assert_eq!(snapshot.source(), source);
    assert_eq!(snapshot.revision(), next);
    assert_ne!(snapshot, SnapshotId::new(source, first));
    assert!(
        snapshot
            .ensure_current(SnapshotId::new(source, first))
            .is_err()
    );
}

#[test]
fn decoded_provenance_identifies_inputs_without_claiming_exact_output_mapping() {
    let source = Source::copy_from_slice(b"encoded", 7).unwrap();
    let input = source.qualified_span(Span::new(0, 7).unwrap()).unwrap();
    let provenance = PayloadProvenance::Derived {
        inputs: vec![input],
        transform: "decode:test".into(),
    };
    match provenance {
        PayloadProvenance::Derived { inputs, transform } => {
            assert_eq!(inputs[0].resolve(&source).unwrap(), b"encoded");
            assert_eq!(transform, "decode:test");
        }
        PayloadProvenance::Exact(_) => {
            panic!("decoded bytes must not claim an exact source mapping")
        }
    }
}
