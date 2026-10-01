use binas_core::{
    Adapter, BudgetLimits, CancellationToken, ContextError, OperationContext, PayloadProvenance,
    ResourceBudget, Source,
    capability::CapabilityStatus,
    graph::{EdgeKind, GraphProjection},
    verification::CheckStatus,
};
use binas_pdf::{
    BatchTextEditRequest, PdfAdapter, PdfAdapterError, PdfGraphNodeId, SurgicalTextEditRequest,
};

fn context(work: u64) -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1_000_000,
            max_work_units: work,
        }),
        CancellationToken::new(),
    )
}
fn pdf(text: &str) -> Vec<u8> {
    let content = format!("BT ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /MediaBox [0 0 200 200] >>".to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
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
fn source(bytes: Vec<u8>) -> Source {
    Source::from_owned(bytes, 1_000_000).unwrap()
}
fn request() -> BatchTextEditRequest {
    BatchTextEditRequest {
        edits: vec![SurgicalTextEditRequest {
            old_text: "alpha".into(),
            replacement: "omega".into(),
            match_index: 0,
        }],
    }
}

#[test]
fn source_bound_lifecycle_and_reference_projection() {
    let adapter = PdfAdapter::default();
    let context = context(1_000_000);
    let document = adapter.open(source(pdf("alpha")), &context).unwrap();
    let graph = adapter.inspect(&document, &context).unwrap();
    assert!(adapter.capabilities().iter().any(|capability| {
        capability.id == "pdf.graph.ownership"
            && matches!(capability.status, CapabilityStatus::Unsupported { .. })
    }));
    assert_eq!(graph.snapshot(), adapter.snapshot(&document));
    assert_eq!(graph.nodes().count(), 6);
    assert_eq!(graph.edges().count(), 6);
    assert_eq!(
        graph
            .edges()
            .filter(|edge| edge.kind == EdgeKind::Reference)
            .count(),
        5
    );
    let payload = graph
        .nodes()
        .find(|node| {
            node.id
                == PdfGraphNodeId::EncodedStream {
                    number: 4,
                    generation: 0,
                }
        })
        .unwrap();
    let Some(PayloadProvenance::Exact(range)) = payload.provenance else {
        panic!("raw encoded stream must have exact provenance")
    };
    assert_eq!(
        range.resolve(document.input_source()).unwrap(),
        b"BT (alpha) Tj ET"
    );
    assert!(
        graph
            .nodes()
            .filter(|node| node.kind != "pdf.encoded_stream")
            .all(|node| node.provenance.is_none())
    );
    assert!(
        graph
            .edges()
            .any(|edge| edge.kind == EdgeKind::Containment && edge.to == payload.id)
    );
    assert!(
        graph
            .edges()
            .any(|edge| edge.from == PdfGraphNodeId::Trailer
                && edge.to
                    == PdfGraphNodeId::Object {
                        number: 1,
                        generation: 0
                    })
    );
    let plan = adapter.plan(&document, &request(), &context).unwrap();
    let candidate = adapter.apply(&document, &plan, &context).unwrap();
    adapter
        .verify(&document, &plan, &candidate, &context)
        .unwrap()
        .validate(plan.required_checks())
        .unwrap();
    assert_ne!(adapter.snapshot(&document), adapter.snapshot(&candidate));
    assert_eq!(document.query_text("alpha", 0).unwrap().text, "alpha");
    assert!(matches!(
        adapter.apply(&candidate, &plan, &context),
        Err(PdfAdapterError::StalePlan)
    ));
    let other = adapter.open(source(pdf("bravo")), &context).unwrap();
    assert_eq!(other.source_len(), document.source_len());
    assert!(matches!(
        adapter.apply(&other, &plan, &context),
        Err(PdfAdapterError::StalePlan)
    ));
    assert!(matches!(
        adapter.verify(&other, &plan, &candidate, &context),
        Err(PdfAdapterError::StalePlan)
    ));
}

#[test]
fn reparsable_unrequested_change_fails_byte_evidence() {
    let adapter = PdfAdapter::default();
    let context = context(1_000_000);
    let document = adapter.open(source(pdf("alpha")), &context).unwrap();
    let plan = adapter.plan(&document, &request(), &context).unwrap();
    let candidate = adapter.apply(&document, &plan, &context).unwrap();
    let mut tampered = candidate.input_source().as_bytes().to_vec();
    tampered[7] = b'6'; // Header stays valid and same-sized, outside text spans.
    let tampered = adapter.open(source(tampered), &context).unwrap();
    let report = adapter
        .verify(&document, &plan, &tampered, &context)
        .unwrap();
    assert!(report.validate(plan.required_checks()).is_err());
    assert!(
        report
            .checks()
            .iter()
            .any(|c| c.code.code() == "reparse" && c.status == CheckStatus::Passed)
    );
    assert!(
        report
            .checks()
            .iter()
            .any(|c| c.code.code() == "unchanged_outside_spans" && c.status == CheckStatus::Failed)
    );
    let unchanged = adapter
        .verify(&document, &plan, &document, &context)
        .unwrap();
    assert!(
        unchanged
            .checks()
            .iter()
            .any(|c| c.code.code() == "exact_replacements" && c.status == CheckStatus::Failed)
    );
}

#[test]
fn cancellation_and_shared_work_exhaustion_leave_source_usable() {
    let adapter = PdfAdapter::default();
    let generous = context(1_000_000);
    let document = adapter.open(source(pdf("alpha")), &generous).unwrap();
    let plan = adapter.plan(&document, &request(), &generous).unwrap();
    let cancelled = context(1_000_000);
    cancelled.cancellation().cancel();
    assert!(matches!(
        adapter.apply(&document, &plan, &cancelled),
        Err(PdfAdapterError::Context(ContextError::Cancelled))
    ));
    let limited = context(document.source_len() as u64);
    adapter
        .open(document.input_source().clone(), &limited)
        .unwrap();
    assert!(matches!(
        adapter.inspect(&document, &limited.clone()),
        Err(PdfAdapterError::Context(ContextError::Budget(_)))
    ));
    assert!(matches!(
        adapter.apply(&document, &plan, &limited),
        Err(PdfAdapterError::Context(ContextError::Budget(_)))
    ));
    assert_eq!(document.query_text("alpha", 0).unwrap().text, "alpha");
}

#[test]
fn graph_records_are_reserved_before_allocation_and_released_on_drop() {
    let adapter = PdfAdapter::default();
    let generous = context(1_000_000);
    let document = adapter.open(source(pdf("alpha")), &generous).unwrap();
    let no_memory = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 0,
            max_work_units: 1_000_000,
        }),
        CancellationToken::new(),
    );
    assert!(matches!(
        adapter.inspect(&document, &no_memory),
        Err(PdfAdapterError::Context(ContextError::Budget(_)))
    ));
    assert_eq!(no_memory.budget().snapshot().live_bytes, 0);
    assert!(no_memory.budget().snapshot().work_units > 0);
    let graph = adapter.inspect(&document, &generous).unwrap();
    let before_drop = generous.budget().snapshot();
    let expected = graph.nodes().count() * std::mem::size_of::<PdfGraphNodeId>()
        + graph.edges().count()
            * std::mem::size_of::<binas_core::graph::GraphEdge<PdfGraphNodeId>>();
    // Node records additionally retain optional provenance. Accounting includes
    // at least the public identities/edges and is released with the inspection.
    assert!(before_drop.live_bytes >= expected as u64);
    drop(graph);
    let after_drop = generous.budget().snapshot();
    assert_eq!(after_drop.live_bytes, 0);
    assert_eq!(after_drop.work_units, before_drop.work_units);
}
