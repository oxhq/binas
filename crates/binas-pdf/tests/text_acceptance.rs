use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{OpenOptions, PdfEngine, PdfRevision, PdfWriterMode, SurgicalTextEditRequest};
fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1_000_000,
            max_work_units: 1_000_000,
        }),
        CancellationToken::new(),
    )
}
fn revision() -> PdfRevision {
    let mut input = include_bytes!("fixtures/preservation/metadata.pdf").to_vec();
    let start = input
        .windows(9)
        .position(|value| value == b"/F1 12 Tf")
        .unwrap();
    input[start..start + 9].fill(b' ');
    PdfRevision::new(
        PdfEngine::default()
            .open_owned(input, OpenOptions::default())
            .unwrap(),
    )
}
fn request() -> SurgicalTextEditRequest {
    SurgicalTextEditRequest {
        old_text: "A".into(),
        replacement: "B".into(),
        match_index: 0,
    }
}
// Defect caught: no supported preparation transaction that exports without publishing the live revision.
#[test]
fn prepare_supported_edit_without_publishing_live_revision() {
    let revision = revision();
    let before = revision.snapshot();
    let result = revision.prepare_text_edit(request(), PdfWriterMode::Incremental, 1, &context());
    assert!(
        result.is_ok(),
        "supported edit preparation refused: {result:?}"
    );
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.text().unwrap(), ["A"]);
}

use binas_core::verification::{CheckCode, CheckStatus, EvidenceError, RequiredChecks};
use binas_pdf::{
    CapabilityDecision, EngineConfig, PdfErrorCode, PdfTextEditError, PdfTextEditRefusal,
};
fn all_required() -> RequiredChecks {
    RequiredChecks::new(
        [
            ("pdf.revision", "expected_changes"),
            ("pdf.revision", "untouched_objects"),
            ("pdf.revision", "page_relationships"),
            ("pdf.revision", "requested_payloads"),
            ("pdf.revision", "page_dictionary_effects"),
            ("pdf.revision", "annotation_dictionary_effects"),
            ("pdf.revision", "stream_dictionary_effects"),
            ("pdf.revision", "authorized_objects"),
            ("pdf.revision", "catalog_dictionary_effects"),
            ("pdf.revision", "stream_owners"),
            ("pdf.revision", "requested_text"),
            ("pdf.writer", "reparse"),
            ("pdf.writer", "header_version"),
            ("pdf.writer", "objects_and_encoded_payloads"),
            ("pdf.writer", "trailer_values"),
            ("pdf.writer", "references_resolve"),
            ("pdf.writer", "page_relationships"),
            ("pdf.writer", "writer_mode"),
        ]
        .into_iter()
        .map(|(namespace, name)| CheckCode::new(namespace, name).unwrap())
        .collect(),
    )
    .unwrap()
}
fn prepared(revision: &PdfRevision) -> binas_pdf::PdfPreparedTextEdit {
    revision
        .prepare_text_edit(request(), PdfWriterMode::Incremental, 1, &context())
        .unwrap()
}
fn unchanged(revision: &PdfRevision, before: binas_core::SnapshotId, text: &str) {
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.text().unwrap(), [text]);
}

// Defect caught: incorrect advertised format/op/mode, version, obligations, or configured limit values.
#[test]
fn capability_is_conditional_exact_and_refuses_unknown_identifiers() {
    let revision = revision();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let capability = revision.text_edit_capability("pdf", "pdf.text.retained_ascii", mode);
        assert_eq!(capability.format(), "pdf");
        assert_eq!(capability.operation(), "pdf.text.retained_ascii");
        assert_eq!(capability.contract_version(), 1);
        assert_eq!(capability.engine_version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(capability.mode(), mode);
        assert_eq!(capability.decision(), CapabilityDecision::Conditional);
        assert_eq!(capability.refusal(), None);
        assert_eq!(capability.required_checks(), &all_required());
        assert_eq!(capability.limits().max_input_bytes, 64 * 1024 * 1024);
        assert_eq!(capability.limits().max_output_bytes, 128 * 1024 * 1024);
        assert_eq!(capability.limits().max_decode_live_bytes, 64 * 1024 * 1024);
    }
    for (format, operation, mode, reason) in [
        (
            "unknown",
            "pdf.text.retained_ascii",
            PdfWriterMode::Incremental,
            PdfTextEditRefusal::UnknownFormat,
        ),
        (
            "pdf",
            "unknown",
            PdfWriterMode::Incremental,
            PdfTextEditRefusal::UnknownOperation,
        ),
        (
            "pdf",
            "pdf.text.retained_ascii",
            PdfWriterMode::Surgical,
            PdfTextEditRefusal::UnsupportedWriterMode,
        ),
    ] {
        let capability = revision.text_edit_capability(format, operation, mode);
        assert_eq!(capability.decision(), CapabilityDecision::Refused);
        assert_eq!(capability.refusal(), Some(reason));
    }
}

