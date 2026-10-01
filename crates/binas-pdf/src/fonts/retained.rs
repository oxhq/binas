//! Existing explicit ToUnicode codecs and supplied simple-font metrics.
//! Metrics use declared Widths and FontBBox, without inspecting glyph programs.
//! Shaping, font substitution, general appearance or standards assurance is not inferred.
//! Resource resolution is depth-bounded and prepaid conservatively. CMap parsing
//! shares the document's cumulative work/cancellation budget, with token and
//! expanded-text limits. Temporary mappings are not live-memory accounting.
use super::FontDecoder;
use crate::{
    PdfDocument, PdfError,
    cmap::ToUnicodeCMap,
    object::{ObjectRef, Value},
    parser::ParseBudget,
    parser::decode_stream,
};
use binas_core::OperationContext;
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) struct EncodedFontText {
    pub bytes: Vec<u8>,
    pub font: Option<ObjectRef>,
    pub cmap: ObjectRef,
}

fn dict(value: &Value) -> Result<&BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(value) => Ok(value),
        _ => Err(PdfError::unsupported(
            "font resource must resolve to a dictionary",
        )),
    }
}

pub(crate) fn encode_existing_cmap_text(
    document: &PdfDocument,
    page: ObjectRef,
    font_name: &[u8],
    replacement: &str,
    context: &OperationContext,
) -> Result<EncodedFontText, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    let document = document.with_cancellation(context.cancellation().clone());
    let parsed = document.parsed();
    if replacement.len() > parsed.limits.max_token_bytes
        || font_name.len() > parsed.limits.max_token_bytes
    {
        return Err(PdfError::limit(
            "retained font text/name exceeds max_token_bytes",
        ));
    }
    // Inheritance can resolve a reference chain at each ancestor; prepay the
    // configured upper bound before those existing bounded resolution routines.
    let depth = parsed.limits.max_parser_depth.saturating_add(1);
    let work = depth
        .checked_mul(depth)
        .and_then(|n| n.checked_add(depth * 2))
        .ok_or_else(|| PdfError::limit("font resolution work overflows"))?;
    context
        .charge_work(work as u64)
        .map_err(PdfError::from_context)?;
    let resources = document
        .inherited_resources(page)?
        .ok_or_else(|| PdfError::unsupported("page has no font resources"))?;
    encode_existing_cmap_resource_text(&document, resources, font_name, replacement, context)
}

type ResolvedResourceFont<'a> = (Option<ObjectRef>, &'a BTreeMap<Vec<u8>, Value>);

fn resource_font<'a>(
    document: &'a PdfDocument,
    resources: &'a Value,
    font_name: &[u8],
    context: &OperationContext,
) -> Result<ResolvedResourceFont<'a>, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    let parsed = document.parsed();
    if font_name.len() > parsed.limits.max_token_bytes {
        return Err(PdfError::limit(
            "font resource name exceeds max_token_bytes",
        ));
    }
    let work = parsed
        .limits
        .max_parser_depth
        .saturating_add(1)
        .checked_mul(3)
        .and_then(|value| value.checked_add(font_name.len()))
        .ok_or_else(|| PdfError::limit("font resource resolution work overflows"))?;
    context
        .charge_work(work as u64)
        .map_err(PdfError::from_context)?;
    let resources = dict(document.resolve_value(resources)?)?;
    let fonts = resources
        .get(b"Font".as_slice())
        .ok_or_else(|| PdfError::unsupported("resources have no Font dictionary"))?;
    let fonts = dict(document.resolve_value(fonts)?)?;
    let entry = fonts
        .get(font_name)
        .ok_or_else(|| PdfError::unsupported("selected font name is absent"))?;
    let (font, value) = match entry {
        Value::Ref(reference) => {
            let object = parsed.object(*reference)?;
            if object.stream.is_some() {
                return Err(PdfError::unsupported("font dictionary cannot be a stream"));
            }
            (Some(*reference), &object.value)
        }
        value => (None, value),
    };
    let font_dict = dict(value)?;
    if !matches!(font_dict.get(b"Type".as_slice()), Some(Value::Name(name)) if name == b"Font")
        || !matches!(font_dict.get(b"Subtype".as_slice()), Some(Value::Name(name)) if matches!(name.as_slice(), b"Type0" | b"Type1" | b"MMType1" | b"TrueType" | b"Type3"))
    {
        return Err(PdfError::unsupported(
            "selected resource is not a supported typed font dictionary",
        ));
    }
    Ok((font, font_dict))
}

