//! A deliberately tiny test adapter, not another production format.
use std::{error::Error, fmt};

use binas_core::{
    Adapter, BudgetLimits, CancellationToken, OperationContext, ResourceBudget, RevisionId,
    SnapshotId, Source, Span,
    capability::{Capability, CapabilityStatus, Detection},
    graph::{EdgeKind, GraphEdge, GraphNode, GraphProjection},
    source::PayloadProvenance,
};

#[derive(Debug)]
struct Failure(String);
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl Error for Failure {}
fn failure(error: impl fmt::Display) -> Failure {
    Failure(error.to_string())
}

#[derive(Clone)]
struct Record {
    tag: u8,
    value: Vec<u8>,
    provenance: PayloadProvenance,
}
#[derive(Clone)]
struct Document {
    source: Source,
    snapshot: SnapshotId,
    records: Vec<Record>,
}
struct Request {
    index: usize,
    replacement: Vec<u8>,
}
struct Plan {
    snapshot: SnapshotId,
    index: usize,
    replacement: Vec<u8>,
}
struct Inspection {
    snapshot: SnapshotId,
    provenance: Vec<PayloadProvenance>,
    references: Vec<GraphEdge<usize>>,
}

impl GraphProjection for Inspection {
    type NodeId = usize;
    fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    fn nodes(&self) -> impl Iterator<Item = GraphNode<'_, usize>> {
        std::iter::once(GraphNode {
            id: 0,
            kind: "tlv.root",
            provenance: None,
        })
        .chain(
            self.provenance
                .iter()
                .enumerate()
                .flat_map(|(index, provenance)| {
                    [
                        GraphNode {
                            id: index * 2 + 1,
                            kind: "tlv.record",
                            provenance: None,
                        },
                        GraphNode {
                            id: index * 2 + 2,
                            kind: "tlv.payload",
                            provenance: Some(provenance),
                        },
                    ]
                }),
        )
    }
    fn edges(&self) -> impl Iterator<Item = GraphEdge<usize>> {
        (0..self.provenance.len())
            .flat_map(|index| {
                [
                    GraphEdge {
                        from: 0,
                        to: index * 2 + 1,
                        kind: EdgeKind::Containment,
                    },
                    GraphEdge {
                        from: index * 2 + 1,
                        to: index * 2 + 2,
                        kind: EdgeKind::Ownership,
                    },
                ]
            })
            .chain(self.references.iter().cloned())
    }
}

struct Tlv;
impl Adapter for Tlv {
    type Document = Document;
    type Inspection = Inspection;
    type Request = Request;
    type Plan = Plan;
    type Verification = bool;
    type Error = Failure;

