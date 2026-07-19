use binas_core::Span;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError, SurgicalTextEditRequest, edit::encode_same_span,
    writer::refuse_security_boundaries,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BatchTextEditRequest {
    pub edits: Vec<SurgicalTextEditRequest>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlannedTextEdit {
    pub old_text: String,
    pub replacement: String,
    pub match_index: usize,
    pub span: Span,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BatchTextEditPlan {
    pub edits: Vec<PlannedTextEdit>,
    pub source_bytes: usize,
    pub source_sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BatchTextEditReport {
    pub operation: String,
    pub mode: String,
    pub edit_count: usize,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct BatchTextEditVerification {
    pub passed: bool,
    pub reparse_ok: bool,
    pub page_count_unchanged: bool,
    pub replacements_selectable: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchTextEditOutcome {
    pub bytes: Vec<u8>,
    pub report: BatchTextEditReport,
    pub verification: BatchTextEditVerification,
}

impl PdfDocument {
    pub fn plan_batch_text_edits(
        &self,
        request: BatchTextEditRequest,
    ) -> Result<BatchTextEditPlan, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        if request.edits.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "batch text edit requires at least one edit",
            ));
        }
        if request.edits.len() > self.engine_config().limits.max_container_items {
            return Err(PdfError::limit(
                "batch text edit count exceeds max_container_items",
            ));
        }

        let mut edits = Vec::with_capacity(request.edits.len());
        for request in request.edits {
            if request.old_text.is_empty() || request.replacement.is_empty() {
                return Err(PdfError::unsafe_rewrite(
                    "batch text edits require non-empty old and replacement text",
                ));
            }
            let selected = self.query_text(&request.old_text, request.match_index)?;
            if selected.font_name.is_some() || selected.to_unicode {
                return Err(PdfError::unsafe_rewrite(
                    "font-decoded text requires font_text_edit",
                ));
            }
            let span = selected.source_span.ok_or_else(|| {
                PdfError::unsafe_rewrite("batch text edit requires direct source provenance")
            })?;
            let source = span.slice(self.source()).ok_or_else(|| {
                PdfError::unsafe_rewrite("batch text edit span is outside the source bytes")
            })?;
            encode_same_span(&request.replacement, source)?;
            edits.push(PlannedTextEdit {
                old_text: request.old_text,
                replacement: request.replacement,
                match_index: request.match_index,
                span,
            });
        }
        edits.sort_by_key(|edit| edit.span.start());
        if edits
            .windows(2)
            .any(|pair| pair[0].span.end() > pair[1].span.start())
        {
            return Err(PdfError::unsafe_rewrite("batch text edit spans overlap"));
        }

        Ok(BatchTextEditPlan {
            edits,
            source_bytes: self.source_len(),
            source_sha256: hex(&Sha256::digest(self.source())),
        })
    }

    pub fn apply_batch_text_edits(
        &self,
        plan: BatchTextEditPlan,
    ) -> Result<BatchTextEditOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        if plan.source_bytes != self.source_len()
            || plan.source_sha256 != hex(&Sha256::digest(self.source()))
            || plan.edits.is_empty()
            || plan.edits.len() > self.engine_config().limits.max_container_items
        {
            return Err(PdfError::unsafe_rewrite(
                "batch text edit plan does not match this document",
            ));
        }
        let mut spans = plan.edits.iter().map(|edit| edit.span).collect::<Vec<_>>();
        spans.sort_by_key(|span| span.start());
        if spans.windows(2).any(|pair| pair[0].end() > pair[1].start()) {
            return Err(PdfError::unsafe_rewrite("batch text edit spans overlap"));
        }

        let mut output = self.source().to_vec();
        for edit in &plan.edits {
            let selected = self.query_text(&edit.old_text, edit.match_index)?;
            if selected.source_span != Some(edit.span) {
                return Err(PdfError::unsafe_rewrite(
                    "batch text edit plan does not match this document",
                ));
            }
            let source = edit.span.slice(self.source()).ok_or_else(|| {
                PdfError::unsafe_rewrite("batch text edit span is outside the source bytes")
            })?;
            let encoded = encode_same_span(&edit.replacement, source)?;
            let start = usize::try_from(edit.span.start()).map_err(|_| {
                PdfError::unsafe_rewrite("batch text edit offset does not fit usize")
            })?;
            let end = usize::try_from(edit.span.end()).map_err(|_| {
                PdfError::unsafe_rewrite("batch text edit offset does not fit usize")
            })?;
            output[start..end].copy_from_slice(&encoded);
        }

        let old_pages = self.page_count()?;
        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&output, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("batch text output did not reparse: {error}"))
            })?;
        let page_count_unchanged = rewritten.page_count().map_err(|error| {
            PdfError::verification(format!("batch text page verification failed: {error}"))
        })? == old_pages;
        let replacements_selectable = plan.edits.iter().try_fold(true, |passed, edit| {
            let found = rewritten
                .query_text_all(&edit.replacement)
                .map_err(|error| {
                    PdfError::verification(format!("batch text verification failed: {error}"))
                })?;
            Ok::<_, PdfError>(
                passed && found.iter().any(|item| item.source_span == Some(edit.span)),
            )
        })?;
        let verification = BatchTextEditVerification {
            passed: page_count_unchanged && replacements_selectable,
            reparse_ok: true,
            page_count_unchanged,
            replacements_selectable,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "batch text edit failed post-write verification",
            ));
        }

        Ok(BatchTextEditOutcome {
            bytes: output,
            report: BatchTextEditReport {
                operation: "batch_replace_text".into(),
                mode: "surgical".into(),
                edit_count: plan.edits.len(),
                spans: plan.edits.into_iter().map(|edit| edit.span).collect(),
            },
            verification,
        })
    }

    pub fn batch_text_edit(
        &self,
        request: BatchTextEditRequest,
    ) -> Result<BatchTextEditOutcome, PdfError> {
        let plan = self.plan_batch_text_edits(request)?;
        self.apply_batch_text_edits(plan)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
