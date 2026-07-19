use std::{error::Error, fmt};

use binas_core::Span;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfErrorCode {
    InvalidSyntax,
    UnsupportedFeature,
    ResourceLimit,
    SelectionNotFound,
    UnsafeRewrite,
    VerificationFailed,
    Internal,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfError {
    pub code: PdfErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<Span>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<(u32, u16)>,
}

impl PdfError {
    pub(crate) fn syntax(message: impl Into<String>, offset: usize) -> Self {
        Self {
            code: PdfErrorCode::InvalidSyntax,
            message: message.into(),
            span: Span::from_start_len(offset as u64, 0).ok(),
            object: None,
        }
    }

    pub(crate) fn limit(message: impl Into<String>) -> Self {
        Self {
            code: PdfErrorCode::ResourceLimit,
            message: message.into(),
            span: None,
            object: None,
        }
    }

    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: PdfErrorCode::UnsupportedFeature,
            message: message.into(),
            span: None,
            object: None,
        }
    }

    pub(crate) fn unsafe_rewrite(message: impl Into<String>) -> Self {
        Self {
            code: PdfErrorCode::UnsafeRewrite,
            message: message.into(),
            span: None,
            object: None,
        }
    }

    pub(crate) fn selection(message: impl Into<String>) -> Self {
        Self {
            code: PdfErrorCode::SelectionNotFound,
            message: message.into(),
            span: None,
            object: None,
        }
    }

    pub(crate) fn verification(message: impl Into<String>) -> Self {
        Self {
            code: PdfErrorCode::VerificationFailed,
            message: message.into(),
            span: None,
            object: None,
        }
    }
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl PdfErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidSyntax => "invalid_syntax",
            Self::UnsupportedFeature => "unsupported_feature",
            Self::ResourceLimit => "resource_limit",
            Self::SelectionNotFound => "selection_not_found",
            Self::UnsafeRewrite => "unsafe_rewrite",
            Self::VerificationFailed => "verification_failed",
            Self::Internal => "internal",
        }
    }
}

impl Error for PdfError {}
