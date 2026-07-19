use binas_core::Span;
use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError,
    cmap::ToUnicodeCMap,
    edit::{encode_hex, encode_literal},
    filters::encode_pdf_stream,
    fonts::FontDecoder,
    parser::{self, ObjectRef, ParseBudget},
    writer::{append_object_revision, refuse_security_boundaries},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FontTextEditRequest {
    pub old_text: String,
    pub replacement: String,
    #[serde(default)]
    pub match_index: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FontEditReport {
    pub operation: String,
    pub mode: String,
    pub match_index: usize,
    pub decoded_span: Span,
    pub font_name: String,
    pub object_number: u32,
    pub generation: u16,
    pub encoded_glyph_bytes: usize,
    pub original_bytes: usize,
    pub appended_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FontEditVerification {
    pub passed: bool,
    pub prefix_preserved: bool,
    pub page_count_unchanged: bool,
    pub decoded_stream_verified: bool,
    pub replacement_selectable: bool,
    pub revision_incremented: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontEditOutcome {
    pub bytes: Vec<u8>,
    pub report: FontEditReport,
    pub verification: FontEditVerification,
}

impl PdfDocument {
    pub fn font_text_edit(
        &self,
        request: FontTextEditRequest,
    ) -> Result<FontEditOutcome, PdfError> {
        if request.old_text.is_empty() || request.replacement.is_empty() {
            return Err(PdfError::unsafe_rewrite(
                "font text edit requires non-empty old and replacement text",
            ));
        }
        if request.replacement.len() > self.engine_config().limits.max_token_bytes {
            return Err(PdfError::limit("font replacement exceeds max_token_bytes"));
        }
        refuse_security_boundaries(self.parsed())?;

        let selected = self.query_text(&request.old_text, request.match_index)?;
        let font_name = selected
            .font_name
            .as_deref()
            .ok_or_else(|| PdfError::unsupported("selected text has no active font resource"))?;
        if !selected.to_unicode {
            return Err(PdfError::unsupported("selected font has no ToUnicode CMap"));
        }
        let reference = ObjectRef {
            number: selected.object_number,
            generation: selected.generation,
        };
        let (cmap_reference, content_references) =
            self.font_cmap_ref_for_content(reference, font_name.as_bytes())?;
        if content_references != 1 {
            return Err(PdfError::unsafe_rewrite(
                "font content stream must have one unambiguous page reference",
            ));
        }

        let mut budget = ParseBudget::default();
        let cmap_object = self.parsed().object(cmap_reference)?;
        let cmap_stream = cmap_object
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::syntax("font /ToUnicode reference is not a stream", 0))?;
        let cmap_bytes = parser::decode_stream(
            &cmap_object.value,
            cmap_stream,
            &self.parsed().limits,
            &mut budget,
        )?;
        let codec = FontDecoder::new(ToUnicodeCMap::parse(&cmap_bytes, &self.parsed().limits)?);
        let encoded_replacement = codec.encode(&request.replacement, &self.parsed().limits)?;

        let object = self.parsed().object(reference)?;
        if !object_header_matches(self.source(), object.offset, reference) {
            return Err(PdfError::unsafe_rewrite(
                "font content stream lacks direct file provenance",
            ));
        }
        let encoded_stream = object
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::unsafe_rewrite("selected object is not a stream"))?;
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
            [b'(', .., b')'] => encode_literal(&encoded_replacement),
            [b'<', .., b'>'] => encode_hex(&encoded_replacement),
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
        let encoded_stream =
            encode_pdf_stream(&object.value, &replacement_stream, &self.parsed().limits)?;

        let old_pages = self.page_count()?;
        let old_revisions = self.parsed().xref_revisions;
        let original_len = self.source_len();
        let output = append_object_revision(self, reference, &object.value, Some(&encoded_stream))?;
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
        let replacement_selectable =
            rewritten
                .query_text_all(&request.replacement)?
                .iter()
                .any(|found| {
                    found.object_number == reference.number
                        && found.generation == reference.generation
                        && found.font_name.as_deref() == Some(font_name)
                        && found.to_unicode
                });
        let revision_incremented = rewritten.parsed().xref_revisions == old_revisions + 1;
        let verification = FontEditVerification {
            passed: prefix_preserved
                && page_count_unchanged
                && decoded_stream_verified
                && replacement_selectable
                && revision_incremented,
            prefix_preserved,
            page_count_unchanged,
            decoded_stream_verified,
            replacement_selectable,
            revision_incremented,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "font text edit failed post-write verification",
            ));
        }
        Ok(FontEditOutcome {
            report: FontEditReport {
                operation: "replace_text".into(),
                mode: "font_incremental".into(),
                match_index: request.match_index,
                decoded_span: selected.decoded_span,
                font_name: font_name.into(),
                object_number: reference.number,
                generation: reference.generation,
                encoded_glyph_bytes: encoded_replacement.len(),
                original_bytes: original_len,
                appended_bytes: output.len() - original_len,
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

fn reopen_for_verification(document: &PdfDocument, output: &[u8]) -> Result<PdfDocument, PdfError> {
    let mut config = document.engine_config().clone();
    config.limits.max_input_bytes = config.limits.max_input_bytes.max(output.len());
    PdfEngine::new(config)
        .open(output, OpenOptions::default())
        .map_err(|error| {
            PdfError::verification(format!("font edit output did not reparse: {error}"))
        })
}
