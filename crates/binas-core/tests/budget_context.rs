use binas_core::{
    budget::{BudgetError, BudgetLimits, BudgetResource, BudgetSnapshot, ResourceBudget},
    context::{CancellationToken, ContextError, OperationContext},
};

fn budget(live: u64, work: u64) -> ResourceBudget {
    ResourceBudget::new(BudgetLimits {
        max_live_bytes: live,
        max_work_units: work,
    })
}

#[test]
fn reservations_release_live_memory_without_refunding_work() {
    let budget = budget(100, 20);
    let shared = budget.clone();
    let outer = budget.reserve(40).unwrap();
    assert_eq!(outer.bytes(), 40);
    {
        let _inner = shared.reserve(60).unwrap();
        shared.charge_work(20).unwrap();
        assert_eq!(
            budget.snapshot(),
            BudgetSnapshot {
                live_bytes: 100,
                work_units: 20
            }
        );
        assert_eq!(
            budget.reserve(1).unwrap_err(),
            BudgetError::LimitExceeded {
                resource: BudgetResource::LiveBytes,
                used: 100,
                requested: 1,
                limit: 100,
            }
        );
    }
    assert_eq!(
        shared.snapshot(),
        BudgetSnapshot {
            live_bytes: 40,
            work_units: 20
        }
    );
    drop(outer);
    let _replacement = shared.reserve(100).unwrap();
    assert_eq!(
        shared.charge_work(1).unwrap_err(),
        BudgetError::LimitExceeded {
            resource: BudgetResource::WorkUnits,
            used: 20,
            requested: 1,
            limit: 20,
        }
    );
    assert_eq!(
        budget.limits(),
        BudgetLimits {
            max_live_bytes: 100,
            max_work_units: 20
        }
    );
}

#[test]
fn failures_and_overflow_leave_all_counters_unchanged() {
    let budget = budget(u64::MAX, u64::MAX);
    let reservation = budget.reserve(u64::MAX).unwrap();
    budget.charge_work(u64::MAX).unwrap();
    let before = budget.snapshot();
    assert_eq!(
        budget.reserve(1).unwrap_err(),
        BudgetError::CounterOverflow {
            resource: BudgetResource::LiveBytes,
            used: u64::MAX,
            requested: 1,
        }
    );
    assert_eq!(
        budget.charge_work(1).unwrap_err(),
        BudgetError::CounterOverflow {
            resource: BudgetResource::WorkUnits,
            used: u64::MAX,
            requested: 1,
        }
    );
    assert_eq!(budget.snapshot(), before);
    drop(reservation);
    assert_eq!(
        budget.snapshot(),
        BudgetSnapshot {
            live_bytes: 0,
            work_units: u64::MAX
        }
    );
}

#[test]
fn zero_limits_accept_zero_and_reject_positive_charges() {
    let budget = budget(0, 0);
    let _reservation = budget.reserve(0).unwrap();
    budget.charge_work(0).unwrap();
    assert!(matches!(
        budget.reserve(1),
        Err(BudgetError::LimitExceeded {
            resource: BudgetResource::LiveBytes,
            ..
        })
    ));
    assert!(matches!(
        budget.charge_work(1),
        Err(BudgetError::LimitExceeded {
            resource: BudgetResource::WorkUnits,
            ..
        })
    ));
    assert_eq!(budget.snapshot(), BudgetSnapshot::default());
}

#[test]
fn failed_operation_drops_reservations_but_retains_spent_work() {
    fn fail(context: &OperationContext) -> Result<(), ContextError> {
        let _temporary = context.reserve(50)?;
        context.charge_work(3)?;
        let _over_limit = context.reserve(51)?;
        Ok(())
    }
    let context = OperationContext::new(budget(100, 5), CancellationToken::new());
    assert!(matches!(
        fail(&context),
        Err(ContextError::Budget(BudgetError::LimitExceeded {
            resource: BudgetResource::LiveBytes,
            ..
        }))
    ));
    assert_eq!(
        context.budget().snapshot(),
        BudgetSnapshot {
            live_bytes: 0,
            work_units: 3
        }
    );
    context.charge_work(2).unwrap();
    assert!(context.charge_work(1).is_err());
}

#[test]
fn shared_cancellation_is_sticky_and_prevents_further_charges() {
    let token = CancellationToken::new();
    let context = OperationContext::new(budget(10, 20), token.clone());
    let child = context.clone();
    context.checkpoint().unwrap();
    let reservation = child.reserve(10).unwrap();
    child.charge_work(4).unwrap();
    token.cancel();
    token.cancel();
    assert!(context.cancellation().is_cancelled());
    assert_eq!(child.checkpoint(), Err(ContextError::Cancelled));
    assert_eq!(child.charge_work(5), Err(ContextError::Cancelled));
    assert_eq!(context.reserve(0).unwrap_err(), ContextError::Cancelled);
    drop(reservation);
    assert_eq!(
        context.budget().snapshot(),
        BudgetSnapshot {
            live_bytes: 0,
            work_units: 4
        }
    );
    let retry = OperationContext::new(context.budget().clone(), CancellationToken::new());
    retry.charge_work(16).unwrap();
    assert!(matches!(retry.charge_work(1), Err(ContextError::Budget(_))));
}

#[test]
fn reservations_can_outlive_the_initial_budget_handle() {
    let reservation = {
        let budget = budget(5, 0);
        budget.reserve(5).unwrap()
    };
    assert_eq!(reservation.bytes(), 5);
    drop(reservation);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn concurrent_clones_cannot_overspend_and_cancellation_crosses_threads() {
    use std::sync::{Arc, Barrier};
    let budget = budget(8, 8);
    let barrier = Arc::new(Barrier::new(16));
    let workers: Vec<_> = (0..16)
        .map(|_| {
            let budget = budget.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let reservation = budget.reserve(1).ok();
                let work_accepted = budget.charge_work(1).is_ok();
                barrier.wait(); // Keep accepted reservations alive until all attempts finish.
                (reservation.is_some(), work_accepted)
            })
        })
        .collect();
    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.0).count(), 8);
    assert_eq!(results.iter().filter(|result| result.1).count(), 8);
    assert_eq!(
        budget.snapshot(),
        BudgetSnapshot {
            live_bytes: 0,
            work_units: 8
        }
    );

    let token = CancellationToken::new();
    let context = OperationContext::new(budget, token.clone());
    std::thread::spawn(move || token.cancel()).join().unwrap();
    assert_eq!(context.checkpoint(), Err(ContextError::Cancelled));
}
