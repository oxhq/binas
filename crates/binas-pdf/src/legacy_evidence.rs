//! Compatibility interpretation of the original PDF operation receipts.
//!
//! Conversion only translates the old named checks. It adds no source identity,
//! revision binding, rendering proof, standards conformance or wider preservation
//! assurance. Call `EvidenceReport::validate` with the same explicit obligations
//! before accepting the result. A global legacy boolean never supplies a check.

use binas_core::{
    Diagnostic, Severity,
    legacy::{Invariant, Verification},
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};

/// PDF meanings retained solely for interpreting historical operation receipts.
/// Scope and completeness remain those of the producing legacy operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfLegacyInvariant {
    /// The producing operation reopened its serialized PDF.
    Reparse,
    /// Its old-content selection check no longer found the requested old text.
    OldGone,
    /// Its new-content selection check found the requested replacement.
    NewSelectable,
    /// The page-count comparison performed by the producer passed.
    PageCountUnchanged,
    /// The producing operation reported no fallback strategy.
    NoFallback,
}

impl PdfLegacyInvariant {
    pub fn check_code(self) -> CheckCode {
        let name = match self {
            Self::Reparse => "reparse",
            Self::OldGone => "old-gone",
            Self::NewSelectable => "new-selectable",
            Self::PageCountUnchanged => "page-count-unchanged",
            Self::NoFallback => "no-fallback",
        };
        CheckCode::new("pdf.legacy", name).expect("fixed legacy PDF check code")
    }

    fn recognizes(code: &CheckCode) -> bool {
        [
            Self::Reparse,
            Self::OldGone,
            Self::NewSelectable,
            Self::PageCountUnchanged,
            Self::NoFallback,
        ]
        .into_iter()
        .any(|invariant| invariant.check_code() == *code)
    }
}

impl From<Invariant> for PdfLegacyInvariant {
    fn from(value: Invariant) -> Self {
        match value {
            Invariant::Reparse => Self::Reparse,
            Invariant::OldGone => Self::OldGone,
            Invariant::NewSelectable => Self::NewSelectable,
            Invariant::PageCountUnchanged => Self::PageCountUnchanged,
            Invariant::NoFallback => Self::NoFallback,
        }
    }
}

