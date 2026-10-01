//! Atomic publication of a caller-owned candidate, with source-bound intent.
//! These types do not supply a format's mutation model or preservation rules.

use crate::{
    context::{ContextError, OperationContext},
    identity::SnapshotId,
    verification::{EvidenceError, EvidenceReport, RequiredChecks},
};
use std::{error::Error, fmt};

#[derive(Clone, Debug)]
pub struct PlanEnvelope<I> {
    snapshot: SnapshotId,
    intent: I,
    required: RequiredChecks,
}

impl<I> PlanEnvelope<I> {
    pub fn new(snapshot: SnapshotId, intent: I, required: RequiredChecks) -> Self {
        Self {
            snapshot,
            intent,
            required,
        }
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn intent(&self) -> &I {
        &self.intent
    }
    pub fn required(&self) -> &RequiredChecks {
        &self.required
    }
}

/// The caller must ensure `T` exposes immutable snapshot semantics through `&T`.
/// Interior mutation of committed data from a callback would violate this contract.
#[derive(Debug)]
pub struct RevisionState<T> {
    snapshot: SnapshotId,
    value: T,
}

impl<T> RevisionState<T> {
    pub fn new(snapshot: SnapshotId, value: T) -> Self {
        Self { snapshot, value }
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn value(&self) -> &T {
        &self.value
    }
    pub fn into_value(self) -> T {
        self.value
    }
}

#[derive(Debug)]
pub struct StagedChange<T, C> {
    pub candidate: T,
    pub changes: C,
}

/// Evidence is tied to the before/after snapshot and adapter-owned change data.
/// It does not claim an output-byte hash or checks not listed in this receipt.
#[derive(Debug)]
pub struct CommitReceipt<C, D> {
    pub before: SnapshotId,
    pub after: SnapshotId,
    pub required: RequiredChecks,
    pub changes: C,
    pub evidence: EvidenceReport<D>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum TransactionError<E> {
    StalePlan {
        planned: SnapshotId,
        current: SnapshotId,
    },
    RevisionOverflow,
    Context(ContextError),
    Stage(E),
    Check(E),
    Evidence(EvidenceError),
}

impl<E: fmt::Display> fmt::Display for TransactionError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StalePlan { .. } => {
                f.write_str("plan does not match the current source and revision")
            }
            Self::RevisionOverflow => f.write_str("revision counter exhausted"),
            Self::Context(error) => error.fmt(f),
            Self::Stage(error) => write!(f, "staging failed: {error}"),
            Self::Check(error) => write!(f, "checking failed: {error}"),
            Self::Evidence(error) => error.fmt(f),
        }
    }
}

impl<E: Error + 'static> Error for TransactionError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Context(error) => Some(error),
            Self::Stage(error) | Self::Check(error) => Some(error),
            Self::Evidence(error) => Some(error),
            Self::StalePlan { .. } | Self::RevisionOverflow => None,
        }
    }
}

/// Stage and check have no mutable access to the committed value. The candidate
/// is published only after checking obligations and a final cancellation poll.
/// Cancellation is cooperative: cancellation after that poll may follow commit.
/// A reused successful plan is stale, and failed attempts never advance state.
/// Callbacks must not publish external side effects or mutate shared source data.
pub fn execute_plan<T, I, C, D, E>(
    state: &mut RevisionState<T>,
    plan: &PlanEnvelope<I>,
    context: &OperationContext,
    stage: impl FnOnce(&T, &I, &OperationContext) -> Result<StagedChange<T, C>, E>,
    check: impl FnOnce(&T, &T, &I, &C, &OperationContext) -> Result<EvidenceReport<D>, E>,
) -> Result<CommitReceipt<C, D>, TransactionError<E>> {
    if state.snapshot != plan.snapshot {
        return Err(TransactionError::StalePlan {
            planned: plan.snapshot,
            current: state.snapshot,
        });
    }
    let revision = state
        .snapshot
        .revision()
        .checked_next()
        .ok_or(TransactionError::RevisionOverflow)?;
    context.checkpoint().map_err(TransactionError::Context)?;
    let staged = stage(&state.value, &plan.intent, context).map_err(TransactionError::Stage)?;
    context.checkpoint().map_err(TransactionError::Context)?;
    let evidence = check(
        &state.value,
        &staged.candidate,
        &plan.intent,
        &staged.changes,
        context,
    )
    .map_err(TransactionError::Check)?;
    context.checkpoint().map_err(TransactionError::Context)?;
    evidence
        .validate(&plan.required)
        .map_err(TransactionError::Evidence)?;
    let after = SnapshotId::new(state.snapshot.source(), revision);
    let receipt = CommitReceipt {
        before: state.snapshot,
        after,
        required: plan.required.clone(),
        changes: staged.changes,
        evidence,
    };
    context.checkpoint().map_err(TransactionError::Context)?;
    *state = RevisionState {
        snapshot: after,
        value: staged.candidate,
    };
    Ok(receipt)
}
