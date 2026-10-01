//! Content and revision identities, independent of any format or session.

use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};

/// SHA-256 identity of the complete, original input bytes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SourceId([u8; 32]);

impl SourceId {
    /// Constructs an identity from an already computed SHA-256 digest.
    pub const fn from_sha256(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub const fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RevisionId(u64);

impl RevisionId {
    pub const INITIAL: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> u64 {
        self.0
    }

    /// Exhaustion fails instead of reusing an earlier revision identity.
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Source and revision binding. This is not a document-session lineage ID.
/// Independent sessions editing the same source must supply their own lineage.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SnapshotId {
    source: SourceId,
    revision: RevisionId,
}

impl SnapshotId {
    pub const fn new(source: SourceId, revision: RevisionId) -> Self {
        Self { source, revision }
    }

    pub const fn source(self) -> SourceId {
        self.source
    }

    pub const fn revision(self) -> RevisionId {
        self.revision
    }

    pub fn ensure_current(self, actual: Self) -> Result<(), SnapshotMismatch> {
        if self == actual {
            Ok(())
        } else {
            Err(SnapshotMismatch {
                expected: self,
                actual,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotMismatch {
    pub expected: SnapshotId,
    pub actual: SnapshotId,
}

impl fmt::Display for SnapshotMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("source or revision no longer matches the expected snapshot")
    }
}

impl Error for SnapshotMismatch {}
