//! Conditional contract for one raw ASCII whole-token retained edit.
//! Evidence is same-engine graph/encoded-byte verification, not visual proof.
//! PDF decode limits and context logical accounting remain separate; neither
//! describes whole-process memory. Cancellation is cooperative and may arrive
//! after the final poll. Opaque outcomes cannot be assembled from caller claims.
use super::{CHECKS, PdfRevision, PdfRevisionRequest};
use crate::{
    CapabilityDecision, Limits, PdfError, PdfReferenceTarget, PdfWriterMode,
    SurgicalTextEditRequest, store::ObjectStore,
};
use binas_core::{
    BudgetLimits, ContextError, OperationContext, SnapshotId, Source, SourceId,
    transaction::TransactionError,
    verification::{
        CheckCode, CheckEvidence, CheckStatus, EvidenceError, EvidenceReport, RequiredChecks,
    },
};
use std::{error::Error, fmt};

const OPERATION: &str = "pdf.text.retained_ascii";
const CONTRACT: u32 = 1;
const UNCHECKED: [&str; 3] = ["appearance", "accessibility", "opaque_semantics"];
const WRITER_CHECKS: [&str; 7] = [
    "reparse",
    "header_version",
    "objects_and_encoded_payloads",
    "trailer_values",
    "references_resolve",
    "page_relationships",
    "writer_mode",
];
fn code(namespace: &str, name: &str) -> CheckCode {
    CheckCode::new(namespace, name).expect("static contract code")
}
fn mandatory() -> RequiredChecks {
    RequiredChecks::new(
        CHECKS
            .iter()
            .chain(["stream_owners", "requested_text"].iter())
            .map(|n| code("pdf.revision", n))
            .chain(WRITER_CHECKS.iter().map(|n| code("pdf.writer", n)))
            .collect(),
    )
    .expect("static contract obligations")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfTextEditRefusal {
    UnknownFormat,
    UnknownOperation,
    UnsupportedWriterMode,
    EmptyRequest,
    NonAsciiRequest,
}
#[derive(Debug, PartialEq)]
pub enum PdfTextEditError {
    Refused(PdfTextEditRefusal),
    ContractVersion {
        expected: u32,
        actual: u32,
    },
    RequestMismatch,
    Stale {
        planned: SnapshotId,
        current: SnapshotId,
    },
    ForeignRevision,
    OutputBinding,
    Context(ContextError),
    Pdf(PdfError),
    Transaction(TransactionError<PdfError>),
    Evidence(EvidenceError),
}
impl fmt::Display for PdfTextEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Context(e) => e.fmt(f),
            Self::Pdf(e) => e.fmt(f),
            Self::Transaction(e) => e.fmt(f),
            Self::Evidence(e) => e.fmt(f),
            _ => write!(f, "retained text acceptance: {self:?}"),
        }
    }
}
impl Error for PdfTextEditError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Context(e) => Some(e),
            Self::Pdf(e) => Some(e),
            Self::Transaction(e) => Some(e),
            Self::Evidence(e) => Some(e),
            _ => None,
        }
    }
}

/// Conditional eligibility; actual planning/security/ownership checks decide.
/// Limits are configured ceilings, not a promise of remaining capacity.
#[derive(Clone, Debug)]
pub struct PdfTextEditCapability {
    mode: PdfWriterMode,
    refusal: Option<PdfTextEditRefusal>,
    limits: Limits,
    required: RequiredChecks,
}
impl PdfTextEditCapability {
    pub fn format(&self) -> &'static str {
        "pdf"
    }
    pub fn operation(&self) -> &'static str {
        OPERATION
    }
    pub fn contract_version(&self) -> u32 {
        CONTRACT
    }
    pub fn engine_version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
    pub fn mode(&self) -> PdfWriterMode {
        self.mode
    }
    pub fn decision(&self) -> CapabilityDecision {
        if self.refusal.is_some() {
            CapabilityDecision::Refused
        } else {
            CapabilityDecision::Conditional
        }
    }
    pub fn refusal(&self) -> Option<PdfTextEditRefusal> {
        self.refusal
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    pub fn required_checks(&self) -> &RequiredChecks {
        &self.required
    }
}

