use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    content,
    document::{PdfDocument, PdfEngine},
    error::PdfError,
    limits::OpenOptions,
    parser::Value,
    writer::{
        Output, dict_get, dict_integer, refuse_security_boundaries, require_classic_offset,
        write_name, write_object, write_value,
    },
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
        let mut output = Output::new(self.engine_config().limits.max_output_bytes);
        output.push(b"%PDF-")?;
        output.push(self.parsed().version.as_bytes())?;
        output.push(b"\n%\xE2\xE3\xCF\xD3\n")?;

        let mut offsets = BTreeMap::new();
        for (reference, object) in &self.parsed().objects {
            if offsets.contains_key(&reference.number) {
                return Err(PdfError::unsafe_rewrite(
                    "canonical output cannot represent multiple live generations of one object",
                ));
            }
            let offset = output.len();
            require_classic_offset(offset)?;
            offsets.insert(reference.number, (offset, reference.generation));
            output.formatted(format_args!(
                "{} {} obj\n",
                reference.number, reference.generation
            ))?;
            write_object(&mut output, object, self.parsed().limits.max_parser_depth)?;
            output.push(b"\nendobj\n")?;
        }

        let max_object = offsets.keys().next_back().copied().unwrap_or(0);
        let graph_size = usize::try_from(max_object)
            .ok()
            .and_then(|number| number.checked_add(1))
            .ok_or_else(|| PdfError::limit("canonical xref size overflows"))?;
        let trailer_size = dict_integer(&self.parsed().trailer, b"Size")
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(0);
        let size = graph_size.max(trailer_size).max(1);
        if size > self.engine_config().limits.max_xref_entries {
            return Err(PdfError::limit(
                "canonical xref size exceeds max_xref_entries",
            ));
        }

        let xref_offset = output.len();
        require_classic_offset(xref_offset)?;
        output.formatted(format_args!("xref\n0 {size}\n"))?;
        output.push(b"0000000000 65535 f \n")?;
        for number in 1..size {
            match u32::try_from(number)
                .ok()
                .and_then(|number| offsets.get(&number))
            {
                Some((offset, generation)) => {
                    output.formatted(format_args!("{offset:010} {generation:05} n \n"))?
                }
                None => output.push(b"0000000000 00000 f \n")?,
            }
        }
        output.push(b"trailer\n<<")?;
        output.formatted(format_args!(" /Size {size}"))?;
        for key in [b"Root".as_slice(), b"Info".as_slice(), b"ID".as_slice()] {
            if let Some(value) = dict_get(&self.parsed().trailer, key) {
                output.push(b" ")?;
                write_name(&mut output, key)?;
                output.push(b" ")?;
                write_value(&mut output, value, 0, self.parsed().limits.max_parser_depth)?;
            }
        }
        output.formatted(format_args!(">>\nstartxref\n{xref_offset}\n%%EOF\n"))?;
        let bytes = output.into_bytes();

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
                object_count: offsets.len(),
                xref_offset,
            },
            bytes,
            verification,
        })
    }
}

type TextSemantics = Vec<(u32, u16, String)>;

fn text_query_semantics(document: &PdfDocument) -> Result<Option<TextSemantics>, PdfError> {
    let mut references = Vec::new();
    for page_ref in document.page_refs()? {
        let page = document.parsed().object(page_ref)?;
        match dict_get(&page.value, b"Contents") {
            None => {}
            Some(Value::Ref(reference)) => references.push(*reference),
            Some(Value::Array(values)) => {
                for value in values {
                    let Value::Ref(reference) = value else {
                        return Err(PdfError::syntax(
                            "page /Contents array contains a non-reference",
                            page.offset,
                        ));
                    };
                    references.push(*reference);
                }
            }
            Some(_) => {
                return Err(PdfError::unsupported(
                    "direct page content streams are not implemented",
                ));
            }
        }
    }
    let mut semantics = Vec::new();
    for reference in references {
        let object = document.parsed().object(reference)?;
        if dict_get(&object.value, b"Filter").is_some() {
            return Ok(None);
        }
        let stream = object.stream.as_deref().ok_or_else(|| {
            PdfError::syntax("page content reference is not a stream", object.offset)
        })?;
        for item in
            content::extract_text_show(stream, object.stream_offset, &document.parsed().limits)?
        {
            semantics.push((reference.number, reference.generation, item.text));
        }
    }
    Ok(Some(semantics))
}
