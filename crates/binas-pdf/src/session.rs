//! One retained PDF session for the raw ASCII P2b text-edit contract.
//!
//! The session accounts for original input bytes until close/drop, one maximum
//! output-capacity reservation while a prepared edit is pending, and a transient
//! maximum output-capacity reservation during export. Existing replacement-byte
//! reservations and coarse work charges remain in force. Decoded caches have
//! separate PDF limits; object maps, parser/encoder scratch, returned sources and
//! JS copies are not whole-process memory accounting.
//!
//! Handles carry no PDF data. Close/drop releases pending candidates even when
//! callers retain cloned handles. Acceptance admits one coarse work unit before
//! consuming the pending candidate, then uses the existing acceptance contract
//! (including its own work charge). An admission failure retains the plan; once
//! acceptance starts, success or rejection consumes it. Fresh cancellation tokens
//! retain the same cumulative budget and document decode work.

use std::{error::Error, fmt, sync::Arc};

use binas_core::{
    CancellationToken, ContextError, OperationContext, Reservation, SnapshotId, Source,
    transaction::TransactionError,
};

use crate::{
    EngineConfig, OpenOptions, PdfEngine, PdfError, PdfPreparedTextEdit, PdfRevision,
    PdfRevisionExport, PdfTextEditError, PdfTextEditReceipt, PdfWriterMode,
    SurgicalTextEditRequest, TextExtraction,
};

/// Opaque session ownership and generation; cloning does not retain PDF state.
#[derive(Clone, Debug)]
pub struct PdfSessionPlanHandle {
    owner: Arc<()>,
    snapshot: SnapshotId,
    generation: u64,
}

#[derive(Debug)]
pub enum PdfSessionError {
    Closed,
    ForeignPlan,
    StalePlan {
        planned: SnapshotId,
        current: SnapshotId,
    },
    PlanReleased,
    PlanPending,
    Context(ContextError),
    Pdf(PdfError),
    Text(PdfTextEditError),
}

impl PdfSessionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Closed => "session_closed",
            Self::ForeignPlan => "foreign_plan",
            Self::StalePlan { .. } => "stale_plan",
            Self::PlanReleased => "plan_released",
            Self::PlanPending => "plan_pending",
            Self::Context(error) => context_code(error),
            Self::Pdf(error) => error.code.as_str(),
            Self::Text(error) => match error {
                PdfTextEditError::Refused(_) => "unsupported_feature",
                PdfTextEditError::ContractVersion { .. } => "contract_version",
                PdfTextEditError::RequestMismatch => "request_mismatch",
                PdfTextEditError::Stale { .. } => "stale_plan",
                PdfTextEditError::ForeignRevision => "foreign_plan",
                PdfTextEditError::OutputBinding => "output_binding",
                PdfTextEditError::Context(error) => context_code(error),
                PdfTextEditError::Pdf(error) => error.code.as_str(),
                PdfTextEditError::Evidence(_) => "verification_failed",
                PdfTextEditError::Transaction(error) => match error {
                    TransactionError::StalePlan { .. } => "stale_plan",
                    TransactionError::RevisionOverflow => "resource_limit",
                    TransactionError::Context(error) => context_code(error),
                    TransactionError::Stage(error) | TransactionError::Check(error) => {
                        error.code.as_str()
                    }
                    TransactionError::Evidence(_) => "verification_failed",
                },
            },
        }
    }
}

fn context_code(error: &ContextError) -> &'static str {
    match error {
        ContextError::Cancelled => "cancelled",
        ContextError::Budget(_) => "resource_limit",
    }
}

impl fmt::Display for PdfSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Context(error) => error.fmt(formatter),
            Self::Pdf(error) => error.fmt(formatter),
            Self::Text(error) => error.fmt(formatter),
            _ => formatter.write_str(self.code()),
        }
    }
}

