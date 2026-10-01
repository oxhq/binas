use binas_core::{
    budget::{BudgetLimits, ResourceBudget},
    context::{CancellationToken, ContextError, OperationContext},
    identity::{RevisionId, SnapshotId, SourceId},
    transaction::{PlanEnvelope, RevisionState, StagedChange, TransactionError, execute_plan},
    verification::{
        CheckCode, CheckEvidence, CheckStatus, EvidenceError, EvidenceReport, RequiredChecks,
    },
};

fn code(name: &str) -> CheckCode {
    CheckCode::new("test", name).unwrap()
}
fn required() -> RequiredChecks {
    RequiredChecks::new(vec![code("intent"), code("preservation")]).unwrap()
}
fn report(status: CheckStatus) -> EvidenceReport<()> {
    EvidenceReport::new(vec![
        CheckEvidence {
            code: code("intent"),
            status: CheckStatus::Passed,
            detail: (),
        },
        CheckEvidence {
            code: code("preservation"),
            status,
            detail: (),
        },
    ])
}
fn snapshot(source: u8, revision: u64) -> SnapshotId {
    SnapshotId::new(
        SourceId::from_sha256([source; 32]),
        RevisionId::new(revision),
    )
}
fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 100,
            max_work_units: 100,
        }),
        CancellationToken::new(),
    )
}
fn stage<T: AsRef<[u8]>>(
    current: &T,
    intent: &u8,
    _: &OperationContext,
) -> Result<StagedChange<Vec<u8>, ()>, &'static str> {
    let mut candidate = current.as_ref().to_vec();
    candidate[0] = *intent;
    Ok(StagedChange {
        candidate,
        changes: (),
    })
}

#[test]
fn codes_and_obligations_cannot_be_empty_ambiguous_or_duplicated() {
    for (namespace, code) in [
        ("", "a"),
        ("pdf", ""),
        ("pdf:core", "a"),
        ("pdf", "a b"),
        ("1pdf", "a"),
        ("pdf", "é"),
    ] {
        assert!(CheckCode::new(namespace, code).is_err());
    }
    assert!(CheckCode::new("pdf", "a".repeat(129)).is_err());
    assert_eq!(code("intent").to_string(), "test:intent");
    assert_eq!(
        RequiredChecks::new(vec![]).unwrap_err(),
        EvidenceError::EmptyObligations
    );
    assert_eq!(
        RequiredChecks::new(vec![code("intent"), code("intent")]).unwrap_err(),
        EvidenceError::DuplicateObligation(code("intent"))
    );
}

#[test]
fn only_complete_unique_expected_passing_evidence_satisfies_obligations() {
    let required = required();
    report(CheckStatus::Passed).validate(&required).unwrap();
    for status in [
        CheckStatus::Failed,
        CheckStatus::NotChecked,
        CheckStatus::Unsupported,
    ] {
        assert_eq!(
            report(status).validate(&required),
            Err(EvidenceError::NonPassingCheck {
                code: code("preservation"),
                status
            })
        );
    }
    let empty: EvidenceReport<()> = EvidenceReport::new(vec![]);
    assert!(matches!(
        empty.validate(&required),
        Err(EvidenceError::MissingCheck(_))
    ));
    let mut checks = report(CheckStatus::Passed).checks().to_vec();
    checks.pop();
    assert_eq!(
        EvidenceReport::new(checks).validate(&required),
        Err(EvidenceError::MissingCheck(code("preservation")))
    );
    let mut checks = report(CheckStatus::Passed).checks().to_vec();
    checks.push(checks[0].clone());
    assert_eq!(
        EvidenceReport::new(checks).validate(&required),
        Err(EvidenceError::DuplicateCheck(code("intent")))
    );
    let mut checks = report(CheckStatus::Passed).checks().to_vec();
    checks.push(CheckEvidence {
        code: code("unregistered"),
        status: CheckStatus::Passed,
        detail: (),
    });
    assert_eq!(
        EvidenceReport::new(checks).validate(&required),
        Err(EvidenceError::UnknownCheck(code("unregistered")))
    );
}

