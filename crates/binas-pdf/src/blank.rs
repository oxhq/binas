use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    document::{PdfDocument, PdfEngine},
    error::PdfError,
    limits::OpenOptions,
    parser::{IndirectObject, ObjectRef, Value},
    writer::{Output, require_classic_offset, write_object},
};

/// The dimensions of one blank PDF page, in points.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct BlankPageSize {
    pub width: f64,
    pub height: f64,
}

impl PdfEngine {
    /// Creates a canonical PDF containing one or more blank pages.
    ///
    /// Dimensions must be finite and positive. The engine's normal output,
    /// page, object, xref, and parser limits apply.
    pub fn create_blank_pdf(&self, pages: &[BlankPageSize]) -> Result<Vec<u8>, PdfError> {
        validate_page_sizes(self, pages)?;
        let bytes = write_blank_pdf(self, pages)?;
        verify_blank_pdf(self, &bytes, pages)?;
        Ok(bytes)
    }
}

fn validate_page_sizes(engine: &PdfEngine, pages: &[BlankPageSize]) -> Result<(), PdfError> {
    if pages.is_empty() {
        return Err(PdfError::unsafe_rewrite(
            "blank PDF requires at least one page",
        ));
    }
    let limits = &engine.config().limits;
    if pages.len() > limits.max_pages {
        return Err(PdfError::limit("blank PDF page count exceeds max_pages"));
    }
    if pages.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "blank PDF page count exceeds max_container_items",
        ));
    }
    let object_count = pages
        .len()
        .checked_add(2)
        .ok_or_else(|| PdfError::limit("blank PDF object count overflows"))?;
    let xref_entries = object_count
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("blank PDF xref size overflows"))?;
    if object_count > limits.max_objects || xref_entries > limits.max_xref_entries {
        return Err(PdfError::limit(
            "blank PDF object allocation exceeds configured limits",
        ));
    }
    u32::try_from(object_count)
        .map_err(|_| PdfError::limit("blank PDF object number exceeds u32"))?;
    for page in pages {
        if !page.width.is_finite()
            || !page.height.is_finite()
            || page.width <= 0.0
            || page.height <= 0.0
        {
            return Err(PdfError::unsafe_rewrite(
                "blank page dimensions must be finite and positive",
            ));
        }
    }
    Ok(())
}