impl Error for PdfSessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Context(error) => Some(error),
            Self::Pdf(error) => Some(error),
            Self::Text(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct Pending {
    generation: u64,
    prepared: PdfPreparedTextEdit,
    _output_capacity: Reservation,
}

#[derive(Debug)]
struct State {
    revision: PdfRevision,
    pending: Option<Pending>,
    _source_bytes: Reservation,
}

/// A non-cloneable owner of a retained graph and at most one prepared raw edit.
#[derive(Debug)]
pub struct PdfSession {
    owner: Arc<()>,
    state: Option<State>,
    context: OperationContext,
    generation: u64,
    max_output_bytes: usize,
}

impl PdfSession {
    pub fn open(
        input: Vec<u8>,
        config: EngineConfig,
        context: OperationContext,
    ) -> Result<Self, PdfSessionError> {
        context.checkpoint().map_err(PdfSessionError::Context)?;
        if input.len() > config.limits.max_input_bytes {
            return Err(PdfSessionError::Pdf(PdfError::limit(
                "input exceeds max_input_bytes",
            )));
        }
        let source_bytes = context
            .reserve(input.len() as u64)
            .map_err(PdfSessionError::Context)?;
        context
            .charge_work(input.len() as u64)
            .map_err(PdfSessionError::Context)?;
        let source = Source::from_owned(input, config.limits.max_input_bytes)
            .map_err(|error| PdfSessionError::Pdf(PdfError::limit(error.to_string())))?;
        let max_output_bytes = config.limits.max_output_bytes;
        let document = PdfEngine::new(config)
            .open_source_with_cancellation(
                source,
                OpenOptions::default(),
                context.cancellation().clone(),
            )
            .map_err(PdfSessionError::Pdf)?;
        context.checkpoint().map_err(PdfSessionError::Context)?;
        Ok(Self {
            owner: Arc::new(()),
            state: Some(State {
                revision: PdfRevision::new(document),
                pending: None,
                _source_bytes: source_bytes,
            }),
            context,
            generation: 0,
            max_output_bytes,
        })
    }

    pub fn context(&self) -> &OperationContext {
        &self.context
    }

    /// Replace the sticky token without resetting either cumulative budget.
    pub fn renew_cancellation(&mut self) -> Result<(), PdfSessionError> {
        self.state()?;
        self.context =
            OperationContext::new(self.context.budget().clone(), CancellationToken::new());
        Ok(())
    }

    /// Read immutable identity metadata even when the operation token is cancelled.
    pub fn snapshot(&self) -> Result<SnapshotId, PdfSessionError> {
        Ok(self.state()?.revision.snapshot())
    }

    /// Return the current revision's page spans, using this operation token.
    pub fn extract_page_text_spans(
        &self,
        page_index: usize,
    ) -> Result<TextExtraction, PdfSessionError> {
        self.state()?
            .revision
            .extract_page_text_spans(page_index, &self.context)
            .map_err(PdfSessionError::Pdf)
    }

    /// Prepare exactly the P2b ASCII whole-token operation, with no font fallback.
    pub fn prepare_text_edit(
        &mut self,
        request: SurgicalTextEditRequest,
        mode: PdfWriterMode,
        expected_contract: u32,
    ) -> Result<PdfSessionPlanHandle, PdfSessionError> {
        let state = self.state()?;
        if state.pending.is_some() {
            return Err(PdfSessionError::PlanPending);
        }
        self.context
            .checkpoint()
            .map_err(PdfSessionError::Context)?;
        let generation = self.generation.checked_add(1).ok_or_else(|| {
            PdfSessionError::Pdf(PdfError::limit("session plan generation exhausted"))
        })?;
        let output_capacity = self
            .context
            .reserve(self.max_output_bytes as u64)
            .map_err(PdfSessionError::Context)?;
        let prepared = state
            .revision
            .prepare_text_edit(request, mode, expected_contract, &self.context)
            .map_err(PdfSessionError::Text)?;
        self.context
            .checkpoint()
            .map_err(PdfSessionError::Context)?;
        let handle = PdfSessionPlanHandle {
            owner: self.owner.clone(),
            snapshot: state.revision.snapshot(),
            generation,
        };
        self.state.as_mut().expect("checked open session").pending = Some(Pending {
            generation,
            prepared,
            _output_capacity: output_capacity,
        });
        self.generation = generation;
        Ok(handle)
    }

    /// Admission failure retains the plan; any started acceptance consumes it.
    pub fn accept_text_edit(
        &mut self,
        handle: &PdfSessionPlanHandle,
    ) -> Result<PdfTextEditReceipt, PdfSessionError> {
        self.validate_handle(handle)?;
        self.context
            .charge_work(1)
            .map_err(PdfSessionError::Context)?;
        let state = self.state.as_mut().expect("checked open session");
        let pending = state.pending.take().expect("validated pending plan");
        let request = pending.prepared.receipt().request().clone();
        let contract = pending.prepared.receipt().contract_version();
        let required = pending.prepared.receipt().required_checks().clone();
        state
            .revision
            .accept_text_edit(
                pending.prepared,
                &request,
                contract,
                &required,
                &self.context,
            )
            .map_err(PdfSessionError::Text)
    }

    pub fn release_plan(&mut self, handle: &PdfSessionPlanHandle) -> Result<(), PdfSessionError> {
        self.validate_handle(handle)?;
        self.context
            .checkpoint()
            .map_err(PdfSessionError::Context)?;
        self.state.as_mut().expect("checked open session").pending = None;
        Ok(())
    }

    /// Capacity is reserved only during this call; returned Source owns its bytes
    /// independently and is outside the session's managed live-byte counter.
    pub fn export(&self, mode: PdfWriterMode) -> Result<PdfRevisionExport, PdfSessionError> {
        let state = self.state()?;
        let _output_capacity = self
            .context
            .reserve(self.max_output_bytes as u64)
            .map_err(PdfSessionError::Context)?;
        state
            .revision
            .export(mode, &self.context)
            .map_err(PdfSessionError::Pdf)
    }

    /// Idempotent and available even after cancellation or budget exhaustion.
    pub fn close(&mut self) {
        self.state = None;
    }

    fn state(&self) -> Result<&State, PdfSessionError> {
        self.state.as_ref().ok_or(PdfSessionError::Closed)
    }

    fn validate_handle(&self, handle: &PdfSessionPlanHandle) -> Result<(), PdfSessionError> {
        let state = self.state()?;
        if !Arc::ptr_eq(&self.owner, &handle.owner) {
            return Err(PdfSessionError::ForeignPlan);
        }
        let current = state.revision.snapshot();
        if handle.snapshot != current {
            return Err(PdfSessionError::StalePlan {
                planned: handle.snapshot,
                current,
            });
        }
        if state.pending.as_ref().map(|pending| pending.generation) != Some(handle.generation) {
            return Err(PdfSessionError::PlanReleased);
        }
        Ok(())
    }
}
