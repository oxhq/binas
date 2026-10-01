use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use binas_core::{CancellationToken, Source, SourceId, SourceSpan, Span};
use serde::{Deserialize, Serialize};

use crate::{
    content,
    error::{PdfError, PdfErrorCode},
    fonts::FontDecoder,
    limits::{EngineConfig, OpenOptions},
    parser::{self, ObjectRef, ParseBudget, ParsedDocument, Value},
};

#[derive(Clone, Debug)]
pub struct PdfEngine {
    config: EngineConfig,
}

impl PdfEngine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub fn open(&self, input: &[u8], options: OpenOptions) -> Result<PdfDocument, PdfError> {
        let bytes = Source::copy_from_slice(input, self.config.limits.max_input_bytes)
            .map_err(|error| PdfError::limit(error.to_string()))?;
        self.open_source(bytes, options)
    }

    /// Takes ownership of the existing input allocation without copying its payload.
    pub fn open_owned(
        &self,
        input: Vec<u8>,
        options: OpenOptions,
    ) -> Result<PdfDocument, PdfError> {
        let bytes = Source::from_owned(input, self.config.limits.max_input_bytes)
            .map_err(|error| PdfError::limit(error.to_string()))?;
        self.open_source(bytes, options)
    }

    /// Shares an immutable input allocation. Parsing still allocates objects.
    pub fn open_shared(
        &self,
        input: Arc<[u8]>,
        options: OpenOptions,
    ) -> Result<PdfDocument, PdfError> {
        let bytes = Source::from_shared(input, self.config.limits.max_input_bytes)
            .map_err(|error| PdfError::limit(error.to_string()))?;
        self.open_source(bytes, options)
    }

    /// Opens an existing immutable source without rehashing or copying its bytes.
    pub fn open_source(
        &self,
        bytes: Source,
        options: OpenOptions,
    ) -> Result<PdfDocument, PdfError> {
        self.open_source_with_cancellation(bytes, options, CancellationToken::new())
    }

    /// Opens an immutable source with cooperative cancellation at decode work
    /// checkpoints. This does not make every parser loop interruptible.
    pub fn open_source_with_cancellation(
        &self,
        bytes: Source,
        options: OpenOptions,
        cancellation: CancellationToken,
    ) -> Result<PdfDocument, PdfError> {
        if cancellation.is_cancelled() {
            return Err(PdfError::from_context(binas_core::ContextError::Cancelled));
        }
        if bytes.len() > self.config.limits.max_input_bytes {
            return Err(PdfError::limit("input exceeds max_input_bytes"));
        }
        let parsed = if options.repair {
            parser::parse_document_repair(&bytes, &self.config.limits, cancellation.clone())?
        } else {
            parser::parse_document(&bytes, &self.config.limits, cancellation.clone())?
        };
        Ok(PdfDocument {
            bytes,
            parsed,
            config: self.config.clone(),
        })
    }

    pub fn open_with_password(
        &self,
        input: &[u8],
        password: &str,
        options: OpenOptions,
    ) -> Result<PdfDocument, PdfError> {
        if options.repair {
            return Err(PdfError::unsupported("PDF repair mode is not implemented"));
        }
        let mut encrypted = self.open_skeleton(input)?;
        if !crate::security::inspect_encryption(&encrypted)?.encrypted {
            parser::finish_compressed_objects(&mut encrypted.parsed)?;
            return Ok(encrypted);
        }
        let plaintext = encrypted.decrypt_to_plain(password)?;
        self.open_owned(plaintext.bytes, options)
    }

    pub fn decrypt_input_to_plain(
        &self,
        input: &[u8],
        password: &str,
        options: OpenOptions,
    ) -> Result<crate::DecryptionOutcome, PdfError> {
        if options.repair {
            return Err(PdfError::unsupported("PDF repair mode is not implemented"));
        }
        self.open_skeleton(input)?.decrypt_to_plain(password)
    }

    pub fn inspect_encryption_input(
        &self,
        input: &[u8],
        options: OpenOptions,
    ) -> Result<crate::security::EncryptionMetadata, PdfError> {
        if options.repair {
            return Err(PdfError::unsupported("PDF repair mode is not implemented"));
        }
        crate::security::inspect_encryption(&self.open_skeleton(input)?)
    }

    pub(crate) fn open_skeleton(&self, input: &[u8]) -> Result<PdfDocument, PdfError> {
        let bytes = Source::copy_from_slice(input, self.config.limits.max_input_bytes)
            .map_err(|error| PdfError::limit(error.to_string()))?;
        let parsed =
            parser::parse_document_skeleton(&bytes, &self.config.limits, CancellationToken::new())?;
        Ok(PdfDocument {
            bytes,
            parsed,
            config: self.config.clone(),
        })
    }

    pub(crate) fn config(&self) -> &EngineConfig {
        &self.config
    }
}