impl From<PdfLegacyInvariant> for Invariant {
    fn from(value: PdfLegacyInvariant) -> Self {
        match value {
            PdfLegacyInvariant::Reparse => Self::Reparse,
            PdfLegacyInvariant::OldGone => Self::OldGone,
            PdfLegacyInvariant::NewSelectable => Self::NewSelectable,
            PdfLegacyInvariant::PageCountUnchanged => Self::PageCountUnchanged,
            PdfLegacyInvariant::NoFallback => Self::NoFallback,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PdfLegacyEvidenceDetail {
    /// Original per-check boolean; None means the legacy receipt omitted it.
    pub legacy_passed: Option<bool>,
    pub diagnostic: Option<Diagnostic>,
}

/// Translate historical checks without broadening their assurance.
///
/// Missing known checks become NotChecked, unsupported obligations become
/// Unsupported, and duplicate/extra checks remain visible to exact validation.
/// A false global result demotes otherwise passing checks. Diagnostics carry no
/// typed coverage in the old format: even informational diagnostics conservatively
/// prevent an automatic pass, and their original contents remain available.
pub fn legacy_verification_evidence(
    legacy: &Verification,
    required: &RequiredChecks,
) -> EvidenceReport<PdfLegacyEvidenceDetail> {
    let mut checks = legacy
        .checks
        .iter()
        .map(|check| {
            let status = match &check.diagnostic {
                Some(diagnostic) if diagnostic.code == "unsupported_feature" => {
                    CheckStatus::Unsupported
                }
                Some(diagnostic) if diagnostic.severity == Severity::Error => CheckStatus::Failed,
                _ if !legacy.passed || !check.passed => CheckStatus::Failed,
                Some(_) => CheckStatus::NotChecked,
                None => CheckStatus::Passed,
            };
            CheckEvidence {
                code: PdfLegacyInvariant::from(check.invariant).check_code(),
                status,
                detail: PdfLegacyEvidenceDetail {
                    legacy_passed: Some(check.passed),
                    diagnostic: check.diagnostic.clone(),
                },
            }
        })
        .collect::<Vec<_>>();
    for code in required.codes() {
        if !checks.iter().any(|check| &check.code == code) {
            checks.push(CheckEvidence {
                code: code.clone(),
                status: if PdfLegacyInvariant::recognizes(code) {
                    CheckStatus::NotChecked
                } else {
                    CheckStatus::Unsupported
                },
                detail: PdfLegacyEvidenceDetail {
                    legacy_passed: None,
                    diagnostic: None,
                },
            });
        }
    }
    EvidenceReport::new(checks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use binas_core::legacy::InvariantCheck;
    use std::collections::BTreeMap;

    fn required(invariants: &[PdfLegacyInvariant]) -> RequiredChecks {
        RequiredChecks::new(invariants.iter().map(|value| value.check_code()).collect()).unwrap()
    }

    fn check(invariant: Invariant, passed: bool) -> InvariantCheck {
        InvariantCheck {
            invariant,
            passed,
            diagnostic: None,
        }
    }

    #[test]
    fn compatibility_mapping_is_bidirectional_and_namespaced() {
        for original in [
            Invariant::Reparse,
            Invariant::OldGone,
            Invariant::NewSelectable,
            Invariant::PageCountUnchanged,
            Invariant::NoFallback,
        ] {
            let typed = PdfLegacyInvariant::from(original);
            assert_eq!(Invariant::from(typed), original);
            assert_eq!(typed.check_code().namespace(), "pdf.legacy");
        }
        let old = Verification {
            passed: true,
            checks: vec![check(Invariant::Reparse, true)],
        };
        let required = required(&[PdfLegacyInvariant::Reparse]);
        legacy_verification_evidence(&old, &required)
            .validate(&required)
            .unwrap();
    }

    #[test]
    fn global_boolean_missing_duplicate_extra_and_contradiction_fail_closed() {
        let required = required(&[PdfLegacyInvariant::Reparse]);
        for old in [
            Verification {
                passed: true,
                checks: vec![],
            },
            Verification {
                passed: false,
                checks: vec![],
            },
            Verification {
                passed: false,
                checks: vec![check(Invariant::Reparse, true)],
            },
            Verification {
                passed: true,
                checks: vec![check(Invariant::Reparse, false)],
            },
            Verification {
                passed: true,
                checks: vec![
                    check(Invariant::Reparse, true),
                    check(Invariant::Reparse, true),
                ],
            },
            Verification {
                passed: true,
                checks: vec![
                    check(Invariant::Reparse, true),
                    check(Invariant::NoFallback, true),
                ],
            },
        ] {
            assert!(
                legacy_verification_evidence(&old, &required)
                    .validate(&required)
                    .is_err()
            );
        }
        let missing = legacy_verification_evidence(
            &Verification {
                passed: true,
                checks: vec![],
            },
            &required,
        );
        assert_eq!(missing.checks()[0].status, CheckStatus::NotChecked);
        let broader = self::required(&[
            PdfLegacyInvariant::Reparse,
            PdfLegacyInvariant::PageCountUnchanged,
        ]);
        let only_reparse = Verification {
            passed: true,
            checks: vec![check(Invariant::Reparse, true)],
        };
        let report = legacy_verification_evidence(&only_reparse, &broader);
        assert_eq!(report.checks()[1].status, CheckStatus::NotChecked);
        assert!(report.validate(&broader).is_err());
    }

    #[test]
    fn unknown_obligations_and_diagnostics_cannot_become_success() {
        let required = RequiredChecks::new(vec![
            PdfLegacyInvariant::Reparse.check_code(),
            CheckCode::new("pdf.standards", "conformance").unwrap(),
        ])
        .unwrap();
        let old = Verification {
            passed: true,
            checks: vec![check(Invariant::Reparse, true)],
        };
        let report = legacy_verification_evidence(&old, &required);
        assert_eq!(report.checks()[1].status, CheckStatus::Unsupported);
        assert!(report.validate(&required).is_err());
        let required = self::required(&[PdfLegacyInvariant::Reparse]);
        for (code, severity, status) in [
            (
                "unsupported_feature",
                Severity::Warning,
                CheckStatus::Unsupported,
            ),
            ("some_warning", Severity::Warning, CheckStatus::NotChecked),
            ("some_error", Severity::Error, CheckStatus::Failed),
        ] {
            let mut old = old.clone();
            old.checks[0].diagnostic = Some(Diagnostic {
                code: code.into(),
                severity,
                message: "original diagnostic".into(),
                span: None,
                node: None,
                context: BTreeMap::new(),
            });
            let report = legacy_verification_evidence(&old, &required);
            assert_eq!(report.checks()[0].status, status);
            assert_eq!(
                report.checks()[0].detail.diagnostic,
                old.checks[0].diagnostic
            );
            assert!(report.validate(&required).is_err());
        }
    }
}
