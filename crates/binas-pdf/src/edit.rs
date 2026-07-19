use binas_core::Span;
use serde::{Deserialize, Serialize};

use crate::{OpenOptions, PdfDocument, PdfEngine, PdfError};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SurgicalTextEditRequest {
    pub old_text: String,
    pub replacement: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SurgicalEditReport {
    pub operation: String,
    pub mode: String,
    pub match_index: usize,
    pub span: Span,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SurgicalEditVerification {
    pub passed: bool,
    pub reparse_ok: bool,
    pub page_count_unchanged: bool,
    pub replacement_selectable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SurgicalEditOutcome {
    pub bytes: Vec<u8>,
    pub report: SurgicalEditReport,
    pub verification: SurgicalEditVerification,
}

impl PdfDocument {
    pub fn surgical_text_edit(
        &self,
        request: SurgicalTextEditRequest,
    ) -> Result<SurgicalEditOutcome, PdfError> {
        if request.old_text.is_empty() || request.replacement.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "surgical text edit requires non-empty old and replacement text",
            ));
        }
        let selected = self.query_text(&request.old_text, request.match_index)?;
        if selected.font_name.is_some() || selected.to_unicode {
            return Err(PdfError::unsafe_rewrite(
                "font-decoded text requires font_text_edit",
            ));
        }
        let source_span = selected.source_span.ok_or_else(|| {
            PdfError::unsafe_rewrite("surgical edit requires direct source provenance")
        })?;
        let source = source_span.slice(self.source()).ok_or_else(|| {
            PdfError::unsafe_rewrite("selected text span is outside the source bytes")
        })?;
        let encoded = encode_same_span(&request.replacement, source)?;
        let start = usize::try_from(source_span.start())
            .map_err(|_| PdfError::unsafe_rewrite("selected text offset does not fit usize"))?;
        let end = usize::try_from(source_span.end())
            .map_err(|_| PdfError::unsafe_rewrite("selected text offset does not fit usize"))?;

        let mut output = self.source().to_vec();
        output
            .get_mut(start..end)
            .ok_or_else(|| PdfError::unsafe_rewrite("selected text span is outside the output"))?
            .copy_from_slice(&encoded);

        let old_pages = self.page_count()?;
        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&output, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("rewritten PDF did not reparse: {error}"))
            })?;
        let page_count_unchanged = rewritten.page_count().map_err(|error| {
            PdfError::verification(format!("rewritten PDF page verification failed: {error}"))
        })? == old_pages;
        let replacement_selectable = rewritten
            .query_text_all(&request.replacement)
            .map_err(|error| {
                PdfError::verification(format!("rewritten PDF text verification failed: {error}"))
            })?
            .iter()
            .any(|found| found.source_span == Some(source_span));
        let verification = SurgicalEditVerification {
            passed: page_count_unchanged && replacement_selectable,
            reparse_ok: true,
            page_count_unchanged,
            replacement_selectable,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "surgical text edit failed post-write verification",
            ));
        }
        Ok(SurgicalEditOutcome {
            bytes: output,
            report: SurgicalEditReport {
                operation: "replace_text".into(),
                mode: "surgical".into(),
                match_index: request.match_index,
                span: source_span,
            },
            verification,
        })
    }
}

pub(crate) fn encode_same_span(replacement: &str, source: &[u8]) -> Result<Vec<u8>, PdfError> {
    if !replacement.is_ascii() {
        return Err(PdfError::unsafe_rewrite(
            "surgical text edit only supports ASCII replacement text",
        ));
    }
    let encoded = match source {
        [b'(', .., b')'] => encode_literal(replacement.as_bytes()),
        [b'<', .., b'>'] => encode_hex(replacement.as_bytes()),
        _ => {
            return Err(PdfError::unsafe_rewrite(
                "selected text is not a literal or hex Tj token",
            ));
        }
    };
    if encoded.len() != source.len() {
        return Err(PdfError::unsafe_rewrite(format!(
            "replacement token is {} bytes but selected token is {} bytes",
            encoded.len(),
            source.len()
        )));
    }
    Ok(encoded)
}

pub(crate) fn encode_literal(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() + 2);
    out.push(b'(');
    for byte in value {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(*byte);
            }
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            8 => out.extend_from_slice(b"\\b"),
            12 => out.extend_from_slice(b"\\f"),
            _ => out.push(*byte),
        }
    }
    out.push(b')');
    out
}

pub(crate) fn encode_hex(value: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Vec::with_capacity(value.len() * 2 + 2);
    out.push(b'<');
    for byte in value {
        out.push(HEX[usize::from(byte >> 4)]);
        out.push(HEX[usize::from(byte & 0x0f)]);
    }
    out.push(b'>');
    out
}
