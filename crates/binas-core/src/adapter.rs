//! Small format adapter lifecycle, preserving native associated types.

use crate::{
    capability::{Capability, Detection},
    context::OperationContext,
    graph::GraphProjection,
    identity::SnapshotId,
    source::Source,
};

/// Opening/inspection/planning must leave source bytes and the supplied document
/// unchanged. `apply` stages a separate candidate; it does not publish a revision.
/// Plans must bind the document snapshot and reject stale sources/revisions.
/// Verification compares original + requested effects + candidate. Its native
/// result must distinguish unsupported checks from passes. Consumers decide
/// acceptance and publication using those results.
///
/// Implementations must checkpoint and charge shared context budgets for work
/// they perform. A context parameter alone is not proof of complete accounting
/// or cancellation support. Detection receives a bounded prefix, not ownership
/// of an entire decoded format representation.
pub trait Adapter {
    type Document;
    type Inspection: GraphProjection;
    type Request;
    type Plan;
    type Verification;
    type Error: std::error::Error;

    fn capabilities(&self) -> &[Capability];
    fn detect(&self, prefix: &[u8], context: &OperationContext) -> Result<Detection, Self::Error>;
    fn open(
        &self,
        source: Source,
        context: &OperationContext,
    ) -> Result<Self::Document, Self::Error>;
    fn snapshot(&self, document: &Self::Document) -> SnapshotId;
    fn inspect(
        &self,
        document: &Self::Document,
        context: &OperationContext,
    ) -> Result<Self::Inspection, Self::Error>;
    fn plan(
        &self,
        document: &Self::Document,
        request: &Self::Request,
        context: &OperationContext,
    ) -> Result<Self::Plan, Self::Error>;
    fn apply(
        &self,
        document: &Self::Document,
        plan: &Self::Plan,
        context: &OperationContext,
    ) -> Result<Self::Document, Self::Error>;
    fn verify(
        &self,
        original: &Self::Document,
        plan: &Self::Plan,
        candidate: &Self::Document,
        context: &OperationContext,
    ) -> Result<Self::Verification, Self::Error>;
}