// Defect caught: publishing wrong request/output, receipt omissions, or counting unchecked scopes as passed.
#[test]
fn accepted_receipts_bind_exact_actual_work_for_both_modes_and_sequential_edits() {
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let mut revision = revision();
        let before = revision.snapshot();
        let prepared = revision
            .prepare_text_edit(request(), mode, 1, &context())
            .unwrap();
        let output = prepared.output().clone();
        let observed = prepared.receipt();
        assert_eq!(observed.input_id(), before.source());
        assert_eq!(
            observed.input_len(),
            include_bytes!("fixtures/preservation/metadata.pdf").len()
        );
        assert_eq!(observed.before(), before);
        assert_eq!(observed.after().revision().value(), 1);
        assert_eq!(observed.after().source(), before.source());
        assert_eq!(observed.request(), &request());
        assert_eq!(observed.operation(), "pdf.text.retained_ascii");
        assert_eq!(observed.mode(), mode);
        assert_eq!(observed.output_id(), output.id());
        assert_eq!(observed.output_len(), output.len());
        assert_eq!(observed.engine_version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(observed.contract_version(), 1);
        assert_eq!(observed.required_checks(), &all_required());
        assert_eq!(observed.evidence().checks().len(), 21);
        for expected in all_required().codes() {
            let found: Vec<_> = observed
                .evidence()
                .checks()
                .iter()
                .filter(|check| &check.code == expected)
                .collect();
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].status, CheckStatus::Passed);
        }
        for name in ["appearance", "accessibility", "opaque_semantics"] {
            assert_eq!(
                observed
                    .evidence()
                    .checks()
                    .iter()
                    .find(|check| check.code == CheckCode::new("pdf.scope", name).unwrap())
                    .unwrap()
                    .status,
                CheckStatus::NotChecked
            );
        }
        assert_eq!(observed.changed_objects().len(), 1);
        assert_eq!(observed.context_limits().max_work_units, 1_000_000);
        unchanged(&revision, before, "A");
        let receipt = revision
            .accept_text_edit(prepared, &request(), 1, &all_required(), &context())
            .unwrap();
        assert_eq!(receipt.after(), revision.snapshot());
        assert_eq!(revision.text().unwrap(), ["B"]);
        assert_eq!(
            PdfEngine::default()
                .open_source(output, OpenOptions::default())
                .unwrap()
                .extract_text_spans()
                .unwrap()
                .spans[0]
                .text,
            "B"
        );
        let second = SurgicalTextEditRequest {
            old_text: "B".into(),
            replacement: "C".into(),
            match_index: 0,
        };
        let prepared = revision
            .prepare_text_edit(second.clone(), mode, 1, &context())
            .unwrap();
        let receipt = revision
            .accept_text_edit(prepared, &second, 1, &all_required(), &context())
            .unwrap();
        assert_eq!(receipt.before().revision().value(), 1);
        assert_eq!(receipt.after().revision().value(), 2);
        assert_eq!(receipt.request(), &second);
        assert_eq!(revision.text().unwrap(), ["C"]);
    }
}

