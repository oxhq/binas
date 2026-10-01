//! Compatibility transport for the original stateless plan/report API.
//!
//! These declarations retain their historical Rust and JSON shapes. Invariant
//! names are legacy labels, not active format-neutral semantic contracts; the
//! format adapter owns their interpretation. New operations use source-bound
//! transactions and namespaced, coverage-aware evidence from `verification`.
//! A legacy `passed` boolean is not evidence of universal preservation.

use crate::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PlanId(pub String);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Invariant {
    Reparse,
    OldGone,
    NewSelectable,
    PageCountUnchanged,
    NoFallback,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EditPlan {
    pub id: PlanId,
    pub operation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invariants: Vec<Invariant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InvariantCheck {
    pub invariant: Invariant,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<Diagnostic>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Verification {
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<InvariantCheck>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Report {
    pub operation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<Verification>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn original_plan_and_report_json_roundtrip_exactly() {
        let plan_json = json!({
            "id":"plan-1", "operation":"replace_text",
            "invariants":["reparse","old-gone","new-selectable","page-count-unchanged","no-fallback"],
            "warnings":[{"code":"legacy_warning","severity":"warning","message":"retained"}],
            "metadata":{"strategy":"surgical"}
        });
        let plan: crate::EditPlan = serde_json::from_value(plan_json.clone()).unwrap();
        let _: &EditPlan = &plan;
        assert_eq!(serde_json::to_value(plan).unwrap(), plan_json);
        let report_json = json!({
            "operation":"replace_text", "metadata":{"bytes":8},
            "diagnostics":[{"code":"legacy_info","severity":"info","message":"retained"}],
            "verification":{"passed":false,"checks":[{
                "invariant":"reparse","passed":false,
                "diagnostic":{"code":"failure","severity":"error","message":"retained"}
            }]}
        });
        let report: crate::Report = serde_json::from_value(report_json.clone()).unwrap();
        let _: &Report = &report;
        assert_eq!(serde_json::to_value(report).unwrap(), report_json);
        assert_eq!(
            serde_json::to_value(Verification::default()).unwrap(),
            json!({"passed":false})
        );
        let minimal: EditPlan = serde_json::from_value(json!({"id":"p","operation":"op"})).unwrap();
        assert_eq!(
            serde_json::to_value(minimal).unwrap(),
            json!({"id":"p","operation":"op"})
        );
    }
}
