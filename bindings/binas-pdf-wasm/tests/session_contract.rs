//! Public consumer lifecycle and P3 byte parity, native and real browser WASM.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfEngine, PdfPageTextEditRequest, PdfRevision,
    PdfRevisionRequest, PdfSession, PdfWriterMode, SurgicalTextEditRequest,
};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::*;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test_configure!(run_in_browser);

const RAW: &[u8] = include_bytes!(
    "../../../crates/binas-pdf/tests/fixtures/retained-ascii/retained-ascii-plain.pdf"
);
const RAW_FLATE: &[u8] = include_bytes!(
    "../../../crates/binas-pdf/tests/fixtures/retained-ascii/retained-ascii-flate.pdf"
);
const FONTED: &[u8] =
    include_bytes!("../../../crates/binas-pdf/tests/fixtures/fonted-text/fonted-text-plain.pdf");
const FONTED_FLATE: &[u8] =
    include_bytes!("../../../crates/binas-pdf/tests/fixtures/fonted-text/fonted-text-flate.pdf");

fn config() -> EngineConfig {
    EngineConfig {
        limits: Limits {
            max_output_bytes: 65536,
            ..Limits::default()
        },
    }
}
fn context(live: u64, work: u64) -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: live,
            max_work_units: work,
        }),
        CancellationToken::new(),
    )
}
fn ample() -> OperationContext {
    context(1_000_000, 100_000_000)
}
fn request(replacement: &str) -> SurgicalTextEditRequest {
    SurgicalTextEditRequest {
        old_text: "ALPHA".into(),
        replacement: replacement.into(),
        match_index: 0,
    }
}
fn text(session: &PdfSession) -> Vec<String> {
    let result = session.extract_page_text_spans(0).unwrap();
    assert!(result.warnings.is_empty());
    result.spans.into_iter().map(|span| span.text).collect()
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn raw_sessions_match_four_independently_checked_p3a_outputs() {
    let expected: [[&[u8]; 2]; 2] = [
        [
            include_bytes!("fixtures/sessions/p3a-plain-incremental.pdf"),
            include_bytes!("fixtures/sessions/p3a-plain-compact.pdf"),
        ],
        [
            include_bytes!("fixtures/sessions/p3a-flate-incremental.pdf"),
            include_bytes!("fixtures/sessions/p3a-flate-compact.pdf"),
        ],
    ];
    for (shape, input) in [RAW, RAW_FLATE].iter().enumerate() {
        for (writer, mode) in [PdfWriterMode::Incremental, PdfWriterMode::Compact]
            .iter()
            .enumerate()
        {
            let ctx = ample();
            let mut session = PdfSession::open(input.to_vec(), config(), ctx.clone()).unwrap();
            let before = session.snapshot().unwrap();
            let original = text(&session);
            let plan = session
                .prepare_text_edit(request("OMEGA"), *mode, 1)
                .unwrap();
            assert_eq!(session.snapshot().unwrap(), before);
            assert_eq!(text(&session), original);
            let receipt = session.accept_text_edit(&plan).unwrap();
            assert_eq!(receipt.before(), before);
            assert_eq!(receipt.after().source(), before.source());
            assert_eq!(receipt.after().revision().value(), 1);
            assert_eq!(text(&session)[0], "OMEGA");
            assert_eq!(
                session.accept_text_edit(&plan).unwrap_err().code(),
                "stale_plan"
            );
            for _ in 0..2 {
                let output = session.export(*mode).unwrap();
                assert_eq!(output.snapshot, receipt.after());
                assert_eq!(output.output.as_bytes(), expected[shape][writer]);
            }
            assert_eq!(ctx.budget().snapshot().live_bytes, input.len() as u64);
            session.close();
            session.close();
            assert_eq!(ctx.budget().snapshot().live_bytes, 0);
            assert!(ctx.budget().snapshot().work_units > 0);
            assert_eq!(session.snapshot().unwrap_err().code(), "session_closed");
            assert_eq!(
                session.accept_text_edit(&plan).unwrap_err().code(),
                "session_closed"
            );
            assert_eq!(
                session.extract_page_text_spans(0).unwrap_err().code(),
                "session_closed"
            );
            assert_eq!(session.export(*mode).unwrap_err().code(), "session_closed");
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn same_source_foreign_released_handles_and_drop_preserve_ownership() {
    let ctx = ample();
    let mut first = PdfSession::open(RAW.to_vec(), config(), ctx.clone()).unwrap();
    let mut second = PdfSession::open(RAW.to_vec(), config(), ctx.clone()).unwrap();
    assert_eq!(first.snapshot().unwrap(), second.snapshot().unwrap());
    let plan = first
        .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
        .unwrap();
    let token_clone = plan.clone();
    assert_eq!(
        second.accept_text_edit(&plan).unwrap_err().code(),
        "foreign_plan"
    );
    assert_eq!(
        first
            .prepare_text_edit(request("OTHER"), PdfWriterMode::Compact, 1)
            .unwrap_err()
            .code(),
        "plan_pending"
    );
    first.release_plan(&plan).unwrap();
    assert_eq!(
        first.accept_text_edit(&token_clone).unwrap_err().code(),
        "plan_released"
    );
    let next = first
        .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
        .unwrap();
    assert_eq!(
        first.accept_text_edit(&plan).unwrap_err().code(),
        "plan_released"
    );
    first.close();
    assert_eq!(ctx.budget().snapshot().live_bytes, RAW.len() as u64);
    drop(first);
    drop(next);
    drop(plan);
    drop(token_clone);
    drop(second);
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
    let mut third = PdfSession::open(RAW.to_vec(), config(), ctx.clone()).unwrap();
    let _handle = third
        .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
        .unwrap();
    drop(third);
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn sticky_cancellation_has_atomic_retry_and_close_releases_pending() {
    let ctx = ample();
    let mut session = PdfSession::open(RAW.to_vec(), config(), ctx.clone()).unwrap();
    let before = session.snapshot().unwrap();
    let plan = session
        .prepare_text_edit(request("OMEGA"), PdfWriterMode::Incremental, 1)
        .unwrap();
    let counters = ctx.budget().snapshot();
    ctx.cancellation().cancel();
    assert_eq!(
        session.extract_page_text_spans(0).unwrap_err().code(),
        "cancelled"
    );
    assert_eq!(
        session.accept_text_edit(&plan).unwrap_err().code(),
        "cancelled"
    );
    assert_eq!(
        session.export(PdfWriterMode::Compact).unwrap_err().code(),
        "cancelled"
    );
    assert_eq!(ctx.budget().snapshot(), counters);
    assert_eq!(session.snapshot().unwrap(), before);
    session.renew_cancellation().unwrap();
    assert!(ctx.cancellation().is_cancelled()); // old clones remain sticky
    session.accept_text_edit(&plan).unwrap();
    assert_eq!(text(&session)[0], "OMEGA");
    session.context().cancellation().cancel();
    session.close();
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
    assert_eq!(
        session.renew_cancellation().unwrap_err().code(),
        "session_closed"
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn budget_failures_and_invalid_requests_publish_nothing_and_release_reservations() {
    let ctx = ample();
    let mut too_small = config();
    too_small.limits.max_input_bytes = RAW.len() - 1;
    assert_eq!(
        PdfSession::open(RAW.to_vec(), too_small, ctx.clone())
            .unwrap_err()
            .code(),
        "resource_limit"
    );
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
    assert_eq!(ctx.budget().snapshot().work_units, 0);
    ctx.cancellation().cancel();
    assert_eq!(
        PdfSession::open(RAW.to_vec(), config(), ctx.clone())
            .unwrap_err()
            .code(),
        "cancelled"
    );
    assert_eq!(ctx.budget().snapshot().work_units, 0);
    let ctx = ample();
    let mut too_small = config();
    too_small.limits.max_output_bytes = 200;
    let mut limited_output = PdfSession::open(RAW.to_vec(), too_small, ctx.clone()).unwrap();
    let before = limited_output.snapshot().unwrap();
    assert_eq!(
        limited_output
            .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
            .unwrap_err()
            .code(),
        "resource_limit"
    );
    assert_eq!(
        limited_output
            .export(PdfWriterMode::Compact)
            .unwrap_err()
            .code(),
        "resource_limit"
    );
    assert_eq!(limited_output.snapshot().unwrap(), before);
    assert_eq!(ctx.budget().snapshot().live_bytes, RAW.len() as u64);
    assert!(ctx.budget().snapshot().work_units > RAW.len() as u64);
    limited_output.close();
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
    let ctx = context(RAW.len() as u64, 100_000_000);
    let mut session = PdfSession::open(RAW.to_vec(), config(), ctx.clone()).unwrap();
    let before = session.snapshot().unwrap();
    let counters = ctx.budget().snapshot();
    assert_eq!(
        session
            .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
            .unwrap_err()
            .code(),
        "resource_limit"
    );
    assert_eq!(
        session.export(PdfWriterMode::Compact).unwrap_err().code(),
        "resource_limit"
    );
    assert_eq!(session.snapshot().unwrap(), before);
    assert_eq!(ctx.budget().snapshot(), counters);
    drop(session);
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
    let ctx = context(1_000_000, RAW.len() as u64 - 1);
    assert_eq!(
        PdfSession::open(RAW.to_vec(), config(), ctx.clone())
            .unwrap_err()
            .code(),
        "resource_limit"
    );
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
    let ctx = ample();
    let mut session = PdfSession::open(RAW.to_vec(), config(), ctx.clone()).unwrap();
    for (request, version) in [
        (request("OMEGA"), 99),
        (request(""), 1),
        (
            SurgicalTextEditRequest {
                old_text: "MISSING".into(),
                ..request("OMEGA")
            },
            1,
        ),
    ] {
        let before = session.snapshot().unwrap();
        assert!(
            session
                .prepare_text_edit(request, PdfWriterMode::Compact, version)
                .is_err()
        );
        assert_eq!(session.snapshot().unwrap(), before);
        assert_eq!(ctx.budget().snapshot().live_bytes, RAW.len() as u64);
    }
    let plan = session
        .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
        .unwrap();
    session.release_plan(&plan).unwrap();
    assert_eq!(text(&session)[0], "ALPHA");
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn successive_commits_read_current_revision_and_changed_input_is_foreign() {
    let mut session = PdfSession::open(RAW.to_vec(), config(), ample()).unwrap();
    let first = session
        .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
        .unwrap();
    session.accept_text_edit(&first).unwrap();
    let second = session
        .prepare_text_edit(
            SurgicalTextEditRequest {
                old_text: "OMEGA".into(),
                replacement: "SIGMA".into(),
                match_index: 0,
            },
            PdfWriterMode::Incremental,
            1,
        )
        .unwrap();
    session.accept_text_edit(&second).unwrap();
    assert_eq!(session.snapshot().unwrap().revision().value(), 2);
    assert_eq!(text(&session)[0], "SIGMA");
    let mut changed = RAW.to_vec();
    let start = changed
        .windows(5)
        .position(|window| window == b"ALPHA")
        .unwrap();
    changed[start..start + 5].copy_from_slice(b"DELTA");
    let mut other = PdfSession::open(changed, config(), ample()).unwrap();
    assert_ne!(
        session.snapshot().unwrap().source(),
        other.snapshot().unwrap().source()
    );
    assert_eq!(
        other.accept_text_edit(&second).unwrap_err().code(),
        "foreign_plan"
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn started_acceptance_budget_failure_consumes_plan_and_retains_revision() {
    let ctx = ample();
    let mut session = PdfSession::open(RAW.to_vec(), config(), ctx.clone()).unwrap();
    let before = session.snapshot().unwrap();
    let plan = session
        .prepare_text_edit(request("OMEGA"), PdfWriterMode::Compact, 1)
        .unwrap();
    let used = ctx.budget().snapshot().work_units;
    let ceiling = ctx.budget().limits().max_work_units;
    ctx.budget().charge_work(ceiling - used - 1).unwrap();
    assert_eq!(
        session.accept_text_edit(&plan).unwrap_err().code(),
        "resource_limit"
    );
    assert_eq!(session.snapshot().unwrap(), before);
    assert_eq!(ctx.budget().snapshot().live_bytes, RAW.len() as u64);
    assert_eq!(ctx.budget().snapshot().work_units, ceiling);
    assert_eq!(
        session.accept_text_edit(&plan).unwrap_err().code(),
        "plan_released"
    );
    session.close();
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn fonted_shared_rust_api_matches_p3b_without_expanding_raw_session_capability() {
    let expected: [[&[u8]; 2]; 2] = [
        [
            include_bytes!("fixtures/sessions/p3b-plain-incremental.pdf"),
            include_bytes!("fixtures/sessions/p3b-plain-compact.pdf"),
        ],
        [
            include_bytes!("fixtures/sessions/p3b-flate-incremental.pdf"),
            include_bytes!("fixtures/sessions/p3b-flate-compact.pdf"),
        ],
    ];
    for (shape, input) in [FONTED, FONTED_FLATE].iter().enumerate() {
        let ctx = ample();
        let mut session = PdfSession::open(input.to_vec(), config(), ctx.clone()).unwrap();
        let before = session.snapshot().unwrap();
        assert_eq!(
            session
                .prepare_text_edit(request("OMEGA-LONG"), PdfWriterMode::Compact, 1)
                .unwrap_err()
                .code(),
            "unsupported_feature"
        );
        assert_eq!(session.snapshot().unwrap(), before);
        let mut live = PdfRevision::new(
            PdfEngine::default()
                .open_owned(input.to_vec(), OpenOptions::default())
                .unwrap(),
        );
        let original = live.extract_page_text_spans(0, &ctx).unwrap();
        assert!(original.warnings.is_empty());
        let plan = live
            .plan(
                PdfRevisionRequest {
                    page_text: vec![PdfPageTextEditRequest {
                        page_index: 0,
                        edit: request("OMEGA-LONG"),
                        shared: Default::default(),
                    }],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        live.commit(&plan, &ctx).unwrap();
        assert_eq!(
            live.extract_page_text_spans(0, &ctx).unwrap().spans[0].text,
            "OMEGA-LONG"
        );
        for (writer, mode) in [PdfWriterMode::Incremental, PdfWriterMode::Compact]
            .iter()
            .enumerate()
        {
            assert_eq!(
                live.export(*mode, &ctx).unwrap().output.as_bytes(),
                expected[shape][writer]
            );
        }
    }
}