// Defect caught: acceptance skipping request/version/cancellation/consumer coverage policy.
#[test]
fn acceptance_refusals_preserve_prior_revision() {
    let mut revision = revision();
    let before = revision.snapshot();
    let outcome = prepared(&revision);
    let mut wrong = request();
    wrong.match_index = 1;
    assert_eq!(
        revision
            .accept_text_edit(outcome, &wrong, 1, &all_required(), &context())
            .unwrap_err(),
        PdfTextEditError::RequestMismatch
    );
    unchanged(&revision, before, "A");
    let outcome = prepared(&revision);
    assert_eq!(
        revision
            .accept_text_edit(outcome, &request(), 2, &all_required(), &context())
            .unwrap_err(),
        PdfTextEditError::ContractVersion {
            expected: 2,
            actual: 1
        }
    );
    unchanged(&revision, before, "A");
    let outcome = prepared(&revision);
    let ctx = context();
    ctx.cancellation().cancel();
    assert_eq!(
        revision
            .accept_text_edit(outcome, &request(), 1, &all_required(), &ctx)
            .unwrap_err(),
        PdfTextEditError::Context(binas_core::ContextError::Cancelled)
    );
    unchanged(&revision, before, "A");
    for (namespace, name, expected) in [
        (
            "pdf.scope",
            "appearance",
            EvidenceError::NonPassingCheck {
                code: CheckCode::new("pdf.scope", "appearance").unwrap(),
                status: CheckStatus::NotChecked,
            },
        ),
        (
            "consumer",
            "unknown",
            EvidenceError::UnknownCheck(CheckCode::new("consumer", "unknown").unwrap()),
        ),
    ] {
        let outcome = prepared(&revision);
        let policy = RequiredChecks::new(vec![CheckCode::new(namespace, name).unwrap()]).unwrap();
        assert_eq!(
            revision
                .accept_text_edit(outcome, &request(), 1, &policy, &context())
                .unwrap_err(),
            PdfTextEditError::Evidence(expected)
        );
        unchanged(&revision, before, "A");
    }
    // A consumer may require a subset; the engine still verifies its full mandatory eighteen.
    let outcome = prepared(&revision);
    let subset =
        RequiredChecks::new(vec![CheckCode::new("pdf.writer", "reparse").unwrap()]).unwrap();
    revision
        .accept_text_edit(outcome, &request(), 1, &subset, &context())
        .unwrap();
    assert_eq!(revision.snapshot().revision().value(), 1);
    assert_eq!(revision.text().unwrap(), ["B"]);
}

// Defect caught: source/revision equality standing in for actual originating object-store lineage.
#[test]
fn independent_equal_source_and_divergent_equal_revision_are_foreign_but_shared_clone_is_admitted()
{
    let original = revision();
    let mut independent = revision();
    let before = independent.snapshot();
    assert_eq!(original.snapshot(), before);
    assert_eq!(
        independent
            .accept_text_edit(
                prepared(&original),
                &request(),
                1,
                &all_required(),
                &context()
            )
            .unwrap_err(),
        PdfTextEditError::ForeignRevision
    );
    unchanged(&independent, before, "A");
    let mut shared = original.clone();
    shared
        .accept_text_edit(
            prepared(&original),
            &request(),
            1,
            &all_required(),
            &context(),
        )
        .unwrap();
    assert_eq!(shared.text().unwrap(), ["B"]);
    unchanged(&original, original.snapshot(), "A");
    let mut branch = original.clone();
    let branch_request = SurgicalTextEditRequest {
        replacement: "C".into(),
        ..request()
    };
    let branch_prepared = branch
        .prepare_text_edit(
            branch_request.clone(),
            PdfWriterMode::Incremental,
            1,
            &context(),
        )
        .unwrap();
    branch
        .accept_text_edit(
            branch_prepared,
            &branch_request,
            1,
            &all_required(),
            &context(),
        )
        .unwrap();
    assert_eq!(branch.snapshot(), shared.snapshot());
    let branch_before = branch.snapshot();
    let next = SurgicalTextEditRequest {
        old_text: "B".into(),
        replacement: "D".into(),
        match_index: 0,
    };
    let outcome = shared
        .prepare_text_edit(next.clone(), PdfWriterMode::Incremental, 1, &context())
        .unwrap();
    assert_eq!(
        branch
            .accept_text_edit(outcome, &next, 1, &all_required(), &context())
            .unwrap_err(),
        PdfTextEditError::ForeignRevision
    );
    unchanged(&branch, branch_before, "C");
}

// Defect caught: accepting a stale successful operation twice.
#[test]
fn parallel_preparations_become_stale_after_one_is_accepted() {
    let mut revision = revision();
    let first = prepared(&revision);
    let second = prepared(&revision);
    let before = revision.snapshot();
    revision
        .accept_text_edit(first, &request(), 1, &all_required(), &context())
        .unwrap();
    let after = revision.snapshot();
    assert_eq!(
        revision
            .accept_text_edit(second, &request(), 1, &all_required(), &context())
            .unwrap_err(),
        PdfTextEditError::Stale {
            planned: before,
            current: after
        }
    );
    unchanged(&revision, after, "B");
}

