//! PDF-owned target metadata and coverage-aware reporting. Registry entries are
//! planned scope, never a claim of implemented conversion or validation.
mod registry;
mod report;
pub use registry::*;
pub use report::*;
