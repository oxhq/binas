//! Typed PDF object values, independent of lexer and xref parsing.
//!
//! Values inside an object remain owned; ObjectStore shares unchanged objects
//! and clones only a mutated object. Encoded streams share owned allocations or
//! checked source ranges. Operation-level transaction checks are separate work.

use crate::payload::StreamPayload;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ObjectRef {
    pub number: u32,
    pub generation: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<Value>),
    Dict(BTreeMap<Vec<u8>, Value>),
    Ref(ObjectRef),
}

#[derive(Clone, Debug)]
pub(crate) struct IndirectObject {
    pub value: Value,
    pub stream: Option<StreamPayload>,
    pub stream_offset: usize,
    pub offset: usize,
}
