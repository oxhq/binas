use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

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
