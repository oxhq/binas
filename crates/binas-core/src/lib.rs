use std::{collections::BTreeMap, error::Error, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod adapter;
pub mod budget;
pub mod capability;
pub mod context;
pub mod graph;
pub mod identity;
pub mod source;
pub mod transaction;
pub mod verification;

pub use adapter::Adapter;

pub use budget::{
    BudgetError, BudgetLimits, BudgetResource, BudgetSnapshot, Reservation, ResourceBudget,
};
pub use context::{CancellationToken, ContextError, OperationContext};
pub use identity::{RevisionId, SnapshotId, SourceId};
pub use source::{PayloadProvenance, Source, SourceError, SourceSpan};

pub mod span;
pub use span::{Span, SpanError};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub span: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<NodeId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Tree {
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<NodeId>,
    pub nodes: Vec<Node>,
}

impl Tree {
    pub fn add_node(&mut self, mut node: Node) -> Result<NodeId, NodeIdOverflow> {
        let id = NodeId(u32::try_from(self.nodes.len()).map_err(|_| NodeIdOverflow)?);
        node.id = id;
        self.nodes.push(node);
        Ok(id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(usize::try_from(id.0).ok()?)
    }

    pub fn query_all(&self, selector: &Selector) -> Vec<&Node> {
        self.nodes
            .iter()
            .filter(|node| selector.matches(node))
            .collect()
    }

    pub fn query(&self, selector: &Selector) -> Vec<&Node> {
        let matches = self.query_all(selector);
        match selector.match_index {
            Some(index) => matches.get(index).into_iter().copied().collect(),
            None => matches,
        }
    }

    pub fn find_one(&self, selector: &Selector) -> Result<&Node, SelectionError> {
        let matches = self.query_all(selector);
        if let Some(index) = selector.match_index {
            return matches
                .get(index)
                .copied()
                .ok_or(SelectionError::MatchIndexOutOfRange {
                    index,
                    matches: matches.len(),
                });
        }
        match matches.as_slice() {
            [node] => Ok(node),
            [] => Err(SelectionError::NotFound),
            nodes => Err(SelectionError::Ambiguous {
                matches: nodes.len(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeIdOverflow;

impl fmt::Display for NodeIdOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("node count exceeds u32::MAX")
    }
}

impl Error for NodeIdOverflow {}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Selector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meta: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_index: Option<usize>,
}

impl Selector {
    pub fn matches(&self, node: &Node) -> bool {
        self.kind.as_ref().is_none_or(|kind| kind == &node.kind)
            && self
                .name
                .as_ref()
                .is_none_or(|name| node.name.as_ref() == Some(name))
            && self
                .text
                .as_ref()
                .is_none_or(|text| node.value.as_ref().and_then(Value::as_str) == Some(text))
            && self
                .meta
                .iter()
                .all(|(key, value)| node.meta.get(key) == Some(value))
    }
}

pub type Match = Selector;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionError {
    NotFound,
    Ambiguous { matches: usize },
    MatchIndexOutOfRange { index: usize, matches: usize },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("selection not found"),
            Self::Ambiguous { matches } => write!(f, "selection is ambiguous: {matches} matches"),
            Self::MatchIndexOutOfRange { index, matches } => {
                write!(
                    f,
                    "match index {index} is out of range for {matches} matches"
                )
            }
        }
    }
}

impl Error for SelectionError {}

pub mod diagnostic;
pub use diagnostic::{BinasError, Diagnostic, ErrorCode, Severity};

pub mod legacy;
// Source-compatible transport aliases; new contracts live in transaction/verification.
pub use legacy::{EditPlan, Invariant, InvariantCheck, PlanId, Report, Verification};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(value: &str, page: u64) -> Node {
        Node {
            id: NodeId(0),
            kind: "text".into(),
            name: None,
            span: Span::new(0, value.len() as u64).unwrap(),
            value: Some(json!(value)),
            children: vec![],
            meta: BTreeMap::from([("page".into(), json!(page))]),
        }
    }

    #[test]
    fn spans_are_checked_before_use() {
        assert!(Span::new(4, 3).is_err());
        assert!(Span::from_start_len(u64::MAX, 1).is_err());
        let span = Span::from_start_len(1, 2).unwrap();
        assert_eq!(span.slice(b"abcd"), Some(&b"bc"[..]));
        assert_eq!(
            serde_json::from_str::<Span>(r#"{"start":3,"end":2}"#)
                .unwrap_err()
                .to_string(),
            "invalid span 3..2"
        );
    }

    #[test]
    fn selector_matching_and_match_index_are_deterministic() {
        let mut tree = Tree::default();
        tree.add_node(node("same", 1)).unwrap();
        tree.add_node(node("same", 2)).unwrap();

        let selector = Selector {
            kind: Some("text".into()),
            text: Some("same".into()),
            meta: BTreeMap::from([("page".into(), json!(2))]),
            ..Selector::default()
        };
        assert_eq!(tree.find_one(&selector).unwrap().id, NodeId(1));

        let indexed = Selector {
            text: Some("same".into()),
            match_index: Some(1),
            ..Selector::default()
        };
        assert_eq!(tree.find_one(&indexed).unwrap().id, NodeId(1));
        assert_eq!(
            tree.find_one(&Selector {
                match_index: Some(2),
                ..indexed
            })
            .unwrap_err(),
            SelectionError::MatchIndexOutOfRange {
                index: 2,
                matches: 2
            }
        );
    }

    #[test]
    fn report_and_verification_have_stable_json() {
        let report = Report {
            operation: "replace_text".into(),
            diagnostics: vec![],
            metadata: BTreeMap::new(),
            verification: Some(Verification {
                passed: true,
                checks: vec![InvariantCheck {
                    invariant: Invariant::Reparse,
                    passed: true,
                    diagnostic: None,
                }],
            }),
        };
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({
                "operation": "replace_text",
                "verification": {
                    "passed": true,
                    "checks": [{"invariant": "reparse", "passed": true}]
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<Report>(serde_json::to_value(report).unwrap())
                .unwrap()
                .operation,
            "replace_text"
        );
    }
}
