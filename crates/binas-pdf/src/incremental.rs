use binas_core::Span;
use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError,
    edit::{encode_hex, encode_literal},
    parser::{ObjectRef, Value},
    writer::{append_object_revision, refuse_security_boundaries},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IncrementalTextEditRequest {
    pub old_text: String,
    pub replacement: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IncrementalEditReport {
    pub operation: String,
    pub mode: String,
    pub match_index: usize,
    pub original_span: Span,
    pub object_number: u32,
    pub generation: u16,
    pub original_bytes: usize,
    pub appended_bytes: usize,
    pub xref_offset: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IncrementalEditVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub replacement_selectable: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IncrementalEditOutcome {
    pub bytes: Vec<u8>,
    pub report: IncrementalEditReport,
    pub verification: IncrementalEditVerification,
}

impl PdfDocument {
    pub fn incremental_text_edit(
        &self,
        request: IncrementalTextEditRequest,
    ) -> Result<IncrementalEditOutcome, PdfError> {
        if request.old_text.is_empty() || request.replacement.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "incremental text edit requires non-empty old and replacement text",
            ));
        }
        if !request.replacement.is_ascii() {
            return Err(PdfError::unsafe_rewrite(
                "incremental text edit only supports ASCII replacement text",
            ));
        }
        if request.replacement.len() > self.engine_config().limits.max_token_bytes {
            return Err(PdfError::limit(
                "incremental replacement exceeds max_token_bytes",
            ));
        }
        refuse_security_boundaries(self.parsed())?;

        let selected = self.query_text(&request.old_text, request.match_index)?;
        if selected.font_name.is_some() || selected.to_unicode {
            return Err(PdfError::unsafe_rewrite(
                "font-decoded text requires font_text_edit",
            ));
        }
        let reference = ObjectRef {
            number: selected.object_number,
            generation: selected.generation,
        };
        let object = self.parsed().object(reference)?;
        let dictionary = match &object.value {
            Value::Dict(dictionary) => dictionary,
            _ => return Err(PdfError::unsafe_rewrite("content stream has no dictionary")),
        };
        if dictionary.len() != 1 || !dictionary.contains_key(b"Length".as_slice()) {
            return Err(PdfError::unsafe_rewrite(
                "incremental content stream dictionary contains unsupported entries",
            ));
        }
        let Value::Integer(declared_length) = &dictionary[b"Length".as_slice()] else {
            return Err(PdfError::unsafe_rewrite(
                "incremental edit requires a direct /Length integer",
            ));
        };
        let stream = object.stream.as_deref().ok_or_else(|| {
            PdfError::unsafe_rewrite("selected object is not a file content stream")
        })?;
        if usize::try_from(*declared_length).ok() != Some(stream.len()) {
            return Err(PdfError::unsafe_rewrite(
                "content stream /Length does not match its bytes",
            ));
        }
        if !object_header_matches(self.source(), object.offset, reference) {
            return Err(PdfError::unsafe_rewrite(
                "content stream is compressed or lacks direct file provenance",
            ));
        }

        let source_span = selected.source_span.ok_or_else(|| {
            PdfError::unsafe_rewrite("incremental edit requires direct source provenance")
        })?;
        let token = source_span.slice(self.source()).ok_or_else(|| {
            PdfError::unsafe_rewrite("selected text span is outside source bytes")
        })?;
        let encoded = match token {
            [b'(', .., b')'] => encode_literal(request.replacement.as_bytes()),
            [b'<', .., b'>'] => encode_hex(request.replacement.as_bytes()),
            _ => {
                return Err(PdfError::unsafe_rewrite(
                    "selected text is not a literal or hex Tj token",
                ));
            }
        };
        let relative_start = usize::try_from(
            source_span
                .start()
                .checked_sub(object.stream_offset as u64)
                .ok_or_else(|| PdfError::unsafe_rewrite("text span precedes its stream"))?,
        )
        .map_err(|_| PdfError::unsafe_rewrite("text span does not fit usize"))?;
        let relative_end = usize::try_from(
            source_span
                .end()
                .checked_sub(object.stream_offset as u64)
                .ok_or_else(|| PdfError::unsafe_rewrite("text span precedes its stream"))?,
        )
        .map_err(|_| PdfError::unsafe_rewrite("text span does not fit usize"))?;
        if stream.get(relative_start..relative_end) != Some(token) {
            return Err(PdfError::unsafe_rewrite(
                "text span does not resolve inside its content stream",
            ));
        }
        let replacement_stream_len = stream
            .len()
            .checked_sub(token.len())
            .and_then(|length| length.checked_add(encoded.len()))
            .ok_or_else(|| PdfError::limit("replacement stream length overflows"))?;
        if replacement_stream_len > self.engine_config().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "replacement stream exceeds max_stream_bytes",
            ));
        }
        let mut replacement_stream = Vec::with_capacity(replacement_stream_len);
        replacement_stream.extend_from_slice(&stream[..relative_start]);
        replacement_stream.extend_from_slice(&encoded);
        replacement_stream.extend_from_slice(&stream[relative_end..]);

        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let original_len = self.source().len();
        let output =
            append_object_revision(self, reference, &object.value, Some(&replacement_stream))?;
        let xref_offset = last_startxref_value(&output)?;

        let mut verification_config = self.engine_config().clone();
        verification_config.limits.max_input_bytes =
            verification_config.limits.max_input_bytes.max(output.len());
        let rewritten = PdfEngine::new(verification_config)
            .open(&output, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("incremental output did not reparse: {error}"))
            })?;
        let prefix_preserved = output.starts_with(self.source());
        let page_count_unchanged = rewritten.page_count().map_err(|error| {
            PdfError::verification(format!("incremental page verification failed: {error}"))
        })? == old_pages;
        let replacement_selectable = rewritten
            .query_text_all(&request.replacement)
            .map_err(|error| {
                PdfError::verification(format!("incremental text verification failed: {error}"))
            })?
            .iter()
            .any(|found| {
                found.object_number == reference.number
                    && found.generation == reference.generation
                    && found.source_span.and_then(|span| span.slice(&output))
                        == Some(encoded.as_slice())
            });
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = IncrementalEditVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && replacement_selectable
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            replacement_selectable,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "incremental text edit failed post-write verification",
            ));
        }
        Ok(IncrementalEditOutcome {
            report: IncrementalEditReport {
                operation: "replace_text".into(),
                mode: "incremental".into(),
                match_index: request.match_index,
                original_span: source_span,
                object_number: reference.number,
                generation: reference.generation,
                original_bytes: original_len,
                appended_bytes: output.len() - original_len,
                xref_offset,
            },
            bytes: output,
            verification,
        })
    }
}

fn object_header_matches(input: &[u8], offset: usize, reference: ObjectRef) -> bool {
    let expected = format!("{} {} obj", reference.number, reference.generation);
    input
        .get(offset..)
        .is_some_and(|source| source.starts_with(expected.as_bytes()))
}

fn last_startxref_value(input: &[u8]) -> Result<usize, PdfError> {
    let marker = b"startxref";
    let start = input
        .windows(marker.len())
        .rposition(|window| window == marker)
        .ok_or_else(|| PdfError::unsafe_rewrite("missing startxref"))?;
    let mut rest = &input[start + marker.len()..];
    while rest.first().is_some_and(u8::is_ascii_whitespace) {
        rest = &rest[1..];
    }
    let digits = rest
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits == 0 {
        return Err(PdfError::unsafe_rewrite("startxref has no numeric offset"));
    }
    let value = std::str::from_utf8(&rest[..digits])
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value < input.len())
        .ok_or_else(|| PdfError::unsafe_rewrite("startxref offset is invalid"))?;
    Ok(value)
}
