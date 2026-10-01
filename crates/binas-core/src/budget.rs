//! Managed resource accounting, independent of any format or allocator.
//!
//! A reservation accounts for memory held by a caller; it does not allocate or
//! measure process memory. Work is cumulative and is never refunded.

use std::{
    error::Error,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetLimits {
    pub max_live_bytes: u64,
    pub max_work_units: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BudgetSnapshot {
    pub live_bytes: u64,
    pub work_units: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetResource {
    LiveBytes,
    WorkUnits,
}

impl fmt::Display for BudgetResource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::LiveBytes => "live_bytes",
            Self::WorkUnits => "work_units",
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BudgetError {
    LimitExceeded {
        resource: BudgetResource,
        used: u64,
        requested: u64,
        limit: u64,
    },
    CounterOverflow {
        resource: BudgetResource,
        used: u64,
        requested: u64,
    },
}

impl fmt::Display for BudgetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded {
                resource,
                used,
                requested,
                limit,
            } => {
                write!(
                    f,
                    "{resource} limit exceeded: {used} used, {requested} requested, {limit} limit"
                )
            }
            Self::CounterOverflow {
                resource,
                used,
                requested,
            } => {
                write!(
                    f,
                    "{resource} counter overflow: {used} used, {requested} requested"
                )
            }
        }
    }
}

impl Error for BudgetError {}

#[derive(Debug)]
struct BudgetState {
    limits: BudgetLimits,
    counters: Mutex<BudgetSnapshot>,
}

/// Clones share counters and immutable limits, including across threads.
#[derive(Clone, Debug)]
pub struct ResourceBudget {
    state: Arc<BudgetState>,
}

impl ResourceBudget {
    pub fn new(limits: BudgetLimits) -> Self {
        Self {
            state: Arc::new(BudgetState {
                limits,
                counters: Mutex::new(BudgetSnapshot::default()),
            }),
        }
    }

    pub fn limits(&self) -> BudgetLimits {
        self.state.limits
    }

    pub fn snapshot(&self) -> BudgetSnapshot {
        *self.counters()
    }

    /// Reserve before allocating. A failed request leaves both counters unchanged.
    pub fn reserve(&self, bytes: u64) -> Result<Reservation, BudgetError> {
        let mut counters = self.counters();
        counters.live_bytes = checked_charge(
            BudgetResource::LiveBytes,
            counters.live_bytes,
            bytes,
            self.state.limits.max_live_bytes,
        )?;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }

    /// Charge before doing work. Charges remain spent after failure or eviction.
    pub fn charge_work(&self, units: u64) -> Result<(), BudgetError> {
        let mut counters = self.counters();
        counters.work_units = checked_charge(
            BudgetResource::WorkUnits,
            counters.work_units,
            units,
            self.state.limits.max_work_units,
        )?;
        Ok(())
    }

    fn counters(&self) -> MutexGuard<'_, BudgetSnapshot> {
        // No user code runs while locked; recovery also keeps Drop non-panicking
        // if unwinding ever poisons a lock after a future internal change.
        self.state
            .counters
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

fn checked_charge(
    resource: BudgetResource,
    used: u64,
    requested: u64,
    limit: u64,
) -> Result<u64, BudgetError> {
    let total = used
        .checked_add(requested)
        .ok_or(BudgetError::CounterOverflow {
            resource,
            used,
            requested,
        })?;
    if total > limit {
        return Err(BudgetError::LimitExceeded {
            resource,
            used,
            requested,
            limit,
        });
    }
    Ok(total)
}

/// Holds one live-memory charge until dropped. Move it with the accounted data.
/// It is deliberately not Clone: shared data should share the same reservation.
#[must_use = "dropping the reservation immediately releases its live-memory charge"]
#[derive(Debug)]
pub struct Reservation {
    budget: ResourceBudget,
    bytes: u64,
}

impl Reservation {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.counters().live_bytes -= self.bytes;
    }
}
