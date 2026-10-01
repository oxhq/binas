use binas_core::{
    BudgetLimits, CancellationToken, OperationContext, ResourceBudget, RevisionId, SnapshotId,
    capability::CapabilityStatus,
};
use binas_pdf::{
    OpenOptions, PdfDocument, PdfEngine, PdfErrorCode, PdfReferenceIndex, PdfReferenceOrigin,
    PdfReferencePathStep, PdfReferenceTarget,
};

fn context(live: u64, work: u64) -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: live,
            max_work_units: work,
        }),
        CancellationToken::new(),
    )
}

fn document(label: char) -> PdfDocument {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /Custom [4 2 R 4 2 R 4 0 R] >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".to_string(),
        format!("<< /Back 1 0 R /Self 4 2 R /Label ({label}) >>"),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, value) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        let generation = if index == 3 { 2 } else { 0 };
        bytes.extend_from_slice(
            format!("{} {generation} obj\n{value}\nendobj\n", index + 1).as_bytes(),
        );
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for (index, offset) in offsets.into_iter().enumerate() {
        let generation = if index == 3 { 2 } else { 0 };
        bytes.extend_from_slice(format!("{offset:010} {generation:05} n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R /Custom 4 2 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    PdfEngine::default()
        .open_owned(bytes, OpenOptions::default())
        .unwrap()
}

fn target(number: u32, generation: u16) -> PdfReferenceTarget {
    PdfReferenceTarget { number, generation }
}

#[test]
fn preserves_reference_occurrences_paths_cycles_trailer_and_generations() {
    let document = document('A');
    let context = context(100_000, 100_000);
    let index = PdfReferenceIndex::build(&document, &context).unwrap();
    assert_eq!(index.records().len(), 10);
    let incoming = index.incoming(index.snapshot(), target(4, 2)).unwrap();
    assert_eq!(incoming.len(), 4);
    assert!(incoming.iter().all(|entry| !entry.dangling));
    for item in [0, 1] {
        assert!(incoming.iter().any(|entry| entry.origin
            == PdfReferenceOrigin::Object(target(1, 0))
            && entry.path
                == [
                    PdfReferencePathStep::Key(b"Custom".to_vec()),
                    PdfReferencePathStep::Index(item)
                ]));
    }
    assert!(
        incoming
            .iter()
            .any(|entry| entry.origin == PdfReferenceOrigin::Trailer
                && entry.path == [PdfReferencePathStep::Key(b"Custom".to_vec())])
    );
    assert!(incoming.iter().any(
        |entry| entry.origin == PdfReferenceOrigin::Object(target(4, 2))
            && entry.path == [PdfReferencePathStep::Key(b"Self".to_vec())]
    ));
    // A back edge plus the catalog's forward edges is retained without traversing the cycle.
    assert!(
        index
            .incoming(index.snapshot(), target(1, 0))
            .unwrap()
            .iter()
            .any(|entry| entry.origin == PdfReferenceOrigin::Object(target(4, 2)))
    );
    let dangling = index.incoming(index.snapshot(), target(4, 0)).unwrap();
    assert_eq!(dangling.len(), 1);
    assert!(dangling[0].dangling);
    assert_eq!(
        index
            .records()
            .iter()
            .filter(|entry| entry.dangling)
            .count(),
        1
    );
    assert!(matches!(
        index.ownership_capability(),
        CapabilityStatus::Unsupported { .. }
    ));
}

#[test]
fn source_and_revision_binding_reject_same_number_other_document() {
    let first = document('A');
    let second = document('B');
    assert_eq!(first.source_len(), second.source_len());
    let context = context(100_000, 100_000);
    let index = PdfReferenceIndex::build(&first, &context).unwrap();
    let other = PdfReferenceIndex::build(&second, &context).unwrap();
    assert_ne!(index.snapshot(), other.snapshot());
    assert_eq!(
        index
            .incoming(other.snapshot(), target(4, 2))
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
    let newer = SnapshotId::new(first.source_id(), RevisionId::new(1));
    assert!(index.incoming(newer, target(4, 2)).is_err());
}

#[test]
fn retained_revision_binding_matches_current_document_bytes() {
    let document = document('A');
    let context = context(100_000, 100_000);
    let snapshot = SnapshotId::new(document.source_id(), RevisionId::new(7));
    let state = binas_core::transaction::RevisionState::new(snapshot, document.clone());
    let index = PdfReferenceIndex::build_for_revision(&state, &context).unwrap();
    assert_eq!(index.snapshot(), snapshot);
    assert_eq!(index.incoming(snapshot, target(4, 2)).unwrap().len(), 4);
    let stale_source = binas_core::transaction::RevisionState::new(
        SnapshotId::new(
            binas_core::SourceId::from_sha256([0; 32]),
            RevisionId::new(7),
        ),
        document,
    );
    assert_eq!(
        PdfReferenceIndex::build_for_revision(&stale_source, &context)
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
}

#[test]
fn cancellation_and_work_live_limits_release_accounting_without_refunds() {
    let document = document('A');
    let cancelled = context(100_000, 100_000);
    cancelled.cancellation().cancel();
    assert_eq!(
        PdfReferenceIndex::build(&document, &cancelled)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    assert_eq!(cancelled.budget().snapshot().work_units, 0);
    for (live, work) in [(100_000, 0), (0, 100_000), (100_000, 50)] {
        let limited = context(live, work);
        assert_eq!(
            PdfReferenceIndex::build(&document, &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(limited.budget().snapshot().live_bytes, 0);
    }
    let context = context(100_000, 100_000);
    let index = PdfReferenceIndex::build(&document, &context).unwrap();
    let logical = index
        .records()
        .iter()
        .map(|entry| {
            std::mem::size_of_val(entry)
                + entry.path.len() * std::mem::size_of::<PdfReferencePathStep>()
                + entry
                    .path
                    .iter()
                    .map(|step| match step {
                        PdfReferencePathStep::Key(key) => key.len(),
                        PdfReferencePathStep::Index(_) => 0,
                    })
                    .sum::<usize>()
        })
        .sum::<usize>();
    let before = context.budget().snapshot();
    assert_eq!(before.live_bytes, logical as u64);
    assert!(before.work_units > 0);
    drop(index);
    assert_eq!(context.budget().snapshot().live_bytes, 0);
    assert_eq!(context.budget().snapshot().work_units, before.work_units);
}
