//! Coverage-aware aggregation of supplied rule findings. No scanner, validator,
//! metadata parser or repair pass is implemented by this module.
use super::{Coverage, STANDARDS_REGISTRY_VERSION, StandardTarget, target_definition};
use binas_core::SnapshotId;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct RuleId(String);
impl RuleId {
    /// Validate borrowed text before copying it into the retained identifier.
    pub fn new(value: impl AsRef<str>) -> Result<Self, ReportError> {
        let value = value.as_ref();
        if value.len() > 160
            || !value
                .split_once(':')
                .is_some_and(|(namespace, name)| !namespace.is_empty() && !name.is_empty())
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._:-/".contains(&b))
        {
            return Err(ReportError::InvalidRuleId);
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Applicability {
    Applicable,
    NotApplicable,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleOutcome {
    Passed,
    Failed,
    NotChecked,
    Unsupported,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleImplementation {
    Implemented,
    Unimplemented,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredRule {
    pub id: RuleId,
    pub implementation: RuleImplementation,
}
/// One aggregated finding per rule; object-level evidence may be retained by the
/// caller. A NotApplicable finding still needs an evidence/reason string.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleFinding {
    pub rule: RuleId,
    pub applicability: Applicability,
    pub outcome: RuleOutcome,
    pub evidence: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConformanceResult {
    Passed,
    Failed,
    Incomplete,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HumanReviewStatus {
    NotAssessed,
    Required,
    Incomplete,
    Completed,
    NotRequired,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoverageGap {
    NormativeInventoryUnenumerated,
    AmendmentsAndErrataNotFrozen,
    MissingRule(RuleId),
    UnimplementedRule(RuleId),
    UnknownApplicability(RuleId),
    UnevaluatedRule(RuleId),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReportError {
    InvalidRuleId,
    LimitExceeded,
    EmptyEvidence(RuleId),
    DuplicateRequiredRule(RuleId),
    DuplicateFinding(RuleId),
    UnknownFinding(RuleId),
    ContradictoryFinding(RuleId),
}
impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid standards report: {self:?}")
    }
}
impl Error for ReportError {}

#[derive(Clone, Debug)]
pub struct StandardsReport {
    snapshot: SnapshotId,
    target: StandardTarget,
    declared_targets: Vec<StandardTarget>,
    findings: Vec<RuleFinding>,
    evaluated_rules_result: ConformanceResult,
    conformance: ConformanceResult,
    coverage_gaps: Vec<CoverageGap>,
    human_review: HumanReviewStatus,
}
impl StandardsReport {
    /// Aggregates a bounded supplied rule set. Required rules are NOT an asserted
    /// complete normative inventory: that authority remains the versioned registry.
    /// The current registry has no complete target inventory, hence no target can
    /// pass conformance. Applicable failures still report Failed rather than being
    /// hidden behind incomplete coverage. Dependencies are not automatically checked.
    pub fn evaluate(
        snapshot: SnapshotId,
        target: StandardTarget,
        declared_targets: Vec<StandardTarget>,
        required: &[RequiredRule],
        findings: Vec<RuleFinding>,
        human_review: HumanReviewStatus,
    ) -> Result<Self, ReportError> {
        if required.len() > 4096
            || findings.len() > 4096
            || declared_targets.len() > 27
            || findings.iter().any(|f| f.evidence.len() > 4096)
        {
            return Err(ReportError::LimitExceeded);
        }
        let mut rules = BTreeMap::new();
        for rule in required {
            if rules.insert(&rule.id, rule.implementation).is_some() {
                return Err(ReportError::DuplicateRequiredRule(rule.id.clone()));
            }
        }
        let mut seen = BTreeSet::new();
        for finding in &findings {
            if finding.evidence.trim().is_empty() {
                return Err(ReportError::EmptyEvidence(finding.rule.clone()));
            }
            let Some(implementation) = rules.get(&finding.rule) else {
                return Err(ReportError::UnknownFinding(finding.rule.clone()));
            };
            if !seen.insert(&finding.rule) {
                return Err(ReportError::DuplicateFinding(finding.rule.clone()));
            }
            if (finding.applicability != Applicability::Applicable
                && matches!(finding.outcome, RuleOutcome::Passed | RuleOutcome::Failed))
                || (finding.applicability == Applicability::NotApplicable
                    && finding.outcome != RuleOutcome::NotChecked)
                || (*implementation == RuleImplementation::Unimplemented
                    && matches!(finding.outcome, RuleOutcome::Passed | RuleOutcome::Failed))
            {
                return Err(ReportError::ContradictoryFinding(finding.rule.clone()));
            }
        }
        let definition = target_definition(target);
        let mut gaps = Vec::new();
        if definition.normative_rule_inventory != Coverage::Complete {
            gaps.push(CoverageGap::NormativeInventoryUnenumerated);
        }
        if !definition.baseline.amendments_and_errata_frozen {
            gaps.push(CoverageGap::AmendmentsAndErrataNotFrozen);
        }
        let mut failed = false;
        let mut incomplete = required.is_empty();
        let by_id: BTreeMap<_, _> = findings.iter().map(|f| (&f.rule, f)).collect();
        for rule in required {
            if rule.implementation == RuleImplementation::Unimplemented {
                incomplete = true;
                gaps.push(CoverageGap::UnimplementedRule(rule.id.clone()));
            }
            match by_id.get(&rule.id) {
                None => {
                    incomplete = true;
                    gaps.push(CoverageGap::MissingRule(rule.id.clone()));
                }
                Some(finding) => match finding.applicability {
                    Applicability::Unknown => {
                        incomplete = true;
                        gaps.push(CoverageGap::UnknownApplicability(rule.id.clone()));
                    }
                    Applicability::NotApplicable => {}
                    Applicability::Applicable => match finding.outcome {
                        RuleOutcome::Failed => failed = true,
                        RuleOutcome::Passed => {}
                        RuleOutcome::NotChecked | RuleOutcome::Unsupported => {
                            incomplete = true;
                            gaps.push(CoverageGap::UnevaluatedRule(rule.id.clone()));
                        }
                    },
                },
            }
        }
        let evaluated_rules_result = if failed {
            ConformanceResult::Failed
        } else if incomplete {
            ConformanceResult::Incomplete
        } else {
            ConformanceResult::Passed
        };
        let conformance = if failed {
            ConformanceResult::Failed
        } else if definition.normative_rule_inventory == Coverage::Complete
            && definition.baseline.amendments_and_errata_frozen
        {
            evaluated_rules_result
        } else {
            ConformanceResult::Incomplete
        };
        let human_review = if definition.semantic_review_required
            && human_review == HumanReviewStatus::NotRequired
        {
            HumanReviewStatus::Required
        } else {
            human_review
        };
        Ok(Self {
            snapshot,
            target,
            declared_targets,
            findings,
            evaluated_rules_result,
            conformance,
            coverage_gaps: gaps,
            human_review,
        })
    }
    pub fn registry_version(&self) -> &'static str {
        STANDARDS_REGISTRY_VERSION
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }
    pub fn target(&self) -> StandardTarget {
        self.target
    }
    pub fn declared_targets(&self) -> &[StandardTarget] {
        &self.declared_targets
    }
    pub fn findings(&self) -> &[RuleFinding] {
        &self.findings
    }
    pub fn evaluated_rules_result(&self) -> ConformanceResult {
        self.evaluated_rules_result
    }
    pub fn conformance(&self) -> ConformanceResult {
        self.conformance
    }
    pub fn coverage_gaps(&self) -> &[CoverageGap] {
        &self.coverage_gaps
    }
    pub fn human_review(&self) -> HumanReviewStatus {
        self.human_review
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use binas_core::{RevisionId, Source};
    fn snapshot() -> SnapshotId {
        SnapshotId::new(
            Source::from_owned(vec![1], 1).unwrap().id(),
            RevisionId::INITIAL,
        )
    }
    fn rule() -> RequiredRule {
        RequiredRule {
            id: RuleId::new("test:known-rule").unwrap(),
            implementation: RuleImplementation::Implemented,
        }
    }
    fn finding(outcome: RuleOutcome) -> RuleFinding {
        RuleFinding {
            rule: rule().id,
            applicability: Applicability::Applicable,
            outcome,
            evidence: "test evidence, not an implemented normative PDF rule".into(),
        }
    }
    fn report(
        required: &[RequiredRule],
        findings: Vec<RuleFinding>,
    ) -> Result<StandardsReport, ReportError> {
        StandardsReport::evaluate(
            snapshot(),
            StandardTarget::PdfUa1,
            vec![StandardTarget::PdfA2b],
            required,
            findings,
            HumanReviewStatus::Incomplete,
        )
    }
    #[test]
    fn declarations_and_empty_or_partial_success_cannot_become_conformance() {
        let empty = report(&[], vec![]).unwrap();
        assert_eq!(empty.conformance(), ConformanceResult::Incomplete);
        let successful = report(&[rule()], vec![finding(RuleOutcome::Passed)]).unwrap();
        assert_eq!(
            successful.evaluated_rules_result(),
            ConformanceResult::Passed
        );
        assert_eq!(successful.conformance(), ConformanceResult::Incomplete);
        assert_eq!(successful.human_review(), HumanReviewStatus::Incomplete);
        assert_eq!(successful.target(), StandardTarget::PdfUa1);
        assert_eq!(successful.declared_targets(), [StandardTarget::PdfA2b]);
        for definition in super::super::target_registry() {
            assert_eq!(
                StandardsReport::evaluate(
                    snapshot(),
                    definition.target,
                    vec![definition.target],
                    &[],
                    vec![],
                    HumanReviewStatus::Completed
                )
                .unwrap()
                .conformance(),
                ConformanceResult::Incomplete
            );
        }
    }
    #[test]
    fn missing_unimplemented_unknown_or_unsupported_is_incomplete_but_failure_wins() {
        let missing = report(&[rule()], vec![]).unwrap();
        assert!(
            missing
                .coverage_gaps()
                .iter()
                .any(|gap| matches!(gap, CoverageGap::MissingRule(_)))
        );
        let mut absent = rule();
        absent.implementation = RuleImplementation::Unimplemented;
        assert_eq!(
            report(&[absent], vec![]).unwrap().conformance(),
            ConformanceResult::Incomplete
        );
        assert_eq!(
            report(&[rule()], vec![finding(RuleOutcome::Unsupported)])
                .unwrap()
                .conformance(),
            ConformanceResult::Incomplete
        );
        let mut unknown = finding(RuleOutcome::NotChecked);
        unknown.applicability = Applicability::Unknown;
        assert_eq!(
            report(&[rule()], vec![unknown])
                .unwrap()
                .evaluated_rules_result(),
            ConformanceResult::Incomplete
        );
        assert_eq!(
            report(&[rule()], vec![finding(RuleOutcome::Failed)])
                .unwrap()
                .conformance(),
            ConformanceResult::Failed
        );
    }
    #[test]
    fn malformed_evidence_cannot_overwrite_failures_or_claim_unevaluated_passes() {
        assert!(matches!(
            report(
                &[rule()],
                vec![finding(RuleOutcome::Passed), finding(RuleOutcome::Failed)]
            ),
            Err(ReportError::DuplicateFinding(_))
        ));
        assert!(matches!(
            report(&[], vec![finding(RuleOutcome::Passed)]),
            Err(ReportError::UnknownFinding(_))
        ));
        let mut absent = rule();
        absent.implementation = RuleImplementation::Unimplemented;
        assert!(matches!(
            report(&[absent], vec![finding(RuleOutcome::Passed)]),
            Err(ReportError::ContradictoryFinding(_))
        ));
        let mut unknown = finding(RuleOutcome::Passed);
        unknown.applicability = Applicability::Unknown;
        assert!(matches!(
            report(&[rule()], vec![unknown]),
            Err(ReportError::ContradictoryFinding(_))
        ));
        assert!(RuleId::new("unnamespaced").is_err());
    }

    #[test]
    fn contradictory_applicability_cannot_hide_a_failure() {
        for applicability in [Applicability::NotApplicable, Applicability::Unknown] {
            let mut contradictory = finding(RuleOutcome::Failed);
            contradictory.applicability = applicability;
            assert!(matches!(
                report(&[rule()], vec![contradictory]),
                Err(ReportError::ContradictoryFinding(_))
            ));
        }
        let mut inapplicable = finding(RuleOutcome::NotChecked);
        inapplicable.applicability = Applicability::NotApplicable;
        let result = report(&[rule()], vec![inapplicable]).unwrap();
        assert_eq!(result.evaluated_rules_result(), ConformanceResult::Passed);
        assert_eq!(result.conformance(), ConformanceResult::Incomplete);

        let mut missing = rule();
        missing.id = RuleId::new("test:missing-rule").unwrap();
        let failed = report(&[rule(), missing], vec![finding(RuleOutcome::Failed)]).unwrap();
        assert_eq!(failed.evaluated_rules_result(), ConformanceResult::Failed);
        assert_eq!(failed.conformance(), ConformanceResult::Failed);
        assert!(
            failed
                .coverage_gaps()
                .iter()
                .any(|gap| matches!(gap, CoverageGap::MissingRule(_)))
        );
    }

    #[test]
    fn identifiers_and_evidence_have_checked_byte_bounds() {
        let boundary_id = format!("test:{}", "a".repeat(155));
        assert_eq!(RuleId::new(&boundary_id).unwrap().as_str(), boundary_id);
        assert!(RuleId::new(format!("{boundary_id}a")).is_err());
        assert!(RuleId::new("test:ünicode").is_err());
        let oversize = "a".repeat(1024 * 1024);
        assert!(RuleId::new(&oversize).is_err());

        for empty in ["", " \t\r\n", "\u{2003}"] {
            let mut invalid = finding(RuleOutcome::Passed);
            invalid.evidence = empty.into();
            assert!(matches!(
                report(&[rule()], vec![invalid]),
                Err(ReportError::EmptyEvidence(_))
            ));
        }
        let mut bounded = finding(RuleOutcome::Passed);
        bounded.evidence = "a".repeat(4096);
        assert!(report(&[rule()], vec![bounded.clone()]).is_ok());
        bounded.evidence.push('a');
        assert!(matches!(
            report(&[rule()], vec![bounded]),
            Err(ReportError::LimitExceeded)
        ));
    }

    #[test]
    fn rule_and_declaration_limits_reject_before_aggregation() {
        assert!(matches!(
            report(&vec![rule(); 4097], vec![]),
            Err(ReportError::LimitExceeded)
        ));
        assert!(matches!(
            report(&[rule()], vec![finding(RuleOutcome::Passed); 4097]),
            Err(ReportError::LimitExceeded)
        ));
        assert!(matches!(
            StandardsReport::evaluate(
                snapshot(),
                StandardTarget::PdfA2b,
                vec![StandardTarget::PdfA2b; 28],
                &[],
                vec![],
                HumanReviewStatus::NotAssessed,
            ),
            Err(ReportError::LimitExceeded)
        ));
        assert!(matches!(
            report(&[rule(), rule()], vec![]),
            Err(ReportError::DuplicateRequiredRule(_))
        ));
    }

    #[test]
    fn semantic_review_cannot_be_suppressed_or_imply_a_machine_pass() {
        for target in [
            StandardTarget::PdfUa1,
            StandardTarget::PdfUa2,
            StandardTarget::PdfA1a,
            StandardTarget::PdfA2a,
            StandardTarget::PdfA3a,
        ] {
            for supplied in [HumanReviewStatus::NotRequired, HumanReviewStatus::Completed] {
                let result = StandardsReport::evaluate(
                    snapshot(),
                    target,
                    vec![],
                    &[rule()],
                    vec![finding(RuleOutcome::Passed)],
                    supplied,
                )
                .unwrap();
                assert_eq!(
                    result.human_review(),
                    if supplied == HumanReviewStatus::NotRequired {
                        HumanReviewStatus::Required
                    } else {
                        HumanReviewStatus::Completed
                    }
                );
                assert_eq!(result.conformance(), ConformanceResult::Incomplete);
            }
        }
    }
}
