//! Read-only identity/relationship projections of adapter-owned representations.

use crate::{identity::SnapshotId, source::PayloadProvenance};

/// These meanings are deliberately separate: a reference need not imply
/// containment, exclusive ownership, or permission to mutate the target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeKind {
    Containment,
    Reference,
    Ownership,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GraphEdge<Id> {
    pub from: Id,
    pub to: Id,
    pub kind: EdgeKind,
}

#[derive(Clone, Debug)]
pub struct GraphNode<'a, Id> {
    pub id: Id,
    /// Adapter-defined kind, not a copy of the complete native object value.
    pub kind: &'a str,
    pub provenance: Option<&'a PayloadProvenance>,
}

/// Implementations describe coverage separately if their projection is partial.
/// Node identifiers and edges have meaning only within `snapshot()`; callers
/// must not reuse them against a different snapshot without revalidation.
pub trait GraphProjection {
    type NodeId: Clone + Eq;

    fn snapshot(&self) -> SnapshotId;
    fn nodes(&self) -> impl Iterator<Item = GraphNode<'_, Self::NodeId>>;
    fn edges(&self) -> impl Iterator<Item = GraphEdge<Self::NodeId>>;
}