    fn capabilities(&self) -> &[Capability] {
        &[
            Capability {
                id: "tlv.replace",
                status: CapabilityStatus::Supported,
            },
            Capability {
                id: "tlv.insert",
                status: CapabilityStatus::Unsupported {
                    reason: "test adapter supports replacement only",
                },
            },
        ]
    }
    fn detect(&self, prefix: &[u8], context: &OperationContext) -> Result<Detection, Failure> {
        context
            .charge_work(prefix.len().min(3) as u64)
            .map_err(failure)?;
        Ok(if prefix.len() < 3 {
            Detection::Inconclusive
        } else if prefix.starts_with(b"TLV") {
            Detection::Recognized
        } else {
            Detection::Unrecognized
        })
    }
    fn open(&self, source: Source, context: &OperationContext) -> Result<Document, Failure> {
        context.charge_work(source.len() as u64).map_err(failure)?;
        if !source.as_bytes().starts_with(b"TLV") {
            return Err(failure("not TLV"));
        }
        let mut records = Vec::new();
        let mut cursor = 3;
        while cursor < source.len() {
            context.checkpoint().map_err(failure)?;
            let header = source
                .as_bytes()
                .get(cursor..cursor + 2)
                .ok_or_else(|| failure("truncated header"))?;
            let length = usize::from(header[1]);
            let end = cursor + 2 + length;
            let range = source
                .qualified_span(Span::new((cursor + 2) as u64, end as u64).map_err(failure)?)
                .map_err(failure)?;
            let value = range.resolve(&source).map_err(failure)?.to_vec();
            if header[0] == 2 && (value.len() != 1 || usize::from(value[0]) >= records.len()) {
                return Err(failure("invalid backwards reference"));
            }
            records.push(Record {
                tag: header[0],
                value,
                provenance: PayloadProvenance::Exact(range),
            });
            cursor = end;
        }
        Ok(Document {
            snapshot: SnapshotId::new(source.id(), RevisionId::INITIAL),
            source,
            records,
        })
    }
    fn snapshot(&self, document: &Document) -> SnapshotId {
        document.snapshot
    }
    fn inspect(
        &self,
        document: &Document,
        context: &OperationContext,
    ) -> Result<Inspection, Failure> {
        context
            .charge_work(document.records.len() as u64)
            .map_err(failure)?;
        Ok(Inspection {
            snapshot: document.snapshot,
            provenance: document
                .records
                .iter()
                .map(|record| record.provenance.clone())
                .collect(),
            references: document
                .records
                .iter()
                .enumerate()
                .filter(|(_, record)| record.tag == 2)
                .map(|(index, record)| GraphEdge {
                    from: index * 2 + 2,
                    to: usize::from(record.value[0]) * 2 + 2,
                    kind: EdgeKind::Reference,
                })
                .collect(),
        })
    }
    fn plan(
        &self,
        document: &Document,
        request: &Request,
        context: &OperationContext,
    ) -> Result<Plan, Failure> {
        context.charge_work(1).map_err(failure)?;
        let record = document
            .records
            .get(request.index)
            .ok_or_else(|| failure("unknown selection"))?;
        if record.tag == 2 {
            return Err(failure("reference replacement unsupported"));
        }
        Ok(Plan {
            snapshot: document.snapshot,
            index: request.index,
            replacement: request.replacement.clone(),
        })
    }
    fn apply(
        &self,
        document: &Document,
        plan: &Plan,
        context: &OperationContext,
    ) -> Result<Document, Failure> {
        context.checkpoint().map_err(failure)?;
        plan.snapshot
            .ensure_current(document.snapshot)
            .map_err(failure)?;
        context
            .charge_work(plan.replacement.len() as u64)
            .map_err(failure)?;
        let mut candidate = document.clone();
        let record = &mut candidate.records[plan.index];
        let inputs = match &record.provenance {
            PayloadProvenance::Exact(input) => vec![*input],
            PayloadProvenance::Derived { inputs, .. } => inputs.clone(),
        };
        record.value = plan.replacement.clone();
        record.provenance = PayloadProvenance::Derived {
            inputs,
            transform: "tlv.replace".into(),
        };
        candidate.snapshot = SnapshotId::new(
            document.source.id(),
            document
                .snapshot
                .revision()
                .checked_next()
                .ok_or_else(|| failure("revision exhausted"))?,
        );
        Ok(candidate)
    }
    fn verify(
        &self,
        original: &Document,
        plan: &Plan,
        candidate: &Document,
        context: &OperationContext,
    ) -> Result<bool, Failure> {
        context
            .charge_work(original.records.len() as u64)
            .map_err(failure)?;
        plan.snapshot
            .ensure_current(original.snapshot)
            .map_err(failure)?;
        Ok(candidate.source.id() == original.source.id()
            && candidate.snapshot.revision()
                == original.snapshot.revision().checked_next().unwrap()
            && candidate.records.len() == original.records.len()
            && original
                .records
                .iter()
                .zip(&candidate.records)
                .enumerate()
                .all(|(index, (before, after))| {
                    before.tag == after.tag
                        && if index == plan.index {
                            after.value == plan.replacement
                        } else {
                            before.value == after.value
                        }
                }))
    }
}