impl Default for PdfEngine {
    fn default() -> Self {
        Self::new(EngineConfig::default())
    }
}

#[derive(Clone, Debug)]
pub struct PdfDocument {
    bytes: Source,
    parsed: ParsedDocument,
    config: EngineConfig,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InspectResult {
    pub version: String,
    pub object_count: usize,
    pub page_count: usize,
    pub xref_revisions: usize,
}

/// Effective, zero-based page geometry after page-tree inheritance.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PageGeometry {
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    /// Direct page `/BleedBox`, or `None`; no fallback or parent inheritance is applied.
    pub bleed_box: Option<[f64; 4]>,
    /// Direct page `/TrimBox`, or `None`; no fallback or parent inheritance is applied.
    pub trim_box: Option<[f64; 4]>,
    /// Direct page `/ArtBox`, or `None`; no fallback or parent inheritance is applied.
    pub art_box: Option<[f64; 4]>,
    pub rotation_degrees: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct QueryMatch {
    pub match_index: usize,
    pub text: String,
    pub object_number: u32,
    pub generation: u16,
    pub source_span: Option<Span>,
    pub decoded_span: Span,
    pub font_name: Option<String>,
    pub to_unicode: bool,
    pub operator: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextGeometryConfidence {
    ExactOrigin,
    UnknownAdvance,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextGeometry {
    pub user_matrix: [f64; 6],
    pub text_matrix: Option<[f64; 6]>,
    pub origin: Option<[f64; 2]>,
    pub font_size: Option<f64>,
    pub confidence: TextGeometryConfidence,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextSpan {
    pub page_index: usize,
    pub text: String,
    pub object_number: u32,
    pub generation: u16,
    pub source_span: Option<Span>,
    pub decoded_span: Span,
    pub font_name: Option<String>,
    pub to_unicode: bool,
    pub operator: String,
    pub geometry: TextGeometry,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextExtractionWarning {
    pub page_index: usize,
    pub object_number: u32,
    pub generation: u16,
    pub code: PdfErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextExtraction {
    pub spans: Vec<TextSpan>,
    pub warnings: Vec<TextExtractionWarning>,
}

struct TextPart<'a> {
    reference: ObjectRef,
    resources: Option<&'a Value>,
}

struct TextExtractionState<'a> {
    budget: ParseBudget,
    cmap_cache: BTreeMap<ObjectRef, FontDecoder>,
    stream_visits: usize,
    parts: Vec<TextPart<'a>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub object_count: usize,
    pub page_count: usize,
}

impl PdfDocument {
    /// Clones the document with a different decode cancellation token while
    /// retaining the same shared cache and spent decode work. Source/stream
    /// payloads and object/xref maps are shared. Later mutations copy sparse
    /// change-map metadata and the affected object; this is not an operation
    /// transaction or a reset of resource quotas.
    pub fn with_cancellation(&self, cancellation: CancellationToken) -> Self {
        let mut document = self.clone();
        document.parsed.decode_cancellation = cancellation;
        document
    }

    /// Reports managed decoded-payload retention and cumulative decode work,
    /// excluding parser graphs, allocator overhead and decoder scratch memory.
    pub fn decode_metrics(&self) -> crate::DecodeMetrics {
        self.parsed.decode_state.metrics()
    }

    /// Evicts cached results without refunding spent work. Outstanding decoded
    /// handles keep their allocation reservations until the last handle drops.
    pub fn clear_decode_cache(&self) {
        self.parsed.decode_state.clear_cache();
    }

    pub(crate) fn with_parsed(&self, parsed: ParsedDocument) -> Self {
        Self {
            bytes: self.bytes.clone(),
            parsed,
            config: self.config.clone(),
        }
    }

    pub fn inspect(&self) -> Result<InspectResult, PdfError> {
        Ok(InspectResult {
            version: self.parsed.version.clone(),
            object_count: self.parsed.objects.len(),
            page_count: self.page_count()?,
            xref_revisions: self.parsed.xref_revisions,
        })
    }

    pub fn validate(&self) -> Result<ValidationResult, PdfError> {
        let page_count = self.page_count()?;
        Ok(ValidationResult {
            valid: true,
            object_count: self.parsed.objects.len(),
            page_count,
        })
    }

    /// Reads the effective geometry for a zero-based page index.
    pub fn page_geometry(&self, page_index: usize) -> Result<PageGeometry, PdfError> {
        let pages = self.page_refs()?;
        let page = *pages.get(page_index).ok_or_else(|| {
            PdfError::selection(format!(
                "page index {page_index} exceeds page count {}",
                pages.len()
            ))
        })?;
        let media_box = self
            .inherited_page_box(page, b"MediaBox", "media box")?
            .ok_or_else(|| PdfError::syntax("page has no inherited MediaBox", 0))?;
        let crop_box = self
            .inherited_page_box(page, b"CropBox", "crop box")?
            .unwrap_or(media_box);
        let bleed_box = self.direct_page_box(page, b"BleedBox", "bleed box")?;
        let trim_box = self.direct_page_box(page, b"TrimBox", "trim box")?;
        let art_box = self.direct_page_box(page, b"ArtBox", "art box")?;
        let rotation_degrees = match self.inherited_page_value(page, b"Rotate")? {
            None => 0,
            Some(Value::Integer(rotation)) => rotation.rem_euclid(360) as i32,
            Some(_) => return Err(PdfError::syntax("page /Rotate is not an integer", 0)),
        };
        Ok(PageGeometry {
            media_box,
            crop_box,
            bleed_box,
            trim_box,
            art_box,
            rotation_degrees,
        })
    }

    pub fn query_text(&self, needle: &str, match_index: usize) -> Result<QueryMatch, PdfError> {
        let matches = self.query_text_all(needle)?;
        let count = matches.len();
        matches
            .into_iter()
            .nth(match_index)
            .ok_or_else(|| PdfError {
                code: PdfErrorCode::SelectionNotFound,
                message: format!(
                    "text match index {match_index} is out of range for {count} matches"
                ),
                span: None,
                object: None,
            })
    }

    pub fn query_text_all(&self, needle: &str) -> Result<Vec<QueryMatch>, PdfError> {
        let extraction = self.extract_text_spans()?;
        let found = extraction
            .spans
            .into_iter()
            .filter(|span| span.text == needle)
            .collect::<Vec<_>>();
        if found.is_empty()
            && let Some(warning) = extraction.warnings.first()
        {
            return Err(PdfError {
                code: warning.code,
                message: warning.message.clone(),
                span: None,
                object: Some((warning.object_number, warning.generation)),
            });
        }
        Ok(found
            .into_iter()
            .enumerate()
            .map(|(match_index, span)| QueryMatch {
                match_index,
                text: span.text,
                object_number: span.object_number,
                generation: span.generation,
                source_span: span.source_span,
                decoded_span: span.decoded_span,
                font_name: span.font_name,
                to_unicode: span.to_unicode,
                operator: span.operator,
            })
            .collect())
    }

    pub fn extract_text_spans(&self) -> Result<TextExtraction, PdfError> {
        self.extract_text_for_pages(self.page_refs()?.into_iter().enumerate())
    }

    /// Extracts one page using its original zero-based index. The page tree is
    /// resolved, but unrelated page content streams are not decoded or parsed.
    pub fn extract_page_text_spans(&self, page_index: usize) -> Result<TextExtraction, PdfError> {
        let pages = self.page_refs()?;
        let page = *pages.get(page_index).ok_or_else(|| {
            PdfError::selection(format!(
                "page index {page_index} exceeds page count {}",
                pages.len()
            ))
        })?;
        self.extract_text_for_pages([(page_index, page)])
    }

    /// Caller supplies already resolved page identities and original indices.
    /// One extraction state shares stream quotas and CMap analysis across them.
    pub(crate) fn extract_text_for_pages(
        &self,
        pages: impl IntoIterator<Item = (usize, ObjectRef)>,
    ) -> Result<TextExtraction, PdfError> {
        let mut spans = Vec::new();
        let mut warnings = Vec::new();
        let mut state = TextExtractionState {
            budget: ParseBudget::for_document(self.parsed()),
            cmap_cache: BTreeMap::new(),
            stream_visits: 0,
            parts: Vec::new(),
        };
        state.budget.charge_work(0)?;
        for (selected_index, (page_index, page_reference)) in pages.into_iter().enumerate() {
            state.budget.charge_work(0)?;
            if selected_index >= self.parsed.limits.max_pages {
                return Err(PdfError::limit(
                    "selected extraction page count exceeds limit",
                ));
            }
            let resources = self.inherited_resources(page_reference)?;
            let mut active_forms = BTreeSet::new();
            state.parts.clear();
            let mut content_state = content::ContentTextState::new();
            let mut complete = true;
            for reference in self.page_content_refs(page_reference)? {
                if !self.extract_text_stream(
                    page_index,
                    reference,
                    resources,
                    [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                    0,
                    &mut content_state,
                    &mut active_forms,
                    &mut state,
                    &mut spans,
                    &mut warnings,
                )? {
                    complete = false;
                    break;
                }
            }
            if complete {
                content_state.finish()?;
            }
        }
        Ok(TextExtraction { spans, warnings })
    }

    #[allow(clippy::too_many_arguments)]
    fn extract_text_stream<'a>(
        &'a self,
        page_index: usize,
        reference: ObjectRef,
        resources: Option<&'a Value>,
        parent_matrix: [f64; 6],
        depth: usize,
        content_state: &mut content::ContentTextState,
        active_forms: &mut BTreeSet<ObjectRef>,
        state: &mut TextExtractionState<'a>,
        spans: &mut Vec<TextSpan>,
        warnings: &mut Vec<TextExtractionWarning>,
    ) -> Result<bool, PdfError> {
        state.stream_visits = state
            .stream_visits
            .checked_add(1)
            .ok_or_else(|| PdfError::limit("text stream traversal count overflows"))?;
        if state.stream_visits > self.parsed.limits.max_container_items {
            return Err(PdfError::limit("text stream traversal count exceeds limit"));
        }

        let object = self.parsed.object(reference)?;
        let part_index = state.parts.len();
        state.parts.push(TextPart {
            reference,
            resources,
        });
        let encoded = object.stream.as_ref().ok_or_else(|| {
            PdfError::syntax("text content reference is not a stream", object.offset)
        })?;
        let stream = match parser::decode_stream(
            &object.value,
            encoded,
            &self.parsed.limits,
            &mut state.budget,
        ) {
            Ok(stream) => stream,
            Err(error) if error.code == PdfErrorCode::UnsupportedFeature => {
                warnings.push(TextExtractionWarning {
                    page_index,
                    object_number: reference.number,
                    generation: reference.generation,
                    code: error.code,
                    message: error.message,
                });
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let extraction = match content::extract_text_show_part(
            &stream,
            0,
            part_index,
            content_state,
            &self.parsed.limits,
            &mut state.budget,
        ) {
            Ok(extraction) => extraction,
            Err(error) if error.code == PdfErrorCode::UnsupportedFeature => {
                warnings.push(TextExtractionWarning {
                    page_index,
                    object_number: reference.number,
                    generation: reference.generation,
                    code: error.code,
                    message: error.message,
                });
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        for event in extraction.events {
            match event {
                content::TextShowEvent::Text(item) => {
                    let origin = state.parts.get(item.part_index).ok_or_else(|| {
                        PdfError::verification("text token part provenance is missing")
                    })?;
                    let reference = origin.reference;
                    let origin_object = self.parsed.object(reference)?;
                    let encoded = origin_object.stream.as_ref().ok_or_else(|| {
                        PdfError::verification("text token origin is not a stream")
                    })?;
                    let filtered = dict_get(&origin_object.value, b"Filter").is_some();
                    let font_resources = match item.font_part_index {
                        Some(index) => {
                            state
                                .parts
                                .get(index)
                                .ok_or_else(|| {
                                    PdfError::verification(
                                        "text font context provenance is missing",
                                    )
                                })?
                                .resources
                        }
                        None if item.font.is_none() => None,
                        None => {
                            return Err(PdfError::verification(
                                "active font has no defining resource context",
                            ));
                        }
                    };
                    let (text, to_unicode) = self.decode_text_item(
                        font_resources,
                        item.font.as_deref(),
                        &item.raw,
                        &mut state.budget,
                        &mut state.cmap_cache,
                    )?;
                    // A staged replacement or imported foreign payload has no
                    // byte range in this document's original input. Numeric
                    // parser offsets alone do not establish exact provenance.
                    let source_span = match encoded.source_span() {
                        Some(range) if !filtered && range.source() == self.source_id() => {
                            let start = usize::try_from(range.span().start()).map_err(|_| {
                                PdfError::limit("stream source offset exceeds usize")
                            })?;
                            Some(shift_span(item.decoded_span, start)?)
                        }
                        _ => None,
                    };
                    let font_name = item.font.as_deref().map(font_name).transpose()?;
                    let operator = String::from_utf8(item.operator)
                        .map_err(|_| PdfError::verification("text-show operator is not ASCII"))?;
                    let geometry = item
                        .geometry
                        .ok_or_else(|| PdfError::verification("text user matrix is missing"))?;
                    let user_matrix = multiply_affine(parent_matrix, geometry.user_matrix)?;
                    let combined = geometry
                        .text_matrix
                        .map(|matrix| multiply_affine(user_matrix, matrix))
                        .transpose()?;
                    let origin = combined
                        .map(|matrix| raised_origin(matrix, geometry.text_rise))
                        .transpose()?;
                    spans.push(TextSpan {
                        page_index,
                        text,
                        object_number: reference.number,
                        generation: reference.generation,
                        source_span,
                        decoded_span: item.decoded_span,
                        font_name,
                        to_unicode,
                        operator,
                        geometry: TextGeometry {
                            user_matrix,
                            text_matrix: geometry.text_matrix,
                            origin,
                            font_size: geometry.font_size,
                            confidence: if combined.is_some() {
                                TextGeometryConfidence::ExactOrigin
                            } else {
                                TextGeometryConfidence::UnknownAdvance
                            },
                        },
                    });
                }
                content::TextShowEvent::XObject(invocation) => {
                    let Some(form_reference) =
                        self.form_xobject_reference(resources, &invocation.name)?
                    else {
                        continue;
                    };
                    if depth >= self.parsed.limits.max_parser_depth {
                        return Err(PdfError::limit("Form XObject nesting depth exceeds limit"));
                    }
                    if !active_forms.insert(form_reference) {
                        return Err(PdfError::syntax("cycle in Form XObjects", 0));
                    }
                    let form = self.parsed.object(form_reference)?;
                    let form_resources = self.form_resources(&form.value, resources)?;
                    let form_parent = multiply_affine(
                        multiply_affine(parent_matrix, invocation.user_matrix)?,
                        self.form_matrix(&form.value)?,
                    )?;
                    let mut form_state = content::ContentTextState::for_form(&invocation);
                    let result = self.extract_text_stream(
                        page_index,
                        form_reference,
                        form_resources,
                        form_parent,
                        depth + 1,
                        &mut form_state,
                        active_forms,
                        state,
                        spans,
                        warnings,
                    );
                    active_forms.remove(&form_reference);
                    if result? {
                        form_state.finish()?;
                    }
                }
            }
        }
        Ok(true)
    }

    fn form_xobject_reference(
        &self,
        resources: Option<&Value>,
        name: &[u8],
    ) -> Result<Option<ObjectRef>, PdfError> {
        let resources =
            resources.ok_or_else(|| PdfError::syntax("content has no resources for /Do", 0))?;
        let resources = self.resolve_value(resources)?;
        let xobjects = dict_get(resources, b"XObject")
            .ok_or_else(|| PdfError::syntax("content resources have no /XObject dictionary", 0))?;
        let xobjects = self.resolve_value(xobjects)?;
        let xobject = dict_get(xobjects, name).ok_or_else(|| {
            PdfError::syntax(
                format!("/Do resource /{} is missing", String::from_utf8_lossy(name)),
                0,
            )
        })?;
        let Value::Ref(reference) = xobject else {
            return Err(PdfError::unsupported(
                "direct /Do XObject resources are not implemented",
            ));
        };
        let object = self.parsed.object(*reference)?;
        match dict_name(&object.value, b"Subtype") {
            Some(subtype) if subtype == b"Form" => {
                if object.stream.is_none() {
                    return Err(PdfError::syntax(
                        "Form XObject has no stream",
                        object.offset,
                    ));
                }
                if let Some(kind) = dict_name(&object.value, b"Type")
                    && kind != b"XObject"
                {
                    return Err(PdfError::syntax(
                        "Form XObject /Type is not /XObject",
                        object.offset,
                    ));
                }
                Ok(Some(*reference))
            }
            Some(_) => Ok(None),
            None => Err(PdfError::syntax(
                "/Do XObject has no /Subtype",
                object.offset,
            )),
        }
    }

    fn form_resources<'a>(
        &'a self,
        form: &'a Value,
        inherited: Option<&'a Value>,
    ) -> Result<Option<&'a Value>, PdfError> {
        let Some(resources) = dict_get(form, b"Resources") else {
            return Ok(inherited);
        };
        let resources = self.resolve_value(resources)?;
        // PDF 32000-1:2008 7.3.9: a null dictionary value is equivalent
        // to an absent entry. Preserve this method's existing absent-resource fallback.
        if matches!(resources, Value::Null) {
            return Ok(inherited);
        }

        if !matches!(resources, Value::Dict(_)) {
            return Err(PdfError::syntax(
                "Form XObject /Resources is not a dictionary",
                0,
            ));
        }
        Ok(Some(resources))
    }

    fn form_matrix(&self, form: &Value) -> Result<[f64; 6], PdfError> {
        let Some(matrix) = dict_get(form, b"Matrix") else {
            return Ok([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        };
        let matrix = self.resolve_value(matrix)?;
        let Value::Array(values) = matrix else {
            return Err(PdfError::syntax("Form XObject /Matrix is not an array", 0));
        };
        if values.len() != 6 {
            return Err(PdfError::syntax(
                "Form XObject /Matrix must contain six numbers",
                0,
            ));
        }
        let mut result = [0.0; 6];
        for (index, value) in values.iter().enumerate() {
            result[index] = match self.resolve_value(value)? {
                Value::Integer(value) => *value as f64,
                Value::Real(value) if value.is_finite() => *value,
                _ => {
                    return Err(PdfError::syntax(
                        "Form XObject /Matrix must contain finite numbers",
                        0,
                    ));
                }
            };
        }
        Ok(result)
    }

    fn decode_text_item(
        &self,
        resources: Option<&Value>,
        font_name: Option<&[u8]>,
        encoded: &[u8],
        budget: &mut ParseBudget,
        cmap_cache: &mut BTreeMap<ObjectRef, FontDecoder>,
    ) -> Result<(String, bool), PdfError> {
        if let Some(font_name) = font_name
            && let Some(cmap_reference) = self.to_unicode_ref(resources, font_name)?
        {
            let decoder = match cmap_cache.entry(cmap_reference) {
                std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::btree_map::Entry::Vacant(entry) => {
                    let object = self.parsed.object(cmap_reference)?;
                    let stream = object.stream.as_deref().ok_or_else(|| {
                        PdfError::syntax("font /ToUnicode reference is not a stream", object.offset)
                    })?;
                    let decoded =
                        parser::decode_stream(&object.value, stream, &self.parsed.limits, budget)?;
                    let cmap = crate::cmap::ToUnicodeCMap::parse_with_budget(
                        &decoded,
                        &self.parsed.limits,
                        budget,
                    )?;
                    entry.insert(FontDecoder::new(cmap))
                }
            };
            return Ok((
                decoder.decode_bounded(encoded, &self.parsed.limits, budget)?,
                true,
            ));
        }
        if encoded.len() > self.parsed.limits.max_token_bytes {
            return Err(PdfError::limit("decoded text exceeds max_token_bytes"));
        }
        budget.charge_work(encoded.len())?;
        let text = std::str::from_utf8(encoded).map_err(|_| {
            PdfError::unsupported("font has no ToUnicode CMap and text bytes are not UTF-8")
        })?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(text.len())
            .map_err(|_| PdfError::limit("decoded text allocation failed"))?;
        owned.push_str(text);
        Ok((owned, false))
    }

    fn to_unicode_ref(
        &self,
        resources: Option<&Value>,
        font_name: &[u8],
    ) -> Result<Option<ObjectRef>, PdfError> {
        let Some(resources) = resources else {
            return Err(PdfError::syntax("page has no resources for active font", 0));
        };
        let fonts = dict_get(resources, b"Font")
            .ok_or_else(|| PdfError::syntax("page resources have no /Font dictionary", 0))?;
        let fonts = self.resolve_value(fonts)?;
        let font = dict_get(fonts, font_name)
            .ok_or_else(|| PdfError::syntax("active font is missing from page resources", 0))?;
        let font = self.resolve_value(font)?;
        match dict_get(font, b"ToUnicode") {
            None => Ok(None),
            Some(Value::Ref(reference)) => Ok(Some(*reference)),
            Some(_) => Err(PdfError::unsupported(
                "direct font /ToUnicode streams are not implemented",
            )),
        }
    }

    pub(crate) fn font_cmap_ref_for_content(
        &self,
        content: ObjectRef,
        font_name: &[u8],
    ) -> Result<(ObjectRef, usize), PdfError> {
        let mut cmap = None;
        let mut references = 0usize;
        for page in self.page_refs()? {
            let page_references = self
                .page_content_refs(page)?
                .into_iter()
                .filter(|reference| *reference == content)
                .count();
            if page_references != 0 {
                references = references
                    .checked_add(page_references)
                    .ok_or_else(|| PdfError::limit("content reference count overflows"))?;
                let found = self
                    .to_unicode_ref(self.inherited_resources(page)?, font_name)?
                    .ok_or_else(|| PdfError::unsupported("active font has no ToUnicode CMap"))?;
                if cmap.is_some_and(|existing| existing != found) {
                    return Err(PdfError::unsafe_rewrite(
                        "shared content stream resolves the font to different ToUnicode CMaps",
                    ));
                }
                cmap = Some(found);
            }
        }
        cmap.map(|cmap| (cmap, references))
            .ok_or_else(|| PdfError::syntax("content stream is not referenced by a page", 0))
    }

    pub(crate) fn page_count(&self) -> Result<usize, PdfError> {
        Ok(self.page_refs()?.len())
    }

    pub(crate) fn page_refs(&self) -> Result<Vec<ObjectRef>, PdfError> {
        let root = dict_ref(&self.parsed.trailer, b"Root")
            .ok_or_else(|| PdfError::syntax("trailer has no /Root reference", 0))?;
        let catalog = self.parsed.object(root)?;
        let pages = dict_ref(&catalog.value, b"Pages")
            .ok_or_else(|| PdfError::syntax("catalog has no /Pages reference", catalog.offset))?;
        let mut visited = BTreeMap::new();
        let mut pages_out = Vec::new();
        self.collect_pages(pages, 0, &mut visited, &mut pages_out)?;
        Ok(pages_out)
    }

    fn collect_pages(
        &self,
        reference: ObjectRef,
        depth: usize,
        visited: &mut BTreeMap<ObjectRef, ()>,
        pages: &mut Vec<ObjectRef>,
    ) -> Result<(), PdfError> {
        if depth > self.parsed.limits.max_parser_depth {
            return Err(PdfError::limit("page tree depth exceeds limit"));
        }
        if visited.insert(reference, ()).is_some() {
            return Err(PdfError::syntax("cycle in page tree", 0));
        }
        let object = self.parsed.object(reference)?;
        let kind = dict_name(&object.value, b"Type");
        if kind == Some(b"Page".as_slice()) {
            pages.push(reference);
            if pages.len() > self.parsed.limits.max_pages {
                return Err(PdfError::limit("page count exceeds limit"));
            }
        } else {
            let kids = dict_array(&object.value, b"Kids").ok_or_else(|| {
                PdfError::syntax("page tree node has no /Kids array", object.offset)
            })?;
            for kid in kids {
                let Value::Ref(kid) = kid else {
                    return Err(PdfError::syntax(
                        "page tree kid is not a reference",
                        object.offset,
                    ));
                };
                self.collect_pages(*kid, depth + 1, visited, pages)?;
            }
        }
        visited.remove(&reference);
        Ok(())
    }

    pub(crate) fn page_content_refs(
        &self,
        page_ref: ObjectRef,
    ) -> Result<Vec<ObjectRef>, PdfError> {
        let mut refs = Vec::new();
        let page = self.parsed.object(page_ref)?;
        match dict_get(&page.value, b"Contents") {
            None => {}
            Some(Value::Ref(reference)) => refs.push(*reference),
            Some(Value::Array(values)) => {
                for value in values {
                    let Value::Ref(reference) = value else {
                        return Err(PdfError::syntax(
                            "page /Contents array contains a non-reference",
                            page.offset,
                        ));
                    };
                    refs.push(*reference);
                }
            }
            Some(_) => {
                return Err(PdfError::unsupported(
                    "direct page content streams are not implemented",
                ));
            }
        }
        Ok(refs)
    }

    fn inherited_page_box(
        &self,
        page_ref: ObjectRef,
        key: &[u8],
        label: &str,
    ) -> Result<Option<[f64; 4]>, PdfError> {
        self.inherited_page_value(page_ref, key)?
            .map(|value| crate::pages::page_rectangle(self, Some(value), label))
            .transpose()
    }

    fn direct_page_box(
        &self,
        page_ref: ObjectRef,
        key: &[u8],
        label: &str,
    ) -> Result<Option<[f64; 4]>, PdfError> {
        let page = self.parsed.object(page_ref)?;
        let Some(value) = dict_get(&page.value, key) else {
            return Ok(None);
        };
        let value = self.resolve_value(value)?;
        if matches!(value, Value::Null) {
            return Ok(None);
        }
        crate::pages::page_rectangle(self, Some(value), label).map(Some)
    }

    pub(crate) fn inherited_resources(
        &self,
        page_ref: ObjectRef,
    ) -> Result<Option<&Value>, PdfError> {
        self.inherited_page_value(page_ref, b"Resources")
    }

    fn inherited_page_value<'a>(
        &'a self,
        page_ref: ObjectRef,
        key: &[u8],
    ) -> Result<Option<&'a Value>, PdfError> {
        let mut current = page_ref;
        let mut visited = BTreeSet::new();
        for _ in 0..=self.parsed.limits.max_parser_depth {
            if !visited.insert(current) {
                return Err(PdfError::syntax("cycle in page inheritance", 0));
            }
            let object = self.parsed.object(current)?;
            if let Some(value) = dict_get(&object.value, key) {
                let value = self.resolve_value(value)?;
                if !matches!(value, Value::Null) {
                    return Ok(Some(value));
                }
            }
            match dict_ref(&object.value, b"Parent") {
                Some(parent) => current = parent,
                None => return Ok(None),
            }
        }
        Err(PdfError::limit("page inheritance depth exceeds limit"))
    }

    pub(crate) fn resolve_value<'a>(&'a self, mut value: &'a Value) -> Result<&'a Value, PdfError> {
        let mut visited = BTreeSet::new();
        for _ in 0..=self.parsed.limits.max_parser_depth {
            match value {
                Value::Ref(reference) => {
                    if !visited.insert(*reference) {
                        return Err(PdfError::syntax("cycle in indirect resource values", 0));
                    }
                    value = &self.parsed.object(*reference)?.value;
                }
                _ => return Ok(value),
            }
        }
        Err(PdfError::limit("indirect resource depth exceeds limit"))
    }

    pub fn source_len(&self) -> usize {
        self.bytes.len()
    }

    /// Identity of the immutable input bytes; inspection does not change it.
    pub fn source_id(&self) -> SourceId {
        self.bytes.id()
    }

    /// Qualifies an exact input range. Decoded content offsets are not source ranges.
    pub fn source_span(&self, span: Span) -> Result<SourceSpan, PdfError> {
        self.bytes
            .qualified_span(span)
            .map_err(|error| PdfError::unsafe_rewrite(error.to_string()))
    }

    /// Read-only input storage, shared by document clones.
    pub fn input_source(&self) -> &Source {
        &self.bytes
    }

    pub(crate) fn source(&self) -> &[u8] {
        self.bytes.as_bytes()
    }

    pub(crate) fn engine_config(&self) -> &EngineConfig {
        &self.config
    }

    pub(crate) fn parsed(&self) -> &ParsedDocument {
        &self.parsed
    }
}

fn shift_span(span: Span, base: usize) -> Result<Span, PdfError> {
    let start = u64::try_from(base)
        .ok()
        .and_then(|base| base.checked_add(span.start()))
        .ok_or_else(|| PdfError::limit("content source span overflows"))?;
    Span::from_start_len(start, span.len())
        .map_err(|_| PdfError::limit("content source span is invalid"))
}

fn multiply_affine(left: [f64; 6], right: [f64; 6]) -> Result<[f64; 6], PdfError> {
    let result = [
        left[0] * right[0] + left[2] * right[1],
        left[1] * right[0] + left[3] * right[1],
        left[0] * right[2] + left[2] * right[3],
        left[1] * right[2] + left[3] * right[3],
        left[0] * right[4] + left[2] * right[5] + left[4],
        left[1] * right[4] + left[3] * right[5] + left[5],
    ];
    if result.iter().all(|value| value.is_finite()) {
        Ok(result)
    } else {
        Err(PdfError::syntax(
            "composed text/Form matrix is not finite",
            0,
        ))
    }
}

fn raised_origin(matrix: [f64; 6], rise: f64) -> Result<[f64; 2], PdfError> {
    // Text rise acts on the vertical coordinate in unscaled text space, before Tm/CTM.
    let origin = [matrix[2] * rise + matrix[4], matrix[3] * rise + matrix[5]];
    if origin.iter().all(|value| value.is_finite()) {
        Ok(origin)
    } else {
        Err(PdfError::syntax("text origin with rise is not finite", 0))
    }
}

fn font_name(value: &[u8]) -> Result<String, PdfError> {
    String::from_utf8(value.to_vec())
        .map_err(|_| PdfError::syntax("font resource name is not UTF-8", 0))
}

fn dict_ref(value: &Value, key: &[u8]) -> Option<ObjectRef> {
    match dict_get(value, key)? {
        Value::Ref(value) => Some(*value),
        _ => None,
    }
}

fn dict_name<'a>(value: &'a Value, key: &[u8]) -> Option<&'a [u8]> {
    match dict_get(value, key)? {
        Value::Name(value) => Some(value),
        _ => None,
    }
}

fn dict_array<'a>(value: &'a Value, key: &[u8]) -> Option<&'a [Value]> {
    match dict_get(value, key)? {
        Value::Array(value) => Some(value),
        _ => None,
    }
}

fn dict_get<'a>(value: &'a Value, key: &[u8]) -> Option<&'a Value> {
    match value {
        Value::Dict(value) => value.get(key),
        _ => None,
    }
}
