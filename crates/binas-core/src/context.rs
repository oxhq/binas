//! Cooperative operation cancellation and shared resource accounting.

use std::{
    error::Error,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::budget::{BudgetError, Reservation, ResourceBudget};

/// Cancellation is sticky and shared by clones. A new operation may use a new
/// token while retaining the same cumulative resource budget.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContextError {
    Cancelled,
    Budget(BudgetError),
}

impl fmt::Display for ContextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("operation cancelled"),
            Self::Budget(error) => error.fmt(f),
        }
    }
}

impl Error for ContextError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Cancelled => None,
            Self::Budget(error) => Some(error),
        }
    }
}

impl From<BudgetError> for ContextError {
    fn from(error: BudgetError) -> Self {
        Self::Budget(error)
    }
}

/// Call checkpoints inside bounded work loops. This does not interrupt a running
/// codec, yield a WASM event loop, or implement transaction rollback by itself.
#[derive(Clone, Debug)]
pub struct OperationContext {
    budget: ResourceBudget,
    cancellation: CancellationToken,
}

impl OperationContext {
    pub fn new(budget: ResourceBudget, cancellation: CancellationToken) -> Self {
        Self {
            budget,
            cancellation,
        }
    }

    pub fn budget(&self) -> &ResourceBudget {
        &self.budget
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    pub fn checkpoint(&self) -> Result<(), ContextError> {
        if self.cancellation.is_cancelled() {
            Err(ContextError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// Cancellation is checked before charging; a later cancellation does not
    /// refund work already charged. Callers must continue polling checkpoints.
    pub fn charge_work(&self, units: u64) -> Result<(), ContextError> {
        self.checkpoint()?;
        self.budget.charge_work(units)?;
        Ok(())
    }

    pub fn reserve(&self, bytes: u64) -> Result<Reservation, ContextError> {
        self.checkpoint()?;
        Ok(self.budget.reserve(bytes)?)
    }
}