fn context(work: u64) -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1024,
            max_work_units: work,
        }),
        CancellationToken::new(),
    )
}
fn input(value: u8) -> Source {
    Source::from_owned(vec![b'T', b'L', b'V', 1, 1, value, 2, 1, 0], 9).unwrap()
}

#[test]
fn associated_lifecycle_preserves_source_and_projects_distinct_edges() {
    let context = context(100);
    let source = input(b'x');
    let retained = source.clone();
    assert_eq!(
        Tlv.detect(source.as_bytes(), &context).unwrap(),
        Detection::Recognized
    );
    let original = Tlv.open(source, &context).unwrap();
    let graph = Tlv.inspect(&original, &context).unwrap();
    assert_eq!(graph.snapshot(), Tlv.snapshot(&original));
    assert_eq!(graph.nodes().count(), 5);
    let edges: Vec<_> = graph.edges().collect();
    assert!(edges.contains(&GraphEdge {
        from: 0,
        to: 1,
        kind: EdgeKind::Containment
    }));
    assert!(edges.contains(&GraphEdge {
        from: 1,
        to: 2,
        kind: EdgeKind::Ownership
    }));
    assert!(edges.contains(&GraphEdge {
        from: 4,
        to: 2,
        kind: EdgeKind::Reference
    }));
    assert!(original.source.shares_storage_with(&retained));
    let plan = Tlv
        .plan(
            &original,
            &Request {
                index: 0,
                replacement: b"updated".to_vec(),
            },
            &context,
        )
        .unwrap();
    let candidate = Tlv.apply(&original, &plan, &context).unwrap();
    assert!(Tlv.verify(&original, &plan, &candidate, &context).unwrap());
    assert_eq!(original.records[0].value, b"x");
    assert_eq!(retained.as_bytes(), input(b'x').as_bytes());
    assert!(matches!(
        candidate.records[0].provenance,
        PayloadProvenance::Derived { .. }
    ));
    let mut corrupted = candidate.clone();
    corrupted.records[1].value = vec![255];
    assert!(!Tlv.verify(&original, &plan, &corrupted, &context).unwrap());
    assert!(Tlv.apply(&candidate, &plan, &context).is_err());
    let replacement = Tlv.open(input(b'y'), &context).unwrap();
    assert!(Tlv.apply(&replacement, &plan, &context).is_err());
}

#[test]
fn context_clones_share_work_and_cancellation_prevents_staging() {
    let shared = context(18);
    let original = Tlv.open(input(b'x'), &shared).unwrap();
    let second = Tlv.open(input(b'x'), &shared.clone()).unwrap();
    drop(second);
    assert!(Tlv.open(input(b'x'), &shared).is_err());
    assert_eq!(shared.budget().snapshot().work_units, 18);
    let operation = context(100);
    let plan = Tlv
        .plan(
            &original,
            &Request {
                index: 0,
                replacement: vec![b'y'],
            },
            &operation,
        )
        .unwrap();
    operation.cancellation().cancel();
    assert!(Tlv.apply(&original, &plan, &operation).is_err());
    assert_eq!(original.records[0].value, b"x");
    assert_eq!(original.snapshot.revision(), RevisionId::INITIAL);
}

#[test]
fn missing_capability_and_bounded_prefix_detection_are_explicit() {
    assert!(matches!(
        Tlv.capabilities()[1].status,
        CapabilityStatus::Unsupported { .. }
    ));
    assert_eq!(
        Tlv.detect(b"T", &context(1)).unwrap(),
        Detection::Inconclusive
    );
    assert_eq!(
        Tlv.detect(b"BAD", &context(3)).unwrap(),
        Detection::Unrecognized
    );
    assert!(
        Tlv.open(
            Source::copy_from_slice(b"TLV\x01\x05x", 6).unwrap(),
            &context(100)
        )
        .is_err()
    );
}