struct ResourceCodec {
    font: Option<ObjectRef>,
    cmap: ObjectRef,
    decoder: FontDecoder,
}

fn resource_codec(
    document: &PdfDocument,
    resources: &Value,
    font_name: &[u8],
    context: &OperationContext,
) -> Result<ResourceCodec, PdfError> {
    let document = document.with_cancellation(context.cancellation().clone());
    let parsed = document.parsed();
    let (font, font_dict) = resource_font(&document, resources, font_name, context)?;
    let Some(Value::Ref(cmap_ref)) = font_dict.get(b"ToUnicode".as_slice()) else {
        return Err(PdfError::unsupported(
            "retained font encoding requires an indirect explicit ToUnicode stream",
        ));
    };
    let object = parsed.object(*cmap_ref)?;
    let stream = object
        .stream
        .as_deref()
        .ok_or_else(|| PdfError::unsupported("ToUnicode object is not a stream"))?;
    let mut budget = ParseBudget::for_document(parsed);
    let bytes = decode_stream(&object.value, stream, &parsed.limits, &mut budget)?;
    context
        .charge_work(bytes.len() as u64)
        .map_err(PdfError::from_context)?;
    let cmap = ToUnicodeCMap::parse_with_budget(&bytes, &parsed.limits, &budget)?;
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(ResourceCodec {
        font,
        cmap: *cmap_ref,
        decoder: FontDecoder::new(cmap),
    })
}

/// Resolve the font only in the supplied resources (for example an appearance
/// Form). Page resources are never an implicit fallback.
pub(crate) fn encode_existing_cmap_resource_text(
    document: &PdfDocument,
    resources: &Value,
    font_name: &[u8],
    replacement: &str,
    context: &OperationContext,
) -> Result<EncodedFontText, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if replacement.len() > document.parsed().limits.max_token_bytes {
        return Err(PdfError::limit("font replacement exceeds max_token_bytes"));
    }
    let codec = resource_codec(document, resources, font_name, context)?;
    // Different character codes can select distinct glyphs even when their
    // extracted Unicode is identical. Retained editing must not pick one silently.
    // This conservative check rejects duplicate mappings whose destination occurs
    // anywhere in the replacement, even if a different segmentation would avoid
    // that mapping. Duplicates unrelated to the replacement do not block it.
    let mut reverse = BTreeMap::new();
    let mut reservations = Vec::new();
    for (source, destination) in codec.decoder.to_unicode.mappings() {
        context
            .charge_work((source.len() + destination.len() + replacement.len() + 1) as u64)
            .map_err(PdfError::from_context)?;
        reservations.push(
            context
                .reserve(std::mem::size_of::<(&str, &[u8])>() as u64)
                .map_err(PdfError::from_context)?,
        );
        if reverse.insert(destination, source).is_some() && replacement.contains(destination) {
            return Err(PdfError::unsafe_rewrite(
                "replacement Unicode has multiple font codes; glyph equivalence is unknown",
            ));
        }
    }
    drop(reverse);
    drop(reservations);
    let encoded =
        codec
            .decoder
            .encode_with_context(replacement, &document.parsed().limits, context)?;
    Ok(EncodedFontText {
        bytes: encoded,
        font: codec.font,
        cmap: codec.cmap,
    })
}