/// Observed facts from one actual plan/commit/export, with immutable association.
/// Optional NotChecked scope facts must never count as required Passed checks.
#[derive(Debug)]
pub struct PdfTextEditReceipt {
    capability: PdfTextEditCapability,
    input_id: SourceId,
    input_len: usize,
    before: SnapshotId,
    after: SnapshotId,
    request: SurgicalTextEditRequest,
    output_id: SourceId,
    output_len: usize,
    changed_objects: Vec<PdfReferenceTarget>,
    evidence: EvidenceReport<String>,
    context_limits: BudgetLimits,
}
impl PdfTextEditReceipt {
    pub fn input_id(&self) -> SourceId {
        self.input_id
    }
    pub fn input_len(&self) -> usize {
        self.input_len
    }
    pub fn before(&self) -> SnapshotId {
        self.before
    }
    pub fn after(&self) -> SnapshotId {
        self.after
    }
    pub fn request(&self) -> &SurgicalTextEditRequest {
        &self.request
    }
    pub fn operation(&self) -> &'static str {
        self.capability.operation()
    }
    pub fn mode(&self) -> PdfWriterMode {
        self.capability.mode()
    }
    pub fn output_id(&self) -> SourceId {
        self.output_id
    }
    pub fn output_len(&self) -> usize {
        self.output_len
    }
    pub fn engine_version(&self) -> &'static str {
        self.capability.engine_version()
    }
    pub fn contract_version(&self) -> u32 {
        self.capability.contract_version()
    }
    pub fn required_checks(&self) -> &RequiredChecks {
        self.capability.required_checks()
    }
    /// Includes eighteen checked obligations and three optional NotChecked facts.
    /// Raw core `validate(required_checks())` rejects the optional extra entries;
    /// use `accept_text_edit` to validate mandatory and caller-selected scopes.
    pub fn evidence(&self) -> &EvidenceReport<String> {
        &self.evidence
    }
    pub fn changed_objects(&self) -> &[PdfReferenceTarget] {
        &self.changed_objects
    }
    pub fn pdf_limits(&self) -> &Limits {
        self.capability.limits()
    }
    /// Preparation context ceilings; acceptance independently uses its caller context.
    /// These limits neither promise remaining capacity nor account for process memory.
    pub fn context_limits(&self) -> BudgetLimits {
        self.context_limits
    }
}