#[test]
fn successful_commit_advances_once_and_replay_is_stale() {
    let before = snapshot(1, 0);
    let mut state = RevisionState::new(before, vec![1, 2]);
    let plan = PlanEnvelope::new(before, 7, required());
    let receipt = execute_plan(
        &mut state,
        &plan,
        &context(),
        stage,
        |original, candidate, intent, _, _| {
            assert_eq!(original, &[1, 2]);
            assert_eq!(candidate, &[*intent, 2]);
            Ok(report(CheckStatus::Passed))
        },
    )
    .unwrap();
    assert_eq!(state.value(), &[7, 2]);
    assert_eq!(state.snapshot(), snapshot(1, 1));
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after, state.snapshot());
    receipt.evidence.validate(plan.required()).unwrap();
    let result = execute_plan(&mut state, &plan, &context(), stage, |_, _, _, _, _| {
        Ok(report(CheckStatus::Passed))
    });
    assert!(matches!(result, Err(TransactionError::StalePlan { .. })));
    assert_eq!(state.snapshot(), snapshot(1, 1));
}

#[test]
fn same_sized_different_source_and_revision_overflow_never_stage() {
    for (current, planned, overflow) in [
        (snapshot(2, 0), snapshot(1, 0), false),
        (snapshot(1, 1), snapshot(1, 0), false),
        (snapshot(1, u64::MAX), snapshot(1, u64::MAX), true),
    ] {
        let mut state = RevisionState::new(current, vec![2, 3]);
        let plan = PlanEnvelope::new(planned, 7, required());
        let result = execute_plan(
            &mut state,
            &plan,
            &context(),
            |_, _, _| -> Result<StagedChange<Vec<u8>, ()>, &'static str> {
                panic!("must not stage")
            },
            |_, _, _, _, _| Ok(report(CheckStatus::Passed)),
        );
        if overflow {
            assert!(matches!(result, Err(TransactionError::RevisionOverflow)));
        } else {
            assert!(matches!(result, Err(TransactionError::StalePlan { .. })));
        }
        assert_eq!(state.value(), &[2, 3]);
        assert_eq!(state.snapshot(), current);
    }
}

#[test]
fn stage_check_and_evidence_failure_never_publish_candidates() {
    let before = snapshot(1, 0);
    let plan = PlanEnvelope::new(before, 7, required());
    let mut state = RevisionState::new(before, vec![1, 2]);
    let result = execute_plan(
        &mut state,
        &plan,
        &context(),
        |_, _, _| -> Result<StagedChange<Vec<u8>, ()>, &'static str> { Err("stage rejected") },
        |_, _, _, _, _| Ok(report(CheckStatus::Passed)),
    );
    assert!(matches!(
        result,
        Err(TransactionError::Stage("stage rejected"))
    ));
    let result = execute_plan(
        &mut state,
        &plan,
        &context(),
        stage,
        |_, _, _, _, _| -> Result<EvidenceReport<()>, &'static str> { Err("check rejected") },
    );
    assert!(matches!(
        result,
        Err(TransactionError::Check("check rejected"))
    ));
    for evidence in [
        report(CheckStatus::Failed),
        report(CheckStatus::Unsupported),
        EvidenceReport::new(vec![]),
    ] {
        let result = execute_plan(&mut state, &plan, &context(), stage, |_, _, _, _, _| {
            Ok(evidence)
        });
        assert!(matches!(result, Err(TransactionError::Evidence(_))));
        assert_eq!(state.value(), &[1, 2]);
        assert_eq!(state.snapshot(), before);
    }
}

#[test]
fn cancellation_before_during_stage_and_during_check_retains_committed_state() {
    for cancellation_phase in 0..3 {
        let context = context();
        let before = snapshot(1, 0);
        let plan = PlanEnvelope::new(before, 7, required());
        let mut state = RevisionState::new(before, vec![1, 2]);
        if cancellation_phase == 0 {
            context.cancellation().cancel();
        }
        let result = execute_plan(
            &mut state,
            &plan,
            &context,
            |current, intent, context| -> Result<StagedChange<Vec<u8>, ()>, &'static str> {
                assert_ne!(cancellation_phase, 0);
                let candidate = stage(current, intent, context)?;
                context.charge_work(3).unwrap();
                if cancellation_phase == 1 {
                    context.cancellation().cancel();
                }
                Ok(candidate)
            },
            |_, _, _, _, context| {
                assert_eq!(cancellation_phase, 2);
                context.cancellation().cancel();
                Ok(report(CheckStatus::Passed))
            },
        );
        assert!(matches!(
            result,
            Err(TransactionError::Context(ContextError::Cancelled))
        ));
        assert_eq!(state.value(), &[1, 2]);
        assert_eq!(state.snapshot(), before);
        assert_eq!(
            context.budget().snapshot().work_units,
            if cancellation_phase == 0 { 0 } else { 3 }
        );
    }
}
