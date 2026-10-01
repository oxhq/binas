//! Bounded PDF transformation primitives, without consumer presets or size search.
//! Stream byte savings do not establish a smaller final PDF or conformance.

pub(crate) mod images;
pub(crate) mod structure;

pub use images::{
    PdfLosslessImageEffect, PdfLosslessImagePlan, PdfLosslessImageReceipt, PdfLosslessImageSkip,
    PdfLosslessImageSkipReason,
};

pub use structure::{
    PdfFlateEffect, PdfFlatePlan, PdfFlateReceipt, PdfFlateSkip, PdfFlateSkipReason,
    PdfFlateStreamRole,
};
