//! Initial generic lifecycle for source-bound surgical text batches.
//!
//! Staging uses the existing byte writer and reopens output; it is not retained
//! COW. Work accounting charges input bytes, request counts and visited graph
//! values. Parser and decode checkpoints share this cancellation token; decode
//! work has separate retained document limits. Some legacy edit loops are not
//! yet interruptible or fully charged to this context. No whole-process
//! memory, semantic preservation or ownership guarantee is advertised here.

use std::{error::Error, fmt};

use binas_core::{
    Adapter, ContextError, OperationContext, PayloadProvenance, Reservation, RevisionId,
    SnapshotId, Source,
    capability::{Capability, CapabilityStatus, Detection},
    graph::{EdgeKind, GraphEdge, GraphNode, GraphProjection},
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};

use crate::{
    BatchTextEditPlan, BatchTextEditRequest, OpenOptions, PdfDocument, PdfEngine, PdfError,
    edit::encode_same_span, object::Value,
};

#[derive(Debug)]
pub enum PdfAdapterError {
    Pdf(PdfError),
    Context(ContextError),
    StalePlan,
}
impl fmt::Display for PdfAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pdf(e) => e.fmt(f),
            Self::Context(e) => e.fmt(f),
            Self::StalePlan => f.write_str("PDF adapter plan does not match the source snapshot"),
        }
    }
}
impl Error for PdfAdapterError {}
impl From<PdfError> for PdfAdapterError {
    fn from(value: PdfError) -> Self {
        Self::Pdf(value)
    }
}
impl From<ContextError> for PdfAdapterError {
    fn from(value: ContextError) -> Self {
        Self::Context(value)
    }
}

#[derive(Clone, Debug)]
pub struct PdfAdapterPlan {
    snapshot: SnapshotId,
    batch: BatchTextEditPlan,
    required: RequiredChecks,
}
impl PdfAdapterPlan {
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn required_checks(&self) -> &RequiredChecks {
        &self.required
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfGraphNodeId {
    Trailer,
    Object { number: u32, generation: u16 },
    EncodedStream { number: u32, generation: u16 },
}

#[derive(Debug)]
struct NodeRecord {
    id: PdfGraphNodeId,
    provenance: Option<PayloadProvenance>,
}

#[derive(Debug)]
pub struct PdfGraphInspection {
    snapshot: SnapshotId,
    nodes: Vec<NodeRecord>,
    edges: Vec<GraphEdge<PdfGraphNodeId>>,
    // Accounts for graph node/edge record storage only, not allocator overhead,
    // native PDF storage, recursion stack or whole-process memory.
    _records: Reservation,
}
impl GraphProjection for PdfGraphInspection {
    type NodeId = PdfGraphNodeId;
    fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    fn nodes(&self) -> impl Iterator<Item = GraphNode<'_, Self::NodeId>> {
        self.nodes.iter().map(|record| GraphNode {
            id: record.id,
            kind: match record.id {
                PdfGraphNodeId::Trailer => "pdf.trailer",
                PdfGraphNodeId::Object { .. } => "pdf.indirect_object",
                PdfGraphNodeId::EncodedStream { .. } => "pdf.encoded_stream",
            },
            // Exact ranges describe only source-backed encoded payloads, never
            // a changed dictionary or a decoded object-stream member.
            provenance: record.provenance.as_ref(),
        })
    }
    fn edges(&self) -> impl Iterator<Item = GraphEdge<Self::NodeId>> {
        self.edges.iter().cloned()
    }
}

#[derive(Clone, Debug, Default)]
pub struct PdfAdapter {
    engine: PdfEngine,
}
impl PdfAdapter {
    pub fn new(engine: PdfEngine) -> Self {
        Self { engine }
    }
    fn current(
        &self,
        document: &PdfDocument,
        plan: &PdfAdapterPlan,
    ) -> Result<(), PdfAdapterError> {
        if self.snapshot(document) != plan.snapshot {
            return Err(PdfAdapterError::StalePlan);
        }
        Ok(())
    }
}

const CHECKS: [&str; 4] = [
    "unchanged_outside_spans",
    "exact_replacements",
    "reparse",
    "page_relationships",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.surgical", name).expect("static PDF check code")
}

impl Adapter for PdfAdapter {
    type Document = PdfDocument;
    type Inspection = PdfGraphInspection;
    type Request = BatchTextEditRequest;
    type Plan = PdfAdapterPlan;
    type Verification = EvidenceReport<String>;
    type Error = PdfAdapterError;

