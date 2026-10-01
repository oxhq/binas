use serde::{Deserialize, Serialize};

use crate::{
    content,
    document::{PdfDocument, PdfEngine},
    error::PdfError,
    limits::OpenOptions,
    parser::{ParseBudget, Value},
    writer::{dict_get, refuse_security_boundaries, write_compact_document},
};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CanonicalizeReport {
    pub operation: String,
    pub mode: String,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub object_count: usize,
    pub xref_offset: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CanonicalizeVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub page_count_unchanged: bool,
    pub text_queries_available: bool,
    pub text_query_semantics_unchanged: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CanonicalizeOutcome {
    pub bytes: Vec<u8>,
    pub report: CanonicalizeReport,
    pub verification: CanonicalizeVerification,
}

impl PdfDocument {
    pub fn canonicalize(&self) -> Result<CanonicalizeOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        let old_pages = self.page_count()?;
        let old_text = text_query_semantics(self)?;
        let (bytes, xref_offset) = write_compact_document(self, None)?;

        let rewritten = PdfEngine::new(self.engine_config().clone())
            .open(&bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("canonical output did not reparse: {error}"))
            })?;
        let page_count_unchanged = rewritten.page_count().map_err(|error| {
            PdfError::verification(format!("canonical page verification failed: {error}"))
        })? == old_pages;
        let new_text = text_query_semantics(&rewritten).map_err(|error| {
            PdfError::verification(format!("canonical text verification failed: {error}"))
        })?;
        let text_queries_available = old_text.is_some();
        let text_query_semantics_unchanged = old_text == new_text;
        let verification = CanonicalizeVerification {
            passed: page_count_unchanged && text_query_semantics_unchanged,
            reparsed: true,
            page_count_unchanged,
            text_queries_available,
            text_query_semantics_unchanged,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "canonical output failed post-write verification",
            ));
        }
        Ok(CanonicalizeOutcome {
            report: CanonicalizeReport {
                operation: "canonicalize".into(),
                mode: "canonical".into(),
                input_bytes: self.source_len(),
                output_bytes: bytes.len(),
                object_count: self.parsed().objects.len(),
                xref_offset,
            },
            bytes,
            verification,
        })
    }
}

// Include page and physical operand identity; an operator can occur in a later
// Contents part without moving the string it consumes into that other object.
type TextSemantics = Vec<(usize, u32, u16, String)>;

fn text_query_semantics(document: &PdfDocument) -> Result<Option<TextSemantics>, PdfError> {
    let mut semantics = Vec::new();
    let mut budget = ParseBudget::for_document(document.parsed());
    let mut available = true;
    for (page_index, page_ref) in document.page_refs()?.into_iter().enumerate() {
        budget.charge_work(0)?;
        let page = document.parsed().object(page_ref)?;
        let references = match dict_get(&page.value, b"Contents") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Ref(reference)) => vec![*reference],
            Some(Value::Array(values)) => values
                .iter()
                .map(|value| {
                    let Value::Ref(reference) = value else {
                        return Err(PdfError::syntax(
                            "page /Contents array contains a non-reference",
                            page.offset,
                        ));
                    };
                    Ok(*reference)
                })
                .collect::<Result<Vec<_>, PdfError>>()?,
            Some(_) => {
                return Err(PdfError::unsupported(
                    "direct page content streams are not implemented",
                ));
            }
        };
        let mut filtered = false;
        for reference in &references {
            let object = document.parsed().object(*reference)?;
            if object.stream.is_none() {
                return Err(PdfError::syntax(
                    "page content reference is not a stream",
                    object.offset,
                ));
            }
            filtered |= dict_get(&object.value, b"Filter")
                .is_some_and(|filter| !matches!(filter, Value::Null));
        }
        if filtered {
            // Preserve the legacy explicit unavailable result for filtered text.
            // Never parse later unfiltered parts with a fabricated initial state.
            // Other complete pages are still checked; unsupported graphics-state
            // interpretation there must not be hidden by this unavailable page.
            available = false;
            continue;
        }
        let mut state = content::ContentTextState::new();
        for (part_index, reference) in references.iter().enumerate() {
            let object = document.parsed().object(*reference)?;
            let extraction = content::extract_text_show_part(
                object.stream.as_deref().expect("stream checked above"),
                0,
                part_index,
                &mut state,
                &document.parsed().limits,
                &mut budget,
            )?;
            for event in extraction.events {
                if let content::TextShowEvent::Text(item) = event {
                    let origin = references.get(item.part_index).ok_or_else(|| {
                        PdfError::verification("canonical text operand provenance is missing")
                    })?;
                    semantics.push((page_index, origin.number, origin.generation, item.text));
                }
            }
        }
        state.finish()?;
    }
    Ok(available.then_some(semantics))
}