// Defect caught: performing work before negotiation or broadening the single raw ASCII request contract.
#[test]
fn rejected_negotiation_and_requests_do_not_publish_or_spend_work() {
    let revision = revision();
    let before = revision.snapshot();
    for (edit, mode, contract, expected) in [
        (
            request(),
            PdfWriterMode::Incremental,
            9,
            PdfTextEditError::ContractVersion {
                expected: 9,
                actual: 1,
            },
        ),
        (
            request(),
            PdfWriterMode::Surgical,
            1,
            PdfTextEditError::Refused(PdfTextEditRefusal::UnsupportedWriterMode),
        ),
        (
            SurgicalTextEditRequest {
                replacement: "".into(),
                ..request()
            },
            PdfWriterMode::Compact,
            1,
            PdfTextEditError::Refused(PdfTextEditRefusal::EmptyRequest),
        ),
        (
            SurgicalTextEditRequest {
                replacement: "é".into(),
                ..request()
            },
            PdfWriterMode::Compact,
            1,
            PdfTextEditError::Refused(PdfTextEditRefusal::NonAsciiRequest),
        ),
    ] {
        let ctx = context();
        assert_eq!(
            revision
                .prepare_text_edit(edit, mode, contract, &ctx)
                .unwrap_err(),
            expected
        );
        assert_eq!(ctx.budget().snapshot().work_units, 0);
        unchanged(&revision, before, "A");
    }
}

// Defect caught: exposing a committed revision after export/cancellation/resource failure.
#[test]
fn preparation_export_and_resource_failures_leave_original_revision_unpublished() {
    let revision = revision();
    let before = revision.snapshot();
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision
            .prepare_text_edit(request(), PdfWriterMode::Incremental, 1, &cancelled)
            .unwrap_err(),
        PdfTextEditError::Context(binas_core::ContextError::Cancelled)
    );
    unchanged(&revision, before, "A");
    let limited = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1_000_000,
            max_work_units: 0,
        }),
        CancellationToken::new(),
    );
    assert!(
        matches!(revision.prepare_text_edit(request(), PdfWriterMode::Incremental, 1, &limited), Err(PdfTextEditError::Pdf(error)) if error.code == PdfErrorCode::ResourceLimit)
    );
    unchanged(&revision, before, "A");
    let mut input = include_bytes!("fixtures/preservation/metadata.pdf").to_vec();
    let start = input
        .windows(9)
        .position(|value| value == b"/F1 12 Tf")
        .unwrap();
    input[start..start + 9].fill(b' ');
    let mut config = EngineConfig::default();
    config.limits.max_output_bytes = 1;
    let revision = PdfRevision::new(
        PdfEngine::new(config)
            .open_owned(input, OpenOptions::default())
            .unwrap(),
    );
    let before = revision.snapshot();
    let ctx = context();
    assert_eq!(
        revision
            .text_edit_capability("pdf", "pdf.text.retained_ascii", PdfWriterMode::Incremental)
            .limits()
            .max_output_bytes,
        1
    );
    assert!(
        matches!(revision.prepare_text_edit(request(), PdfWriterMode::Incremental, 1, &ctx), Err(PdfTextEditError::Pdf(error)) if error.code == PdfErrorCode::ResourceLimit)
    );
    unchanged(&revision, before, "A");
    assert!(ctx.budget().snapshot().work_units > 0);
    assert_eq!(ctx.budget().snapshot().live_bytes, 0);
}

// Defect caught: acceptance ignoring a finite work budget and publishing despite exhaustion.
#[test]
fn zero_work_acceptance_context_refuses_actual_prepared_edit_without_publication() {
    let mut revision = revision();
    let before = revision.snapshot();
    let outcome = prepared(&revision);
    let ctx = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1_000_000,
            max_work_units: 0,
        }),
        CancellationToken::new(),
    );
    let error = revision
        .accept_text_edit(outcome, &request(), 1, &all_required(), &ctx)
        .unwrap_err();
    assert_eq!(
        error,
        PdfTextEditError::Context(binas_core::ContextError::Budget(
            binas_core::BudgetError::LimitExceeded {
                resource: binas_core::BudgetResource::WorkUnits,
                used: 0,
                requested: 1,
                limit: 0,
            }
        ))
    );
    unchanged(&revision, before, "A");
    assert_eq!(ctx.budget().snapshot().work_units, 0);
}
