use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError, content,
    parser::{ObjectRef, ParsedDocument, Value},
    writer::refuse_security_boundaries,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InlineImageFilter {
    Raw,
    Flate,
    AsciiHex,
    Ascii85,
    RunLength,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InlineImageColorSpace {
    Gray,
    Rgb,
    Cmyk,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InlineImageInventoryEntry {
    pub page_index: usize,
    pub image_index: usize,
    pub width: u32,
    pub height: u32,
    pub color_space: InlineImageColorSpace,
    pub filter: InlineImageFilter,
    pub encoded_byte_length: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InlineImageReplacementRequest {
    pub page_index: usize,
    #[serde(default)]
    pub image_index: usize,
    pub encoded_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: InlineImageColorSpace,
    pub filter: InlineImageFilter,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InlineImageReplacementReport {
    pub operation: String,
    pub page_index: usize,
    pub image_index: usize,
    pub content_object_number: u32,
    pub content_object_generation: u16,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub encoded_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InlineImageReplacementVerification {
    pub passed: bool,
    pub reparsed: bool,
    pub surrounding_bytes_preserved: bool,
    pub encoded_bytes_match: bool,
    pub dimensions_match: bool,
    pub metadata_matches: bool,
    pub content_reference_preserved: bool,
    pub page_count_unchanged: bool,
    pub no_dangling_references: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InlineImageReplacementOutcome {
    pub bytes: Vec<u8>,
    pub report: InlineImageReplacementReport,
    pub verification: InlineImageReplacementVerification,
}

/// Lists bounded inline-image metadata from one unfiltered content stream referenced by each page.
pub fn list_inline_images(
    document: &PdfDocument,
) -> Result<Vec<InlineImageInventoryEntry>, PdfError> {
    let mut entries = Vec::new();
    for (page_index, page_ref) in document.page_refs()?.into_iter().enumerate() {
        let page = document.parsed().object(page_ref)?;
        let page = dictionary(&page.value, "page")?;
        let Some(Value::Ref(content_ref)) = page.get(b"Contents".as_slice()) else {
            if page.contains_key(b"Contents".as_slice()) {
                return Err(PdfError::unsupported(
                    "inline image inventory requires one indirect page content stream",
                ));
            }
            continue;
        };
        let content = document.parsed().object(*content_ref)?;
        let dictionary = dictionary(&content.value, "page content")?;
        if dictionary.contains_key(b"Filter".as_slice())
            || dictionary.contains_key(b"DecodeParms".as_slice())
        {
            return Err(PdfError::unsupported(
                "inline image inventory requires unfiltered page content streams",
            ));
        }
        let stream = content
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::unsupported("page content reference is not a stream"))?;
        for (image_index, image) in content::inline_images(stream, &document.parsed().limits)?
            .into_iter()
            .enumerate()
        {
            if entries.len() >= document.parsed().limits.max_container_items {
                return Err(PdfError::limit(
                    "inline image inventory count exceeds container limit",
                ));
            }
            entries.push(InlineImageInventoryEntry {
                page_index,
                image_index,
                width: image.width,
                height: image.height,
                color_space: inventory_color_space(image.color_space),
                filter: inventory_filter(image.filter),
                encoded_byte_length: image.data_end.checked_sub(image.data_start).ok_or_else(
                    || PdfError::verification("inline image parser returned an invalid data span"),
                )?,
            });
        }
    }
    Ok(entries)
}

impl PdfDocument {
    pub fn replace_inline_image(
        &self,
        request: InlineImageReplacementRequest,
    ) -> Result<InlineImageReplacementOutcome, PdfError> {
        refuse_security_boundaries(self.parsed())?;
        if request.encoded_bytes.is_empty() {
            return Err(PdfError::unsupported(
                "inline image replacement bytes must not be empty",
            ));
        }
        if request.encoded_bytes.len() > self.parsed().limits.max_stream_bytes {
            return Err(PdfError::limit(
                "inline image replacement exceeds stream limit",
            ));
        }
        let normalized = normalized_inline_image(&request);
        let parsed_replacement = content::inline_images(&normalized, &self.parsed().limits)?;
        let replacement = parsed_replacement
            .first()
            .filter(|_| parsed_replacement.len() == 1)
            .ok_or_else(|| PdfError::verification("normalized inline image did not parse once"))?;
        if normalized[replacement.data_start..replacement.data_end] != request.encoded_bytes {
            return Err(PdfError::verification(
                "normalized inline image payload boundaries changed",
            ));
        }
        let pages = self.page_refs()?;
        let page_ref = pages.get(request.page_index).copied().ok_or_else(|| {
            PdfError::selection(format!(
                "inline image page index {} exceeds page count {}",
                request.page_index,
                pages.len()
            ))
        })?;
        let content_refs = page_contents(self.parsed().object(page_ref)?.value.clone())?;
        let mut found = Vec::new();
        for reference in content_refs {
            let object = self.parsed().object(reference)?;
            let dictionary = dictionary(&object.value, "page content")?;
            if dictionary.contains_key(b"Filter".as_slice())
                || dictionary.contains_key(b"DecodeParms".as_slice())
            {
                return Err(PdfError::unsupported(
                    "inline image replacement requires unfiltered page content streams",
                ));
            }
            let stream = object
                .stream
                .as_deref()
                .ok_or_else(|| PdfError::unsupported("page content reference is not a stream"))?;
            let images = content::inline_images(stream, &self.parsed().limits)?;
            for image in images {
                found.push((reference, image));
            }
        }
        if found.len() != 1 {
            return Err(PdfError::unsupported(
                "safe replacement currently requires exactly one inline image on the page",
            ));
        }
        let (content_ref, image) = found
            .get(request.image_index)
            .cloned()
            .ok_or_else(|| PdfError::selection("inline image index is out of range"))?;
        if request.image_index != 0 {
            return Err(PdfError::selection("inline image index is out of range"));
        }
        if content_reference_count(self, content_ref)? != 1 {
            return Err(PdfError::unsafe_rewrite(
                "inline image content stream is shared by multiple pages",
            ));
        }
        let original_stream = self
            .parsed()
            .object(content_ref)?
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::unsupported("inline image content is not a stream"))?;
        let prefix = &original_stream[..image.start];
        let suffix = &original_stream[image.end..];
        if !prefix.is_ascii() || !suffix.is_ascii() {
            return Err(PdfError::unsupported(
                "inline image replacement requires ASCII surrounding content",
            ));
        }
        let output_length = prefix
            .len()
            .checked_add(normalized.len())
            .and_then(|length| length.checked_add(suffix.len()))
            .ok_or_else(|| PdfError::limit("inline image content length overflows"))?;
        if output_length > self.parsed().limits.max_stream_bytes {
            return Err(PdfError::limit("inline image content exceeds stream limit"));
        }
        let mut stream = Vec::with_capacity(output_length);
        stream.extend_from_slice(prefix);
        stream.extend_from_slice(&normalized);
        stream.extend_from_slice(suffix);
        let old_pages = pages.len();
        let mut parsed = self.parsed().clone();
        let object = parsed
            .objects
            .get_mut(&content_ref)
            .ok_or_else(|| PdfError::selection("inline image content object disappeared"))?;
        let dictionary = dictionary_mut(&mut object.value, "inline image content")?;
        if matches!(dictionary.get(b"Length".as_slice()), Some(Value::Ref(_))) {
            return Err(PdfError::unsupported(
                "inline image replacement does not support indirect stream Length",
            ));
        }
        dictionary.insert(
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(stream.len())
                    .map_err(|_| PdfError::limit("inline image stream length exceeds i64"))?,
            ),
        );
        object.stream = Some(stream.clone());
        object.stream_offset = 0;
        object.offset = 0;
        let canonical = self.with_parsed(parsed).canonicalize()?;
        let reopened = PdfEngine::new(self.engine_config().clone())
            .open(&canonical.bytes, OpenOptions::default())
            .map_err(|error| {
                PdfError::verification(format!("inline image output did not reparse: {error}"))
            })?;
        let verified_object = reopened.parsed().object(content_ref)?;
        let verified_stream = verified_object.stream.as_deref().ok_or_else(|| {
            PdfError::verification("verified inline image content is not a stream")
        })?;
        let verified_images = content::inline_images(verified_stream, &reopened.parsed().limits)?;
        let verified_image = verified_images
            .first()
            .filter(|_| verified_images.len() == 1)
            .ok_or_else(|| PdfError::verification("verified inline image is ambiguous"))?;
        let surrounding_bytes_preserved =
            verified_stream.starts_with(prefix) && verified_stream.ends_with(suffix);
        let encoded_bytes_match = verified_stream
            [verified_image.data_start..verified_image.data_end]
            == request.encoded_bytes;
        let dimensions_match = verified_image.width == request.width
            && verified_image.height == request.height
            && verified_image.bits_per_component == request.bits_per_component;
        let metadata_matches = verified_image.color_space
            == content_color_space(request.color_space)
            && verified_image.filter == content_filter(request.filter);
        let content_reference_preserved = reopened.parsed().objects.contains_key(&content_ref);
        let page_count_unchanged = reopened.page_count()? == old_pages;
        let no_dangling_references = verify_references(reopened.parsed())?;
        let verification = InlineImageReplacementVerification {
            passed: surrounding_bytes_preserved
                && encoded_bytes_match
                && dimensions_match
                && metadata_matches
                && content_reference_preserved
                && page_count_unchanged
                && no_dangling_references,
            reparsed: true,
            surrounding_bytes_preserved,
            encoded_bytes_match,
            dimensions_match,
            metadata_matches,
            content_reference_preserved,
            page_count_unchanged,
            no_dangling_references,
        };
        if !verification.passed {
            return Err(PdfError::verification(
                "inline image replacement failed post-write verification",
            ));
        }
        Ok(InlineImageReplacementOutcome {
            report: InlineImageReplacementReport {
                operation: "replace_inline_image".into(),
                page_index: request.page_index,
                image_index: request.image_index,
                content_object_number: content_ref.number,
                content_object_generation: content_ref.generation,
                input_bytes: self.source_len(),
                output_bytes: canonical.bytes.len(),
                encoded_bytes: request.encoded_bytes.len(),
            },
            bytes: canonical.bytes,
            verification,
        })
    }
}

fn normalized_inline_image(request: &InlineImageReplacementRequest) -> Vec<u8> {
    let color_space = match request.color_space {
        InlineImageColorSpace::Gray => "G",
        InlineImageColorSpace::Rgb => "RGB",
        InlineImageColorSpace::Cmyk => "CMYK",
    };
    let filter = match request.filter {
        InlineImageFilter::Raw => "",
        InlineImageFilter::Flate => " /F /Fl",
        InlineImageFilter::AsciiHex => " /F /AHx",
        InlineImageFilter::Ascii85 => " /F /A85",
        InlineImageFilter::RunLength => " /F /RL",
    };
    let mut output = format!(
        "BI /W {} /H {} /BPC {} /CS /{color_space}{filter} ID\n",
        request.width, request.height, request.bits_per_component
    )
    .into_bytes();
    output.extend_from_slice(&request.encoded_bytes);
    output.extend_from_slice(b"\nEI");
    output
}

fn content_color_space(value: InlineImageColorSpace) -> content::InlineColorSpace {
    match value {
        InlineImageColorSpace::Gray => content::InlineColorSpace::Gray,
        InlineImageColorSpace::Rgb => content::InlineColorSpace::Rgb,
        InlineImageColorSpace::Cmyk => content::InlineColorSpace::Cmyk,
    }
}

fn content_filter(value: InlineImageFilter) -> content::InlineFilter {
    match value {
        InlineImageFilter::Raw => content::InlineFilter::Raw,
        InlineImageFilter::Flate => content::InlineFilter::Flate,
        InlineImageFilter::AsciiHex => content::InlineFilter::AsciiHex,
        InlineImageFilter::Ascii85 => content::InlineFilter::Ascii85,
        InlineImageFilter::RunLength => content::InlineFilter::RunLength,
    }
}

fn inventory_color_space(value: content::InlineColorSpace) -> InlineImageColorSpace {
    match value {
        content::InlineColorSpace::Gray => InlineImageColorSpace::Gray,
        content::InlineColorSpace::Rgb => InlineImageColorSpace::Rgb,
        content::InlineColorSpace::Cmyk => InlineImageColorSpace::Cmyk,
    }
}

fn inventory_filter(value: content::InlineFilter) -> InlineImageFilter {
    match value {
        content::InlineFilter::Raw => InlineImageFilter::Raw,
        content::InlineFilter::Flate => InlineImageFilter::Flate,
        content::InlineFilter::AsciiHex => InlineImageFilter::AsciiHex,
        content::InlineFilter::Ascii85 => InlineImageFilter::Ascii85,
        content::InlineFilter::RunLength => InlineImageFilter::RunLength,
    }
}

fn page_contents(value: Value) -> Result<Vec<ObjectRef>, PdfError> {
    let dictionary = dictionary(&value, "page")?;
    match dictionary.get(b"Contents".as_slice()) {
        None => Ok(Vec::new()),
        Some(Value::Ref(reference)) => Ok(vec![*reference]),
        Some(Value::Array(values)) => values
            .iter()
            .map(|value| match value {
                Value::Ref(reference) => Ok(*reference),
                _ => Err(PdfError::unsupported(
                    "page Contents arrays must contain references",
                )),
            })
            .collect(),
        Some(_) => Err(PdfError::unsupported(
            "direct page content is not supported for inline image replacement",
        )),
    }
}

fn content_reference_count(document: &PdfDocument, target: ObjectRef) -> Result<usize, PdfError> {
    let mut count = 0_usize;
    for page in document.page_refs()? {
        for reference in page_contents(document.parsed().object(page)?.value.clone())? {
            if reference == target {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn dictionary<'a>(value: &'a Value, label: &str) -> Result<&'a BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(dictionary) => Ok(dictionary),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn dictionary_mut<'a>(
    value: &'a mut Value,
    label: &str,
) -> Result<&'a mut BTreeMap<Vec<u8>, Value>, PdfError> {
    match value {
        Value::Dict(dictionary) => Ok(dictionary),
        _ => Err(PdfError::syntax(format!("{label} is not a dictionary"), 0)),
    }
}

fn verify_references(parsed: &ParsedDocument) -> Result<bool, PdfError> {
    fn walk(value: &Value, parsed: &ParsedDocument, depth: usize) -> Result<bool, PdfError> {
        if depth > parsed.limits.max_parser_depth {
            return Err(PdfError::limit(
                "inline image reference validation exceeds depth limit",
            ));
        }
        match value {
            Value::Ref(reference) => Ok(parsed.objects.contains_key(reference)),
            Value::Array(values) => values.iter().try_fold(true, |valid, value| {
                Ok(valid && walk(value, parsed, depth + 1)?)
            }),
            Value::Dict(dictionary) => dictionary.values().try_fold(true, |valid, value| {
                Ok(valid && walk(value, parsed, depth + 1)?)
            }),
            _ => Ok(true),
        }
    }
    if !walk(&parsed.trailer, parsed, 0)? {
        return Ok(false);
    }
    parsed.objects.values().try_fold(true, |valid, object| {
        Ok(valid && walk(&object.value, parsed, 0)?)
    })
}