fn write_blank_pdf(engine: &PdfEngine, pages: &[BlankPageSize]) -> Result<Vec<u8>, PdfError> {
    let limits = &engine.config().limits;
    let mut output = Output::new(limits.max_output_bytes);
    output.push(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n")?;

    let mut offsets = Vec::with_capacity(
        pages
            .len()
            .checked_add(2)
            .ok_or_else(|| PdfError::limit("blank PDF object count overflows"))?,
    );
    write_indirect(
        &mut output,
        1,
        dictionary([
            (b"Type", Value::Name(b"Catalog".to_vec())),
            (b"Pages", reference(2)),
        ]),
        limits.max_parser_depth,
        &mut offsets,
    )?;

    let mut kids = Vec::with_capacity(pages.len());
    for index in 0..pages.len() {
        let page_number = u32::try_from(
            index
                .checked_add(3)
                .ok_or_else(|| PdfError::limit("blank PDF page object number overflows"))?,
        )
        .map_err(|_| PdfError::limit("blank PDF page object number exceeds u32"))?;
        kids.push(reference(page_number));
    }
    write_indirect(
        &mut output,
        2,
        dictionary([
            (b"Type", Value::Name(b"Pages".to_vec())),
            (b"Kids", Value::Array(kids)),
            (
                b"Count",
                Value::Integer(
                    i64::try_from(pages.len())
                        .map_err(|_| PdfError::limit("blank PDF page count exceeds i64"))?,
                ),
            ),
        ]),
        limits.max_parser_depth,
        &mut offsets,
    )?;

    for (index, page) in pages.iter().enumerate() {
        let page_number = u32::try_from(
            index
                .checked_add(3)
                .ok_or_else(|| PdfError::limit("blank PDF page object number overflows"))?,
        )
        .map_err(|_| PdfError::limit("blank PDF page object number exceeds u32"))?;
        write_indirect(
            &mut output,
            page_number,
            dictionary([
                (b"Type", Value::Name(b"Page".to_vec())),
                (b"Parent", reference(2)),
                (
                    b"MediaBox",
                    Value::Array(vec![
                        Value::Integer(0),
                        Value::Integer(0),
                        Value::Real(page.width),
                        Value::Real(page.height),
                    ]),
                ),
            ]),
            limits.max_parser_depth,
            &mut offsets,
        )?;
    }

    let size = offsets
        .len()
        .checked_add(1)
        .ok_or_else(|| PdfError::limit("blank PDF xref size overflows"))?;
    let xref_offset = output.len();
    require_classic_offset(xref_offset)?;
    output.formatted(format_args!("xref\n0 {size}\n"))?;
    output.push(b"0000000000 65535 f \n")?;
    for offset in offsets {
        output.formatted(format_args!("{offset:010} 00000 n \n"))?;
    }
    output.formatted(format_args!(
        "trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
    ))?;
    Ok(output.into_bytes())
}

fn write_indirect(
    output: &mut Output,
    number: u32,
    value: Value,
    max_depth: usize,
    offsets: &mut Vec<usize>,
) -> Result<(), PdfError> {
    let offset = output.len();
    require_classic_offset(offset)?;
    offsets.push(offset);
    output.formatted(format_args!("{number} 0 obj\n"))?;
    write_object(
        output,
        &IndirectObject {
            value,
            stream: None,
            stream_offset: 0,
            offset,
        },
        max_depth,
    )?;
    output.push(b"\nendobj\n")
}

fn dictionary<const N: usize>(entries: [(&[u8], Value); N]) -> Value {
    Value::Dict(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_vec(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn reference(number: u32) -> Value {
    Value::Ref(ObjectRef {
        number,
        generation: 0,
    })
}

fn verify_blank_pdf(
    engine: &PdfEngine,
    bytes: &[u8],
    expected_pages: &[BlankPageSize],
) -> Result<(), PdfError> {
    let document = engine
        .open(bytes, OpenOptions::default())
        .map_err(|error| {
            PdfError::verification(format!("blank PDF output did not reparse: {error}"))
        })?;
    let pages = document.page_refs().map_err(|error| {
        PdfError::verification(format!("blank PDF page verification failed: {error}"))
    })?;
    if pages.len() != expected_pages.len() {
        return Err(PdfError::verification(
            "blank PDF page count changed after re-open",
        ));
    }
    for (reference, expected) in pages.into_iter().zip(expected_pages) {
        if page_media_box(&document, reference)? != [0.0, 0.0, expected.width, expected.height] {
            return Err(PdfError::verification(
                "blank PDF media box changed after re-open",
            ));
        }
    }
    Ok(())
}

fn page_media_box(document: &PdfDocument, reference: ObjectRef) -> Result<[f64; 4], PdfError> {
    let Value::Dict(page) = &document.parsed().object(reference)?.value else {
        return Err(PdfError::verification("blank PDF page is not a dictionary"));
    };
    let Some(Value::Array(values)) = page.get(b"MediaBox".as_slice()) else {
        return Err(PdfError::verification(
            "blank PDF page has no direct media box",
        ));
    };
    let [left, bottom, right, top] = values.as_slice() else {
        return Err(PdfError::verification(
            "blank PDF media box has invalid arity",
        ));
    };
    let number = |value: &Value| match value {
        Value::Integer(value) => Ok(*value as f64),
        Value::Real(value) if value.is_finite() => Ok(*value),
        _ => Err(PdfError::verification(
            "blank PDF media box contains a non-finite number",
        )),
    };
    Ok([number(left)?, number(bottom)?, number(right)?, number(top)?])
}

#[cfg(test)]
mod tests {
    use crate::{BlankPageSize, EngineConfig, OpenOptions, PdfEngine, PdfErrorCode};

    #[test]
    fn creates_blank_pages_that_reopen_with_the_requested_boxes() {
        let engine = PdfEngine::default();
        let bytes = engine
            .create_blank_pdf(&[
                BlankPageSize {
                    width: 595.0,
                    height: 842.0,
                },
                BlankPageSize {
                    width: 612.0,
                    height: 792.0,
                },
            ])
            .unwrap();
        let document = engine.open(&bytes, OpenOptions::default()).unwrap();

        assert_eq!(document.inspect().unwrap().page_count, 2);
        assert_eq!(
            super::page_media_box(&document, document.page_refs().unwrap()[0]).unwrap(),
            [0.0, 0.0, 595.0, 842.0]
        );
        assert_eq!(
            super::page_media_box(&document, document.page_refs().unwrap()[1]).unwrap(),
            [0.0, 0.0, 612.0, 792.0]
        );
        assert!(document.validate().unwrap().valid);
    }

    #[test]
    fn rejects_invalid_dimensions_and_respects_output_limit() {
        let engine = PdfEngine::default();
        for page in [
            BlankPageSize {
                width: 0.0,
                height: 1.0,
            },
            BlankPageSize {
                width: f64::NAN,
                height: 1.0,
            },
            BlankPageSize {
                width: 1.0,
                height: f64::INFINITY,
            },
        ] {
            assert_eq!(
                engine.create_blank_pdf(&[page]).unwrap_err().code,
                PdfErrorCode::UnsafeRewrite
            );
        }

        let mut config = EngineConfig::default();
        config.limits.max_output_bytes = 1;
        assert_eq!(
            PdfEngine::new(config)
                .create_blank_pdf(&[BlankPageSize {
                    width: 1.0,
                    height: 1.0,
                }])
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }
}