#[derive(Debug)]
pub struct PdfPreparedTextEdit {
    receipt: PdfTextEditReceipt,
    output: Source,
    base_store: ObjectStore,
    candidate: PdfRevision,
}
impl PdfPreparedTextEdit {
    pub fn capability(&self) -> &PdfTextEditCapability {
        &self.receipt.capability
    }
    pub fn receipt(&self) -> &PdfTextEditReceipt {
        &self.receipt
    }
    pub fn output(&self) -> &Source {
        &self.output
    }
}
fn version(expected: u32) -> Result<(), PdfTextEditError> {
    if expected != CONTRACT {
        return Err(PdfTextEditError::ContractVersion {
            expected,
            actual: CONTRACT,
        });
    }
    Ok(())
}
fn validate_coverage(
    report: &EvidenceReport<String>,
    required: &RequiredChecks,
) -> Result<(), PdfTextEditError> {
    let known = mandatory();
    for requested in required.codes() {
        if !(known.codes().any(|code| code == requested)
            || requested.namespace() == "pdf.scope" && UNCHECKED.contains(&requested.code()))
        {
            return Err(PdfTextEditError::Evidence(EvidenceError::UnknownCheck(
                requested.clone(),
            )));
        }
    }
    // Preserve duplicates/statuses. Only unchecked or other known scope facts
    // outside the requested policy are excluded from core's exact validation.
    let selected = report
        .checks()
        .iter()
        .filter(|check| required.codes().any(|code| code == &check.code))
        .cloned()
        .collect();
    EvidenceReport::new(selected)
        .validate(required)
        .map_err(PdfTextEditError::Evidence)
}
impl PdfRevision {
    pub fn text_edit_capability(
        &self,
        format: &str,
        operation: &str,
        mode: PdfWriterMode,
    ) -> PdfTextEditCapability {
        let refusal = if format != "pdf" {
            Some(PdfTextEditRefusal::UnknownFormat)
        } else if operation != OPERATION {
            Some(PdfTextEditRefusal::UnknownOperation)
        } else if mode == PdfWriterMode::Surgical {
            Some(PdfTextEditRefusal::UnsupportedWriterMode)
        } else {
            None
        };
        PdfTextEditCapability {
            mode,
            refusal,
            limits: self.state.value().engine_config().limits.clone(),
            required: mandatory(),
        }
    }
    /// Runs plan, commit and checked export on an unpublished clone. Failures
    /// retain the live revision; cumulative attempted work is not refunded.
    pub fn prepare_text_edit(
        &self,
        request: SurgicalTextEditRequest,
        mode: PdfWriterMode,
        expected_contract: u32,
        context: &OperationContext,
    ) -> Result<PdfPreparedTextEdit, PdfTextEditError> {
        version(expected_contract)?;
        context.checkpoint().map_err(PdfTextEditError::Context)?;
        let capability = self.text_edit_capability("pdf", OPERATION, mode);
        if let Some(reason) = capability.refusal {
            return Err(PdfTextEditError::Refused(reason));
        }
        if request.old_text.is_empty() || request.replacement.is_empty() {
            return Err(PdfTextEditError::Refused(PdfTextEditRefusal::EmptyRequest));
        }
        if !request.old_text.is_ascii() || !request.replacement.is_ascii() {
            return Err(PdfTextEditError::Refused(
                PdfTextEditRefusal::NonAsciiRequest,
            ));
        }
        let mut candidate = self.clone();
        let plan = candidate
            .plan(
                PdfRevisionRequest {
                    text: vec![request.clone()],
                    ..Default::default()
                },
                context,
            )
            .map_err(PdfTextEditError::Pdf)?;
        let committed = candidate
            .commit(&plan, context)
            .map_err(PdfTextEditError::Transaction)?;
        committed
            .evidence
            .validate(&committed.required)
            .map_err(PdfTextEditError::Evidence)?;
        let exported = candidate
            .export(mode, context)
            .map_err(PdfTextEditError::Pdf)?;
        let mut checks = committed.evidence.checks().to_vec();
        checks.extend_from_slice(exported.evidence.checks());
        EvidenceReport::new(checks.clone())
            .validate(&capability.required)
            .map_err(PdfTextEditError::Evidence)?;
        for name in UNCHECKED {
            checks.push(CheckEvidence {code: code("pdf.scope", name), status: CheckStatus::NotChecked, detail: "No independent visual, accessibility, opaque-meaning or standards-conformance assurance".into()});
        }
        context.checkpoint().map_err(PdfTextEditError::Context)?;
        Ok(PdfPreparedTextEdit {
            receipt: PdfTextEditReceipt {
                capability,
                input_id: self.state.value().source_id(),
                input_len: self.state.value().source_len(),
                before: committed.before,
                after: exported.snapshot,
                request,
                output_id: exported.output.id(),
                output_len: exported.output.len(),
                changed_objects: committed.changes,
                evidence: EvidenceReport::new(checks),
                context_limits: context.budget().limits(),
            },
            output: exported.output,
            base_store: self.state.value().parsed().objects.clone(),
            candidate,
        })
    }
    /// Publish only this associated candidate after contract/policy/lineage
    /// checks. One coarse work unit admits validation; it is not CPU accounting.
    /// Cancellation arriving after the final checkpoint may follow commit.
    pub fn accept_text_edit(
        &mut self,
        prepared: PdfPreparedTextEdit,
        expected_request: &SurgicalTextEditRequest,
        expected_contract: u32,
        required: &RequiredChecks,
        context: &OperationContext,
    ) -> Result<PdfTextEditReceipt, PdfTextEditError> {
        version(expected_contract)?;
        context.charge_work(1).map_err(PdfTextEditError::Context)?;
        let receipt = &prepared.receipt;
        if receipt.request != *expected_request {
            return Err(PdfTextEditError::RequestMismatch);
        }
        if receipt.before != self.snapshot() {
            return Err(PdfTextEditError::Stale {
                planned: receipt.before,
                current: self.snapshot(),
            });
        }
        if !self
            .state
            .value()
            .parsed()
            .objects
            .shares_snapshot_with(&prepared.base_store)
        {
            return Err(PdfTextEditError::ForeignRevision);
        }
        if receipt.contract_version() != expected_contract
            || receipt.input_id != self.state.value().source_id()
            || receipt.input_len != self.state.value().source_len()
            || prepared.candidate.snapshot() != receipt.after
            || receipt.before.source() != receipt.after.source()
            || receipt.before.revision().checked_next() != Some(receipt.after.revision())
            || receipt.output_id != prepared.output.id()
            || receipt.output_len != prepared.output.len()
        {
            return Err(PdfTextEditError::OutputBinding);
        }
        validate_coverage(receipt.evidence(), &mandatory())?;
        validate_coverage(receipt.evidence(), required)?;
        context.checkpoint().map_err(PdfTextEditError::Context)?;
        *self = prepared.candidate;
        Ok(prepared.receipt)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn required() -> RequiredChecks {
        RequiredChecks::new(vec![code("pdf.writer", "reparse")]).unwrap()
    }
    fn check(status: CheckStatus) -> CheckEvidence<String> {
        CheckEvidence {
            code: code("pdf.writer", "reparse"),
            status,
            detail: "real receipt wiring control".into(),
        }
    }
    // Missing selected evidence must not be lost by filtering and a vacuous pass.
    #[test]
    fn coverage_rejects_missing_mandatory_check() {
        assert_eq!(
            validate_coverage(&EvidenceReport::new(vec![]), &required()).unwrap_err(),
            PdfTextEditError::Evidence(EvidenceError::MissingCheck(code("pdf.writer", "reparse")))
        );
    }
    // Filtering must preserve duplicates so exact core validation can reject them.
    #[test]
    fn coverage_rejects_duplicate_required_check() {
        assert_eq!(
            validate_coverage(
                &EvidenceReport::new(vec![check(CheckStatus::Passed), check(CheckStatus::Passed)]),
                &required()
            )
            .unwrap_err(),
            PdfTextEditError::Evidence(EvidenceError::DuplicateCheck(code(
                "pdf.writer",
                "reparse"
            )))
        );
    }
    // Failed, unchecked or unsupported selected coverage cannot become a pass.
    #[test]
    fn coverage_rejects_each_nonpassing_required_status() {
        for status in [
            CheckStatus::Failed,
            CheckStatus::NotChecked,
            CheckStatus::Unsupported,
        ] {
            assert_eq!(
                validate_coverage(&EvidenceReport::new(vec![check(status)]), &required())
                    .unwrap_err(),
                PdfTextEditError::Evidence(EvidenceError::NonPassingCheck {
                    code: code("pdf.writer", "reparse"),
                    status
                })
            );
        }
    }
    // A standalone invented report must not define the engine's known operation scope.
    #[test]
    fn coverage_rejects_unknown_requirement_even_with_claimed_passing_evidence() {
        let unknown = code("consumer", "invented");
        let report = EvidenceReport::new(vec![CheckEvidence {
            code: unknown.clone(),
            status: CheckStatus::Passed,
            detail: "invented".into(),
        }]);
        let required = RequiredChecks::new(vec![unknown.clone()]).unwrap();
        assert_eq!(
            validate_coverage(&report, &required).unwrap_err(),
            PdfTextEditError::Evidence(EvidenceError::UnknownCheck(unknown))
        );
    }
    // Optional unchecked facts are retained in the receipt and excluded from an unselected scope.
    #[test]
    fn coverage_allows_known_subset_and_rejects_selected_unchecked_scope() {
        let appearance = code("pdf.scope", "appearance");
        let report = EvidenceReport::new(vec![
            check(CheckStatus::Passed),
            CheckEvidence {
                code: appearance.clone(),
                status: CheckStatus::NotChecked,
                detail: "unchecked".into(),
            },
        ]);
        assert_eq!(validate_coverage(&report, &required()), Ok(()));
        let required = RequiredChecks::new(vec![appearance.clone()]).unwrap();
        assert_eq!(
            validate_coverage(&report, &required).unwrap_err(),
            PdfTextEditError::Evidence(EvidenceError::NonPassingCheck {
                code: appearance,
                status: CheckStatus::NotChecked
            })
        );
    }
}
