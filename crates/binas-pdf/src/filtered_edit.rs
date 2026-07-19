use binas_core::Span;
use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError,
    edit::{encode_hex, encode_literal},
    filters::encode_pdf_stream,
    parser::{self, ObjectRef, ParseBudget, Value},
    writer::{append_object_revision, dict_get, refuse_security_boundaries},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FilteredTextEditRequest {
    pub old_text: String,
    pub replacement: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FilteredEditReport {
    pub operation: String,
    pub mode: String,
    pub match_index: usize,
    pub decoded_span: Span,
    pub object_number: u32,
    pub generation: u16,
    pub original_bytes: usize,
    pub appended_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FilteredEditVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub decoded_stream_verified: bool,
    pub filter_metadata_preserved: bool,
    pub replacement_selectable: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilteredEditOutcome {
    pub bytes: Vec<u8>,
    pub report: FilteredEditReport,
    pub verification: FilteredEditVerification,
}

impl PdfDocument {
    pub fn filtered_text_edit(
        &self,
        request: FilteredTextEditRequest,
    ) -> Result<FilteredEditOutcome, PdfError> {
        if request.old_text.is_empty() || request.replacement.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "filtered text edit requires non-empty old and replacement text",
            ));
        }
        if !request.replacement.is_ascii() {
            return Err(PdfError::unsafe_rewrite(
                "filtered text edit only supports ASCII replacement text",
            ));
        }
        if request.replacement.len() > self.engine_config().limits.max_token_bytes {
            return Err(PdfError::limit(
                "filtered replacement exceeds max_token_bytes",
            ));
        }
        refuse_security_boundaries(self.parsed())?;

        let selected = self.query_text(&request.old_text, request.match_index)?;
        if selected.source_span.is_some() {
            return Err(PdfError::unsafe_rewrite(
                "filtered text edit requires a filtered content stream",
            ));
        }
        if selected.font_name.is_some() || selected.to_unicode {
            return Err(PdfError::unsupported(
                "filtered text editing with font decoding is not implemented",
            ));
        }
        let reference = ObjectRef {
            number: selected.object_number,
            generation: selected.generation,
        };
        if content_reference_count(self, reference)? != 1 {
            return Err(PdfError::unsafe_rewrite(
                "filtered content stream must have one unambiguous page reference",
            ));
        }
        let object = self.parsed().object(reference)?;
        let Value::Dict(_) = &object.value else {
            return Err(PdfError::unsafe_rewrite("content stream has no dictionary"));
        };
        if !object_header_matches(self.source(), object.offset, reference) {
            return Err(PdfError::unsafe_rewrite(
                "filtered content stream lacks direct file provenance",
            ));
        }
        let encoded_stream = object
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::unsafe_rewrite("selected object is not a stream"))?;
        let mut budget = ParseBudget::default();
        let decoded = parser::decode_stream(
            &object.value,
            encoded_stream,
            &self.parsed().limits,
            &mut budget,
        )?;
        let token = selected.decoded_span.slice(&decoded).ok_or_else(|| {
            PdfError::unsafe_rewrite("selected decoded span is outside the content stream")
        })?;
        let replacement_token = match token {
            [b'(', .., b')'] => encode_literal(request.replacement.as_bytes()),
            [b'<', .., b'>'] => encode_hex(request.replacement.as_bytes()),
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "selected text is not a literal or hex text token",
                ));
            }
        };
        let start = usize::try_from(selected.decoded_span.start())
            .map_err(|_| PdfError::limit("decoded text span does not fit usize"))?;
        let end = usize::try_from(selected.decoded_span.end())
            .map_err(|_| PdfError::limit("decoded text span does not fit usize"))?;
        let replacement_len = decoded
            .len()
            .checked_sub(token.len())
            .and_then(|length| length.checked_add(replacement_token.len()))
            .ok_or_else(|| PdfError::limit("replacement stream length overflows"))?;
        if replacement_len > self.engine_config().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "replacement stream exceeds max_stream_bytes",
            ));
        }
        let mut replacement_stream = Vec::with_capacity(replacement_len);
        replacement_stream.extend_from_slice(&decoded[..start]);
        replacement_stream.extend_from_slice(&replacement_token);
        replacement_stream.extend_from_slice(&decoded[end..]);
        let compressed = encode_pdf_stream(
            &object.value,
            &replacement_stream,
            &self.engine_config().limits,
        )?;

        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let original_len = self.source_len();
        let output = append_object_revision(self, reference, &object.value, Some(&compressed))?;
        let rewritten = reopen_for_verification(self, &output)?;
        let rewritten_object = rewritten.parsed().object(reference)?;
        let mut verify_budget = ParseBudget::default();
        let verified_stream = parser::decode_stream(
            &rewritten_object.value,
            rewritten_object.stream.as_deref().ok_or_else(|| {
                PdfError::verification("rewritten content object is not a stream")
            })?,
            &rewritten.parsed().limits,
            &mut verify_budget,
        )?;
        let prefix_preserved = output.starts_with(self.source());
        let page_count_unchanged = rewritten.page_count()? == old_pages;
        let decoded_stream_verified = verified_stream == replacement_stream;
        let filter_metadata_preserved = [b"Filter".as_slice(), b"DecodeParms".as_slice()]
            .iter()
            .all(|key| dict_get(&object.value, key) == dict_get(&rewritten_object.value, key));
        let replacement_selectable =
            rewritten
                .query_text_all(&request.replacement)?
                .iter()
                .any(|found| {
                    found.object_number == reference.number
                        && found.generation == reference.generation
                        && found.source_span.is_none()
                });
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = FilteredEditVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && decoded_stream_verified
                && filter_metadata_preserved
                && replacement_selectable
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            decoded_stream_verified,
            filter_metadata_preserved,
            replacement_selectable,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "filtered text edit failed post-write verification",
            ));
        }
        Ok(FilteredEditOutcome {
            report: FilteredEditReport {
                operation: "replace_text".into(),
                mode: "filtered_incremental".into(),
                match_index: request.match_index,
                decoded_span: selected.decoded_span,
                object_number: reference.number,
                generation: reference.generation,
                original_bytes: original_len,
                appended_bytes: output.len() - original_len,
            },
            bytes: output,
            verification,
        })
    }
}

fn content_reference_count(document: &PdfDocument, target: ObjectRef) -> Result<usize, PdfError> {
    let mut count = 0usize;
    for page_ref in document.page_refs()? {
        let page = document.parsed().object(page_ref)?;
        match dict_get(&page.value, b"Contents") {
            Some(Value::Ref(reference)) if *reference == target => count += 1,
            Some(Value::Array(values)) => {
                count += values
                    .iter()
                    .filter(|value| matches!(value, Value::Ref(reference) if *reference == target))
                    .count();
            }
            _ => {}
        }
    }
    Ok(count)
}

fn object_header_matches(input: &[u8], offset: usize, reference: ObjectRef) -> bool {
    let expected = format!("{} {} obj", reference.number, reference.generation);
    input
        .get(offset..)
        .is_some_and(|source| source.starts_with(expected.as_bytes()))
}

fn reopen_for_verification(document: &PdfDocument, output: &[u8]) -> Result<PdfDocument, PdfError> {
    let mut config = document.engine_config().clone();
    config.limits.max_input_bytes = config.limits.max_input_bytes.max(output.len());
    PdfEngine::new(config)
        .open(output, OpenOptions::default())
        .map_err(|error| {
            PdfError::verification(format!("filtered edit output did not reparse: {error}"))
        })
}
