//! Explicit adapter capabilities. Support is scoped to the named operation,
//! not a claim that every possible input is eligible for it.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityStatus {
    Supported,
    Unsupported { reason: &'static str },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capability {
    /// Stable adapter-namespaced operation identifier.
    pub id: &'static str,
    pub status: CapabilityStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Detection {
    Recognized,
    Unrecognized,
    /// A bounded prefix is insufficient; no claim of invalidity is made.
    Inconclusive,
}
