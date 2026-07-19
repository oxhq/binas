use std::{collections::BTreeMap, error::Error, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
struct RawSpan {
    start: u64,
    end: u64,
}

impl Span {
    pub fn new(start: u64, end: u64) -> Result<Self, SpanError> {
        if end < start {
            return Err(SpanError::Reversed { start, end });
        }
        Ok(Self { start, end })
    }

    pub fn from_start_len(start: u64, len: u64) -> Result<Self, SpanError> {
        let end = start
            .checked_add(len)
            .ok_or(SpanError::Overflow { start, len })?;
        Self::new(start, end)
    }

    pub const fn start(self) -> u64 {
        self.start
    }

    pub const fn end(self) -> u64 {
        self.end
    }

    pub const fn len(self) -> u64 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn slice(self, input: &[u8]) -> Option<&[u8]> {
        let start = usize::try_from(self.start).ok()?;
        let end = usize::try_from(self.end).ok()?;
        input.get(start..end)
    }
}

impl TryFrom<RawSpan> for Span {
    type Error = SpanError;

    fn try_from(value: RawSpan) -> Result<Self, Self::Error> {
        Self::new(value.start, value.end)
    }
}

impl From<Span> for RawSpan {
    fn from(value: Span) -> Self {
        Self {
            start: value.start,
            end: value.end,
        }
    }
}

impl<'de> Deserialize<'de> for Span {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        RawSpan::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

impl Serialize for Span {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        RawSpan::from(*self).serialize(serializer)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpanError {
    Reversed { start: u64, end: u64 },
    Overflow { start: u64, len: u64 },
}

impl fmt::Display for SpanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reversed { start, end } => write!(f, "invalid span {start}..{end}"),
            Self::Overflow { start, len } => {
                write!(f, "span start {start} plus length {len} overflows u64")
            }
        }
    }
}

impl Error for SpanError {}

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidSyntax,
    UnsupportedFeature,
    EncryptedDocument,
    InvalidPassword,
    PermissionDenied,
    ResourceLimit,
    SelectionNotFound,
    SelectionAmbiguous,
    UnsafeRewrite,
    SignaturePolicy,
    VerificationFailed,
    ExternalSigner,
    Io,
    Internal,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BinasError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for BinasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl Error for BinasError {}

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
