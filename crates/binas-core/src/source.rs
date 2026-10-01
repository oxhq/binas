//! Immutable, bounded input storage and honest payload provenance.

use std::{error::Error, fmt, sync::Arc};

use sha2::{Digest, Sha256};

use crate::{Span, identity::SourceId};

#[derive(Clone, Debug)]
enum Storage {
    Owned(Arc<Vec<u8>>),
    Shared(Arc<[u8]>),
}

/// Cloning a source shares its payload and cached content identity.
#[derive(Clone, Debug)]
pub struct Source {
    storage: Storage,
    id: SourceId,
}

impl Source {
    /// Takes ownership without copying or reallocating the vector's payload.
    /// The limit is checked before hashing; the caller already owns the input.
    pub fn from_owned(bytes: Vec<u8>, max_bytes: usize) -> Result<Self, SourceError> {
        check_length(bytes.len(), max_bytes)?;
        let id = hash(&bytes);
        Ok(Self {
            storage: Storage::Owned(Arc::new(bytes)),
            id,
        })
    }

    /// Retains the caller's shared allocation without copying its payload.
    pub fn from_shared(bytes: Arc<[u8]>, max_bytes: usize) -> Result<Self, SourceError> {
        check_length(bytes.len(), max_bytes)?;
        let id = hash(&bytes);
        Ok(Self {
            storage: Storage::Shared(bytes),
            id,
        })
    }

    /// Checks the input limit before allocating, copying or hashing input bytes.
    pub fn copy_from_slice(bytes: &[u8], max_bytes: usize) -> Result<Self, SourceError> {
        check_length(bytes.len(), max_bytes)?;
        let mut owned = Vec::new();
        owned
            .try_reserve_exact(bytes.len())
            .map_err(|_| SourceError::AllocationFailed)?;
        owned.extend_from_slice(bytes);
        Self::from_owned(owned, max_bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        match &self.storage {
            Storage::Owned(bytes) => bytes.as_slice(),
            Storage::Shared(bytes) => bytes,
        }
    }

    pub fn len(&self) -> usize {
        self.as_bytes().len()
    }

    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }

    pub const fn id(&self) -> SourceId {
        self.id
    }

    /// Allocation identity, not content equality; empty sources are unambiguous.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (&self.storage, &other.storage) {
            (Storage::Owned(left), Storage::Owned(right)) => Arc::ptr_eq(left, right),
            (Storage::Shared(left), Storage::Shared(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    pub fn qualified_span(&self, span: Span) -> Result<SourceSpan, SourceError> {
        let qualified = SourceSpan::new(self.id, span);
        qualified.resolve(self)?;
        Ok(qualified)
    }
}

fn check_length(actual: usize, maximum: usize) -> Result<(), SourceError> {
    if actual > maximum {
        Err(SourceError::InputTooLarge { actual, maximum })
    } else {
        Ok(())
    }
}

fn hash(bytes: &[u8]) -> SourceId {
    SourceId::from_sha256(Sha256::digest(bytes).into())
}

/// A range qualified by content identity. Construction alone does not validate
/// a range against bytes; use `Source::qualified_span` or `resolve` to do so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    source: SourceId,
    span: Span,
}

impl SourceSpan {
    pub const fn new(source: SourceId, span: Span) -> Self {
        Self { source, span }
    }

    pub const fn source(self) -> SourceId {
        self.source
    }

    pub const fn span(self) -> Span {
        self.span
    }

    pub fn resolve(self, source: &Source) -> Result<&[u8], SourceError> {
        if source.id != self.source {
            return Err(SourceError::SourceMismatch {
                expected: self.source,
                actual: source.id,
            });
        }
        self.span
            .slice(source.as_bytes())
            .ok_or(SourceError::RangeOutOfBounds {
                span: self.span,
                source_len: source.len(),
            })
    }
}

/// `Derived` names contributing encoded/source ranges, not a byte-exact mapping
/// of decoded output offsets to original input. For example, decompression must
/// use `Derived`, even if the decoded byte count happens to equal input length.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PayloadProvenance {
    Exact(SourceSpan),
    Derived {
        inputs: Vec<SourceSpan>,
        transform: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceError {
    InputTooLarge {
        actual: usize,
        maximum: usize,
    },
    AllocationFailed,
    SourceMismatch {
        expected: SourceId,
        actual: SourceId,
    },
    RangeOutOfBounds {
        span: Span,
        source_len: usize,
    },
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputTooLarge { actual, maximum } => {
                write!(formatter, "input length {actual} exceeds limit {maximum}")
            }
            Self::AllocationFailed => formatter.write_str("input allocation failed"),
            Self::SourceMismatch { .. } => {
                formatter.write_str("range belongs to a different source")
            }
            Self::RangeOutOfBounds { span, source_len } => write!(
                formatter,
                "range {}..{} exceeds source length {source_len}",
                span.start(),
                span.end()
            ),
        }
    }
}

impl Error for SourceError {}