pub(crate) fn decode_existing_cmap_resource_text(
    document: &PdfDocument,
    resources: &Value,
    font_name: &[u8],
    encoded: &[u8],
    context: &OperationContext,
) -> Result<String, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if encoded.len() > document.parsed().limits.max_token_bytes {
        return Err(PdfError::limit("font input exceeds max_token_bytes"));
    }
    context
        .charge_work(encoded.len() as u64)
        .map_err(PdfError::from_context)?;
    let document = document.with_cancellation(context.cancellation().clone());
    let codec = resource_codec(&document, resources, font_name, context)?;
    codec.decoder.decode_bounded(
        encoded,
        &document.parsed().limits,
        &ParseBudget::for_document(document.parsed()),
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SimpleFontTextMetrics {
    /// Sum of explicit widths in 1000-unit glyph space.
    pub advance: f64,
    /// Conservative union of the supplied FontBBox at each character origin.
    /// Empty text has no ink and returns [0; 4]. No glyph-program validation.
    pub bbox: [f64; 4],
}

pub(crate) fn simple_font_text_metrics(
    document: &PdfDocument,
    resources: &Value,
    font_name: &[u8],
    encoded: &[u8],
    context: &OperationContext,
) -> Result<SimpleFontTextMetrics, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    let parsed = document.parsed();
    if encoded.len() > parsed.limits.max_token_bytes {
        return Err(PdfError::limit(
            "font metrics input exceeds max_token_bytes",
        ));
    }
    let (_, font) = resource_font(document, resources, font_name, context)?;
    if !matches!(font.get(b"Subtype".as_slice()), Some(Value::Name(kind)) if matches!(kind.as_slice(), b"Type1" | b"MMType1" | b"TrueType"))
    {
        return Err(PdfError::unsupported(
            "text metrics require an explicit simple font",
        ));
    }
    let resolve = |key: &[u8]| -> Result<&Value, PdfError> {
        context
            .charge_work(parsed.limits.max_parser_depth.saturating_add(1) as u64)
            .map_err(PdfError::from_context)?;
        document.resolve_value(
            font.get(key)
                .ok_or_else(|| PdfError::unsupported("simple font metrics entry is missing"))?,
        )
    };
    let (Value::Integer(first), Value::Integer(last)) =
        (resolve(b"FirstChar")?, resolve(b"LastChar")?)
    else {
        return Err(PdfError::unsupported(
            "simple font character bounds must be integers",
        ));
    };
    if *first < 0 || *last > 255 || first > last {
        return Err(PdfError::unsupported(
            "simple font character bounds are invalid",
        ));
    }
    let Value::Array(widths) = resolve(b"Widths")? else {
        return Err(PdfError::unsupported("simple font needs explicit Widths"));
    };
    if widths.len() != (*last - *first + 1) as usize {
        return Err(PdfError::unsupported(
            "simple font Widths disagree with character bounds",
        ));
    }
    let descriptor = dict(resolve(b"FontDescriptor")?)?;
    let descriptor_bbox = descriptor
        .get(b"FontBBox".as_slice())
        .ok_or_else(|| PdfError::unsupported("font descriptor has no FontBBox"))?;
    context
        .charge_work(parsed.limits.max_parser_depth.saturating_add(1) as u64)
        .map_err(PdfError::from_context)?;
    let Value::Array(bounds) = document.resolve_value(descriptor_bbox)? else {
        return Err(PdfError::unsupported(
            "FontBBox must be an explicit rectangle",
        ));
    };
    if bounds.len() != 4 {
        return Err(PdfError::unsupported("FontBBox must contain four numbers"));
    }
    let number = |value: &Value| -> Result<f64, PdfError> {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let value = match value {
            Value::Integer(value) => *value as f64,
            Value::Real(value) => *value,
            _ => return Err(PdfError::unsupported("font metric is not a direct number")),
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(PdfError::unsupported("font metric is nonfinite"))
        }
    };
    let bbox = [
        number(&bounds[0])?,
        number(&bounds[1])?,
        number(&bounds[2])?,
        number(&bounds[3])?,
    ];
    if bbox[0] > bbox[2] || bbox[1] >= bbox[3] {
        return Err(PdfError::unsupported(
            "font ink bounds are unordered or empty",
        ));
    }
    let _widths = context
        .reserve((widths.len() * std::mem::size_of::<f64>()) as u64)
        .map_err(PdfError::from_context)?;
    let mut measured = Vec::with_capacity(widths.len());
    for width in widths {
        let value = number(width)?;
        if value < 0. {
            return Err(PdfError::unsupported("negative simple font width"));
        }
        measured.push(value);
    }
    // A simple font uses one-byte character codes. Refuse a malformed CMap
    // whose longest code interpretation would group these bytes differently.
    let codec = resource_codec(document, resources, font_name, context)?;
    let (spaces, _) = codec.decoder.to_unicode.lookup_counts();
    let code_work = spaces
        .checked_mul(36)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| PdfError::limit("font metric code work overflows"))?;
    let mut result = SimpleFontTextMetrics {
        advance: 0.,
        bbox: [0.; 4],
    };
    for (offset, code) in encoded.iter().enumerate() {
        context
            .charge_work(code_work as u64)
            .map_err(PdfError::from_context)?;
        if codec
            .decoder
            .to_unicode
            .longest_code_len(&encoded[offset..])
            != Some(1)
        {
            return Err(PdfError::unsupported(
                "simple font metrics require one-byte CMap codes",
            ));
        }
        if i64::from(*code) < *first || i64::from(*code) > *last {
            return Err(PdfError::unsupported(
                "character has no explicit font width",
            ));
        }
        let x0 = result.advance + bbox[0];
        let x1 = result.advance + bbox[2];
        result.bbox = if offset == 0 {
            [x0, bbox[1], x1, bbox[3]]
        } else {
            [
                result.bbox[0].min(x0),
                bbox[1],
                result.bbox[2].max(x1),
                bbox[3],
            ]
        };
        result.advance += measured[usize::from(*code) - *first as usize];
        if !result.advance.is_finite() || result.bbox.iter().any(|value| !value.is_finite()) {
            return Err(PdfError::unsupported("font text bounds overflow"));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Limits, OpenOptions, PdfEngine, PdfErrorCode};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};
    fn context(live: u64, work: u64) -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: live,
                max_work_units: work,
            }),
            CancellationToken::new(),
        )
    }
    fn document(duplicate: bool, direct: bool) -> PdfDocument {
        let mappings = if duplicate {
            "3 beginbfchar <01> <0041> <02> <03a9> <03> <0041> endbfchar"
        } else {
            "2 beginbfchar <01> <0041> <02> <03a9> endbfchar"
        };
        let cmap = format!("1 begincodespacerange <00> <ff> endcodespacerange {mappings}");
        let font = "<< /Type /Font /Subtype /Type1 /ToUnicode 5 0 R >>";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!(
                "<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources << /Font << /F1 {} >> >> >>",
                if direct { font } else { "4 0 R" }
            ),
            "<< /Type /Page /Parent 2 0 R >>".into(),
            font.into(),
            format!("<< /Length {} >>\nstream\n{cmap}\nendstream", cmap.len()),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }
    fn page() -> ObjectRef {
        ObjectRef {
            number: 3,
            generation: 0,
        }
    }
    fn metric_document() -> (PdfDocument, Value) {
        let doc = document(false, false);
        let mut parsed = doc.parsed().clone();
        let font_ref = ObjectRef {
            number: 4,
            generation: 0,
        };
        let Value::Dict(font) = &mut parsed.objects.get_mut(&font_ref).unwrap().value else {
            panic!()
        };
        font.insert(b"FirstChar".to_vec(), Value::Integer(1));
        font.insert(b"LastChar".to_vec(), Value::Integer(2));
        font.insert(
            b"Widths".to_vec(),
            Value::Array(vec![Value::Integer(600), Value::Integer(700)]),
        );
        font.insert(
            b"FontDescriptor".to_vec(),
            Value::Dict(BTreeMap::from([(
                b"FontBBox".to_vec(),
                Value::Array([-50, -250, 650, 900].map(Value::Integer).into()),
            )])),
        );
        let resources = Value::Dict(BTreeMap::from([(
            b"Font".to_vec(),
            Value::Dict(BTreeMap::from([(b"APFont".to_vec(), Value::Ref(font_ref))])),
        )]));
        (doc.with_parsed(parsed), resources)
    }

    #[test]
    fn explicit_resources_never_fall_back_to_page_fonts_and_measure_declared_ink() {
        let (doc, resources) = metric_document();
        let ctx = context(1_000_000, 1_000_000);
        let encoded =
            encode_existing_cmap_resource_text(&doc, &resources, b"APFont", "AΩ", &ctx).unwrap();
        assert_eq!(encoded.bytes, [1, 2]);
        assert_eq!(
            decode_existing_cmap_resource_text(&doc, &resources, b"APFont", &[1, 2], &ctx).unwrap(),
            "AΩ"
        );
        assert!(encode_existing_cmap_resource_text(&doc, &resources, b"F1", "A", &ctx).is_err());
        assert!(encode_existing_cmap_text(&doc, page(), b"APFont", "A", &ctx).is_err());
        assert_eq!(
            simple_font_text_metrics(&doc, &resources, b"APFont", &[1, 2], &ctx).unwrap(),
            SimpleFontTextMetrics {
                advance: 1300.,
                bbox: [-50., -250., 1250., 900.],
            }
        );
        assert_eq!(
            simple_font_text_metrics(&doc, &resources, b"APFont", &[], &ctx).unwrap(),
            SimpleFontTextMetrics {
                advance: 0.,
                bbox: [0.; 4]
            }
        );
        assert!(simple_font_text_metrics(&doc, &resources, b"APFont", &[3], &ctx).is_err());
        assert_eq!(ctx.budget().snapshot().live_bytes, 0);
    }

    #[test]
    fn explicit_metrics_reject_missing_malformed_and_overflowing_widths() {
        for value in [
            Value::Null,
            Value::Array(vec![Value::Integer(600)]),
            Value::Array(vec![Value::Integer(-1), Value::Integer(700)]),
            Value::Array(vec![Value::Real(f64::INFINITY), Value::Integer(700)]),
            Value::Array(vec![Value::Real(f64::MAX), Value::Real(f64::MAX)]),
        ] {
            let (doc, resources) = metric_document();
            let mut parsed = doc.parsed().clone();
            let Value::Dict(font) = &mut parsed
                .objects
                .get_mut(&ObjectRef {
                    number: 4,
                    generation: 0,
                })
                .unwrap()
                .value
            else {
                panic!()
            };
            font.insert(b"Widths".to_vec(), value);
            assert!(
                simple_font_text_metrics(
                    &doc.with_parsed(parsed),
                    &resources,
                    b"APFont",
                    &[1, 2],
                    &context(1_000_000, 1_000_000)
                )
                .is_err()
            );
        }
        let (doc, resources) = metric_document();
        let source = doc.input_source().clone();
        for ctx in [context(0, 1_000_000), context(1_000_000, 0)] {
            assert_eq!(
                simple_font_text_metrics(&doc, &resources, b"APFont", &[1], &ctx)
                    .unwrap_err()
                    .code,
                PdfErrorCode::ResourceLimit
            );
            assert_eq!(ctx.budget().snapshot().live_bytes, 0);
        }
        let cancelled = context(1_000_000, 1_000_000);
        cancelled.cancellation().cancel();
        assert_eq!(
            decode_existing_cmap_resource_text(&doc, &resources, b"APFont", &[1], &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        assert_eq!(doc.input_source().as_bytes(), source.as_bytes());
    }

    #[test]
    fn simple_metrics_refuse_multibyte_character_interpretation() {
        let (doc, resources) = metric_document();
        let mut parsed = doc.parsed().clone();
        let cmap = parsed
            .objects
            .get_mut(&ObjectRef {
                number: 5,
                generation: 0,
            })
            .unwrap();
        let bytes = b"1 begincodespacerange <0000> <ffff> endcodespacerange 1 beginbfchar <0102> <0041> endbfchar".to_vec();
        let Value::Dict(values) = &mut cmap.value else {
            panic!()
        };
        values.insert(b"Length".to_vec(), Value::Integer(bytes.len() as i64));
        cmap.stream = Some(bytes.into());
        let doc = doc.with_parsed(parsed);
        let ctx = context(1_000_000, 1_000_000);
        assert_eq!(
            decode_existing_cmap_resource_text(&doc, &resources, b"APFont", &[1, 2], &ctx).unwrap(),
            "A"
        );
        assert_eq!(
            simple_font_text_metrics(&doc, &resources, b"APFont", &[1, 2], &ctx)
                .unwrap_err()
                .code,
            PdfErrorCode::UnsupportedFeature
        );
    }

    #[test]
    fn inherited_explicit_cmap_encodes_unicode_and_rejects_duplicate_codes() {
        for direct in [false, true] {
            let doc = document(false, direct);
            let ctx = context(1_000_000, 1_000_000);
            let encoded = encode_existing_cmap_text(&doc, page(), b"F1", "AΩ", &ctx).unwrap();
            assert_eq!(encoded.bytes, [1, 2]);
            assert_eq!(
                encoded.font,
                if direct {
                    None
                } else {
                    Some(ObjectRef {
                        number: 4,
                        generation: 0,
                    })
                }
            );
            assert_eq!(
                encoded.cmap,
                ObjectRef {
                    number: 5,
                    generation: 0
                }
            );
            assert_eq!(ctx.budget().snapshot().live_bytes, 0);
            assert!(ctx.budget().snapshot().work_units > 0);
            assert!(encode_existing_cmap_text(&doc, page(), b"F1", "missing", &ctx).is_err());
        }
        let ambiguous = document(true, false);
        assert_eq!(
            encode_existing_cmap_text(
                &ambiguous,
                page(),
                b"F1",
                "A",
                &context(1_000_000, 1_000_000)
            )
            .unwrap_err()
            .code,
            PdfErrorCode::UnsafeRewrite
        );
        // Duplicate codes irrelevant to this requested replacement do not block it.
        assert_eq!(
            encode_existing_cmap_text(
                &ambiguous,
                page(),
                b"F1",
                "Ω",
                &context(1_000_000, 1_000_000)
            )
            .unwrap()
            .bytes,
            [2]
        );
    }
    #[test]
    fn exact_generation_and_context_failures_leave_source_unchanged() {
        let doc = document(false, false);
        let source = doc.input_source().clone();
        let mut parsed = doc.parsed().clone();
        if let Value::Dict(font) = &mut parsed
            .objects
            .get_mut(&ObjectRef {
                number: 4,
                generation: 0,
            })
            .unwrap()
            .value
        {
            font.insert(
                b"ToUnicode".to_vec(),
                Value::Ref(ObjectRef {
                    number: 5,
                    generation: 1,
                }),
            );
        }
        assert!(
            encode_existing_cmap_text(
                &doc.with_parsed(parsed),
                page(),
                b"F1",
                "A",
                &context(1_000_000, 1_000_000)
            )
            .is_err()
        );
        for ctx in [context(0, 1_000_000), context(1_000_000, 1)] {
            assert_eq!(
                encode_existing_cmap_text(&doc, page(), b"F1", "A", &ctx)
                    .unwrap_err()
                    .code,
                PdfErrorCode::ResourceLimit
            );
        }
        let cancelled = context(1_000_000, 1_000_000);
        cancelled.cancellation().cancel();
        assert_eq!(
            encode_existing_cmap_text(&doc, page(), b"F1", "A", &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        assert_eq!(doc.input_source().as_bytes(), source.as_bytes());
    }
    #[test]
    fn contextual_decoder_preserves_legacy_selection_and_spent_work() {
        let cmap = ToUnicodeCMap::parse(b"1 begincodespacerange <00> <ff> endcodespacerange 2 beginbfchar <01> <0041> <02> <0041> endbfchar", &Limits::default()).unwrap();
        let decoder = FontDecoder::new(cmap);
        let ctx = context(1_000_000, 1_000_000);
        assert_eq!(
            decoder
                .encode_with_context("AA", &Limits::default(), &ctx)
                .unwrap(),
            decoder.encode("AA", &Limits::default()).unwrap()
        );
        assert_eq!(ctx.budget().snapshot().live_bytes, 0);
        let spent = ctx.budget().snapshot().work_units;
        assert!(spent > 0);
        assert_eq!(
            decoder
                .encode_with_context("A", &Limits::default(), &context(1_000_000, 3))
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(ctx.budget().snapshot().work_units, spent);
    }
}