    fn capabilities(&self) -> &[Capability] {
        &[
            Capability {
                id: "pdf.text.surgical_batch",
                status: CapabilityStatus::Supported,
            },
            Capability {
                id: "pdf.graph.references",
                status: CapabilityStatus::Supported,
            },
            Capability {
                id: "pdf.graph.ownership",
                status: CapabilityStatus::Unsupported {
                    reason: "reference projection does not establish ownership",
                },
            },
        ]
    }
    fn detect(&self, prefix: &[u8], context: &OperationContext) -> Result<Detection, Self::Error> {
        let prefix = &prefix[..prefix.len().min(1028)];
        context.charge_work(prefix.len() as u64)?;
        Ok(if prefix.windows(5).take(1024).any(|v| v == b"%PDF-") {
            Detection::Recognized
        } else if prefix.len() < 1028 {
            Detection::Inconclusive
        } else {
            Detection::Unrecognized
        })
    }
    fn open(&self, source: Source, context: &OperationContext) -> Result<PdfDocument, Self::Error> {
        context.charge_work(source.len() as u64)?;
        let document = self.engine.open_source_with_cancellation(
            source,
            OpenOptions::default(),
            context.cancellation().clone(),
        )?;
        context.checkpoint()?;
        Ok(document)
    }
    fn snapshot(&self, document: &PdfDocument) -> SnapshotId {
        // Each immutable byte document is content-addressed; retained session
        // revision publication is a separate consumer/transaction responsibility.
        SnapshotId::new(document.source_id(), RevisionId::INITIAL)
    }
    fn inspect(
        &self,
        document: &PdfDocument,
        context: &OperationContext,
    ) -> Result<PdfGraphInspection, Self::Error> {
        context.charge_work(1)?;
        let stream_count = document
            .parsed()
            .objects
            .values()
            .filter(|object| object.stream.is_some())
            .count();
        let node_count = document
            .parsed()
            .objects
            .len()
            .checked_add(1)
            .and_then(|count| count.checked_add(stream_count))
            .ok_or_else(|| PdfError::limit("graph node count overflows"))?;
        // Count first without allocating graph records, so the live-byte limit
        // is checked before both vectors allocate. Both traversals charge work.
        let mut edge_count = stream_count;
        count_references(&document.parsed().trailer, &mut edge_count, context)?;
        for object in document.parsed().objects.values() {
            count_references(&object.value, &mut edge_count, context)?;
        }
        let record_bytes = node_count
            .checked_mul(std::mem::size_of::<NodeRecord>())
            .and_then(|nodes| {
                edge_count
                    .checked_mul(std::mem::size_of::<GraphEdge<PdfGraphNodeId>>())
                    .and_then(|edges| nodes.checked_add(edges))
            })
            .ok_or_else(|| PdfError::limit("graph record bytes overflow"))?;
        let records = context.reserve(record_bytes as u64)?;
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(node_count)
            .map_err(|_| PdfError::limit("graph node allocation failed"))?;
        let mut edges = Vec::new();
        edges
            .try_reserve_exact(edge_count)
            .map_err(|_| PdfError::limit("graph edge allocation failed"))?;
        nodes.push(NodeRecord {
            id: PdfGraphNodeId::Trailer,
            provenance: None,
        });
        let mut graph = PdfGraphInspection {
            snapshot: self.snapshot(document),
            nodes,
            edges,
            _records: records,
        };
        references(
            &document.parsed().trailer,
            PdfGraphNodeId::Trailer,
            &mut graph.edges,
            context,
        )?;
        for (reference, object) in &document.parsed().objects {
            context.charge_work(1)?;
            let id = PdfGraphNodeId::Object {
                number: reference.number,
                generation: reference.generation,
            };
            graph.nodes.push(NodeRecord {
                id,
                provenance: None,
            });
            if let Some(stream) = &object.stream {
                let stream_id = PdfGraphNodeId::EncodedStream {
                    number: reference.number,
                    generation: reference.generation,
                };
                graph.nodes.push(NodeRecord {
                    id: stream_id,
                    provenance: stream.source_span().map(PayloadProvenance::Exact),
                });
                graph.edges.push(GraphEdge {
                    from: id,
                    to: stream_id,
                    kind: EdgeKind::Containment,
                });
            }
            references(&object.value, id, &mut graph.edges, context)?;
        }
        Ok(graph)
    }
    fn plan(
        &self,
        document: &PdfDocument,
        request: &BatchTextEditRequest,
        context: &OperationContext,
    ) -> Result<PdfAdapterPlan, Self::Error> {
        context.charge_work(document.source_len() as u64)?;
        context.charge_work(request.edits.len() as u64)?;
        let batch = document
            .with_cancellation(context.cancellation().clone())
            .plan_batch_text_edits(request.clone())?;
        context.checkpoint()?;
        Ok(PdfAdapterPlan {
            snapshot: self.snapshot(document),
            batch,
            required: RequiredChecks::new(CHECKS.iter().map(|name| code(name)).collect())
                .expect("static obligations"),
        })
    }
    fn apply(
        &self,
        document: &PdfDocument,
        plan: &PdfAdapterPlan,
        context: &OperationContext,
    ) -> Result<PdfDocument, Self::Error> {
        self.current(document, plan)?;
        context.charge_work(document.source_len() as u64)?;
        let result = document
            .with_cancellation(context.cancellation().clone())
            .apply_batch_text_edits(plan.batch.clone())?;
        context.checkpoint()?;
        context.charge_work(result.bytes.len() as u64)?;
        let source = Source::from_owned(
            result.bytes,
            document.engine_config().limits.max_input_bytes,
        )
        .map_err(|error| PdfError::limit(error.to_string()))?;
        let candidate = PdfEngine::new(document.engine_config().clone())
            .open_source_with_cancellation(
                source,
                OpenOptions::default(),
                context.cancellation().clone(),
            )?;
        context.checkpoint()?;
        Ok(candidate)
    }
    fn verify(
        &self,
        original: &PdfDocument,
        plan: &PdfAdapterPlan,
        candidate: &PdfDocument,
        context: &OperationContext,
    ) -> Result<Self::Verification, Self::Error> {
        self.current(original, plan)?;
        context.charge_work(original.source_len() as u64)?;
        context.charge_work(candidate.source_len() as u64)?;
        let before = original.source();
        let after = candidate.source();
        let mut outside = before.len() == after.len();
        let mut replacements = true;
        let mut cursor = 0;
        for edit in &plan.batch.edits {
            context.charge_work(1)?;
            let start = edit.span.start() as usize;
            let end = edit.span.end() as usize;
            outside &= after.get(cursor..start) == before.get(cursor..start);
            let expected = encode_same_span(&edit.replacement, &before[start..end])?;
            replacements &= after.get(start..end) == Some(expected.as_slice());
            cursor = end;
        }
        outside &= after.get(cursor..) == before.get(cursor..);
        context.checkpoint()?;
        let reopened = PdfEngine::new(original.engine_config().clone())
            .open_source_with_cancellation(
                candidate.input_source().clone(),
                OpenOptions::default(),
                context.cancellation().clone(),
            );
        let reparse = reopened.is_ok();
        let page_relationships = match reopened {
            Ok(ref document) => document.page_refs()? == original.page_refs()?,
            Err(_) => false,
        };
        context.checkpoint()?;
        Ok(EvidenceReport::new(CHECKS.into_iter().zip([outside, replacements, reparse, page_relationships]).map(|(name, passed)| CheckEvidence {
            code: code(name), status: if passed { CheckStatus::Passed } else { CheckStatus::Failed },
            detail: format!("{name}: scoped surgical-byte/structural check; no universal semantic preservation assertion"),
        }).collect()))
    }
}

fn count_references(
    value: &Value,
    count: &mut usize,
    context: &OperationContext,
) -> Result<(), PdfAdapterError> {
    context.charge_work(1)?;
    match value {
        Value::Ref(_) => {
            *count = count
                .checked_add(1)
                .ok_or_else(|| PdfError::limit("graph edge count overflows"))?
        }
        Value::Array(values) => {
            for value in values {
                count_references(value, count, context)?;
            }
        }
        Value::Dict(values) => {
            for value in values.values() {
                count_references(value, count, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn references(
    value: &Value,
    owner: PdfGraphNodeId,
    edges: &mut Vec<GraphEdge<PdfGraphNodeId>>,
    context: &OperationContext,
) -> Result<(), PdfAdapterError> {
    context.charge_work(1)?;
    match value {
        Value::Ref(reference) => edges.push(GraphEdge {
            from: owner,
            to: PdfGraphNodeId::Object {
                number: reference.number,
                generation: reference.generation,
            },
            kind: EdgeKind::Reference,
        }),
        Value::Array(values) => {
            for value in values {
                references(value, owner, edges, context)?;
            }
        }
        Value::Dict(values) => {
            for value in values.values() {
                references(value, owner, edges, context)?;
            }
        }
        _ => {}
    }
    Ok(())
}
