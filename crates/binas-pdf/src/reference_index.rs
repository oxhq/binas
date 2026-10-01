//! Read-only incoming-reference census of parsed trailer and indirect values.
//!
//! Dictionary/array paths describe syntax, not semantic ownership. References
//! are recorded without following targets: cycles remain explicit finite edges,
//! and missing exact number/generation targets are marked dangling. This index
//! does not inspect references encoded inside opaque stream bytes.

use crate::{
    PdfDocument, PdfError,
    object::{ObjectRef, Value},
};
use binas_core::{
    OperationContext, Reservation, RevisionId, SnapshotId, capability::CapabilityStatus,
    transaction::RevisionState,
};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PdfReferenceTarget {
    pub number: u32,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfReferenceOrigin {
    Trailer,
    Object(PdfReferenceTarget),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfReferencePathStep {
    /// Raw PDF dictionary key bytes, without the slash.
    Key(Vec<u8>),
    Index(usize),
}

#[derive(Debug, Eq, PartialEq)]
pub struct PdfReference {
    pub origin: PdfReferenceOrigin,
    pub target: PdfReferenceTarget,
    pub path: Vec<PdfReferencePathStep>,
    pub dangling: bool,
}

#[derive(Debug)]
pub struct PdfReferenceIndex {
    snapshot: SnapshotId,
    records: Vec<PdfReference>,
    _reservation: Reservation,
}

impl PdfReferenceIndex {
    /// Charges each direct value visit on both census passes and conservative
    /// sorting work. Reserves logical record, path-step and copied key bytes
    /// before allocation. Allocator overhead/spare capacity, native PDF storage
    /// and bounded recursion stack are excluded from managed memory accounting.
    pub fn build(document: &PdfDocument, context: &OperationContext) -> Result<Self, PdfError> {
        Self::build_bound(
            document,
            SnapshotId::new(document.source_id(), RevisionId::INITIAL),
            context,
        )
    }

    /// Indexes an explicitly retained revision. Its source identity must name
    /// the current document bytes; a stale source binding is refused.
    pub fn build_for_revision(
        state: &RevisionState<PdfDocument>,
        context: &OperationContext,
    ) -> Result<Self, PdfError> {
        if state.snapshot().source() != state.value().source_id() {
            return Err(PdfError::selection(
                "reference index revision source does not match document bytes",
            ));
        }
        Self::build_bound(state.value(), state.snapshot(), context)
    }

    fn build_bound(
        document: &PdfDocument,
        snapshot: SnapshotId,
        context: &OperationContext,
    ) -> Result<Self, PdfError> {
        let mut sizing = Sizing::default();
        size_value(&document.parsed().trailer, 0, 0, &mut sizing, context)?;
        for object in document.parsed().objects.values() {
            size_value(&object.value, 0, 0, &mut sizing, context)?;
        }
        let reservation = context
            .reserve(sizing.bytes as u64)
            .map_err(PdfError::from_context)?;
        let scratch_bytes = sizing
            .depth
            .checked_mul(std::mem::size_of::<BorrowedStep<'_>>())
            .ok_or_else(|| PdfError::limit("reference path scratch overflows"))?;
        let _scratch = context
            .reserve(scratch_bytes as u64)
            .map_err(PdfError::from_context)?;
        let mut records = Vec::new();
        records
            .try_reserve_exact(sizing.records)
            .map_err(|_| PdfError::limit("reference record allocation failed"))?;
        let mut path = Vec::new();
        path.try_reserve_exact(sizing.depth)
            .map_err(|_| PdfError::limit("reference path allocation failed"))?;
        collect(
            &document.parsed().trailer,
            PdfReferenceOrigin::Trailer,
            &mut path,
            &mut records,
            document,
            context,
        )?;
        for (reference, object) in &document.parsed().objects {
            collect(
                &object.value,
                PdfReferenceOrigin::Object(target(*reference)),
                &mut path,
                &mut records,
                document,
                context,
            )?;
        }
        let levels = usize::BITS - sizing.records.leading_zeros();
        let sort_work = (sizing.records as u64)
            .checked_mul(u64::from(levels))
            .ok_or_else(|| PdfError::limit("reference sort work overflows"))?;
        context
            .charge_work(sort_work)
            .map_err(PdfError::from_context)?;
        records.sort_unstable_by_key(|record| record.target);
        context.checkpoint().map_err(PdfError::from_context)?;
        Ok(Self {
            snapshot,
            records,
            _reservation: reservation,
        })
    }

    /// Immutable byte-document identity, not a retained session lineage.
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    pub fn records(&self) -> &[PdfReference] {
        &self.records
    }

    /// Returns all distinct syntactic incoming occurrences, including repeated
    /// references from one origin at different paths. Full generation matches.
    pub fn incoming(
        &self,
        snapshot: SnapshotId,
        target: PdfReferenceTarget,
    ) -> Result<&[PdfReference], PdfError> {
        if snapshot != self.snapshot {
            return Err(PdfError::selection(
                "reference index source or revision mismatch",
            ));
        }
        let start = self
            .records
            .partition_point(|record| record.target < target);
        let end = self
            .records
            .partition_point(|record| record.target <= target);
        Ok(&self.records[start..end])
    }

    pub fn ownership_capability(&self) -> CapabilityStatus {
        CapabilityStatus::Unsupported {
            reason: "incoming links do not prove page-local or shared semantic ownership",
        }
    }
}

fn target(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}

#[derive(Default)]
struct Sizing {
    records: usize,
    bytes: usize,
    depth: usize,
}

fn size_value(
    value: &Value,
    depth: usize,
    key_bytes: usize,
    sizing: &mut Sizing,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    sizing.depth = sizing.depth.max(depth);
    match value {
        Value::Ref(_) => {
            let bytes = depth
                .checked_mul(std::mem::size_of::<PdfReferencePathStep>())
                .and_then(|n| n.checked_add(key_bytes))
                .and_then(|n| n.checked_add(std::mem::size_of::<PdfReference>()))
                .ok_or_else(|| PdfError::limit("reference record bytes overflow"))?;
            sizing.bytes = sizing
                .bytes
                .checked_add(bytes)
                .ok_or_else(|| PdfError::limit("reference census bytes overflow"))?;
            sizing.records = sizing
                .records
                .checked_add(1)
                .ok_or_else(|| PdfError::limit("reference count overflows"))?;
        }
        Value::Array(values) => {
            for value in values {
                size_value(value, next_depth(depth)?, key_bytes, sizing, context)?;
            }
        }
        Value::Dict(values) => {
            for (key, value) in values {
                size_value(
                    value,
                    next_depth(depth)?,
                    key_bytes
                        .checked_add(key.len())
                        .ok_or_else(|| PdfError::limit("reference path bytes overflow"))?,
                    sizing,
                    context,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn next_depth(depth: usize) -> Result<usize, PdfError> {
    depth
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("reference path depth overflows"))
}

enum BorrowedStep<'a> {
    Key(&'a [u8]),
    Index(usize),
}

fn collect<'a>(
    value: &'a Value,
    origin: PdfReferenceOrigin,
    path: &mut Vec<BorrowedStep<'a>>,
    records: &mut Vec<PdfReference>,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    match value {
        Value::Ref(reference) => {
            let mut owned_path = Vec::new();
            owned_path
                .try_reserve_exact(path.len())
                .map_err(|_| PdfError::limit("reference owned path allocation failed"))?;
            for step in path.iter() {
                context.charge_work(1).map_err(PdfError::from_context)?;
                owned_path.push(match step {
                    BorrowedStep::Index(index) => PdfReferencePathStep::Index(*index),
                    BorrowedStep::Key(key) => {
                        context
                            .charge_work(key.len() as u64)
                            .map_err(PdfError::from_context)?;
                        let mut bytes = Vec::new();
                        bytes
                            .try_reserve_exact(key.len())
                            .map_err(|_| PdfError::limit("reference key allocation failed"))?;
                        bytes.extend_from_slice(key);
                        PdfReferencePathStep::Key(bytes)
                    }
                });
            }
            records.push(PdfReference {
                origin,
                target: target(*reference),
                path: owned_path,
                dangling: !document.parsed().objects.contains_key(reference),
            });
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                path.push(BorrowedStep::Index(index));
                collect(value, origin, path, records, document, context)?;
                path.pop();
            }
        }
        Value::Dict(values) => {
            for (key, value) in values {
                path.push(BorrowedStep::Key(key));
                collect(value, origin, path, records, document, context)?;
                path.pop();
            }
        }
        _ => {}
    }
    Ok(())
}
