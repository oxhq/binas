//! Explicit, namespaced obligations and coverage-aware evidence.

use std::{collections::BTreeSet, error::Error, fmt};

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CheckCode {
    namespace: String,
    code: String,
}

impl CheckCode {
    /// Components are 1..=128 ASCII letters/digits/dots/underscores/hyphens,
    /// beginning with an ASCII letter. Colons separate components in display.
    pub fn new(
        namespace: impl Into<String>,
        code: impl Into<String>,
    ) -> Result<Self, EvidenceError> {
        let namespace = namespace.into();
        let code = code.into();
        for (component, value) in [("namespace", &namespace), ("code", &code)] {
            if value.is_empty()
                || value.len() > 128
                || !value.as_bytes()[0].is_ascii_alphabetic()
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return Err(EvidenceError::InvalidCode { component });
            }
        }
        Ok(Self { namespace, code })
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Display for CheckCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.code)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckStatus {
    Passed,
    Failed,
    NotChecked,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckEvidence<D> {
    pub code: CheckCode,
    pub status: CheckStatus,
    pub detail: D,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvidenceError {
    InvalidCode {
        component: &'static str,
    },
    EmptyObligations,
    DuplicateObligation(CheckCode),
    MissingCheck(CheckCode),
    UnknownCheck(CheckCode),
    DuplicateCheck(CheckCode),
    NonPassingCheck {
        code: CheckCode,
        status: CheckStatus,
    },
}

impl fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCode { component } => write!(f, "invalid check {component}"),
            Self::EmptyObligations => {
                f.write_str("at least one explicit check obligation is required")
            }
            Self::DuplicateObligation(code) => write!(f, "duplicate obligation {code}"),
            Self::MissingCheck(code) => write!(f, "missing check {code}"),
            Self::UnknownCheck(code) => write!(f, "unexpected check {code}"),
            Self::DuplicateCheck(code) => write!(f, "duplicate check {code}"),
            Self::NonPassingCheck { code, status } => write!(f, "check {code} is {status:?}"),
        }
    }
}
impl Error for EvidenceError {}

/// An exact, nonempty set of expected checks. A planner must include every
/// obligation needed for its operation; the core cannot infer format semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredChecks {
    codes: BTreeSet<CheckCode>,
}

impl RequiredChecks {
    pub fn new(codes: Vec<CheckCode>) -> Result<Self, EvidenceError> {
        if codes.is_empty() {
            return Err(EvidenceError::EmptyObligations);
        }
        let mut unique = BTreeSet::new();
        for code in codes {
            if !unique.insert(code.clone()) {
                return Err(EvidenceError::DuplicateObligation(code));
            }
        }
        Ok(Self { codes: unique })
    }

    pub fn codes(&self) -> impl Iterator<Item = &CheckCode> {
        self.codes.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceReport<D> {
    checks: Vec<CheckEvidence<D>>,
}

impl<D> EvidenceReport<D> {
    pub fn new(checks: Vec<CheckEvidence<D>>) -> Self {
        Self { checks }
    }
    pub fn checks(&self) -> &[CheckEvidence<D>] {
        &self.checks
    }

    /// Success is scoped to the explicit obligations, never a universal pass.
    /// Missing, extra, duplicated and non-passing evidence all fail closed.
    pub fn validate(&self, required: &RequiredChecks) -> Result<(), EvidenceError> {
        let mut seen = BTreeSet::new();
        for check in &self.checks {
            if !required.codes.contains(&check.code) {
                return Err(EvidenceError::UnknownCheck(check.code.clone()));
            }
            if !seen.insert(&check.code) {
                return Err(EvidenceError::DuplicateCheck(check.code.clone()));
            }
            if check.status != CheckStatus::Passed {
                return Err(EvidenceError::NonPassingCheck {
                    code: check.code.clone(),
                    status: check.status,
                });
            }
        }
        for code in &required.codes {
            if !seen.contains(code) {
                return Err(EvidenceError::MissingCheck(code.clone()));
            }
        }
        Ok(())
    }
}
