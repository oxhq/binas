//! Retained graph revisions for a deliberately narrow mutation slice.
//!
//! Legacy text edits replace complete ASCII tokens without font interpretation.
//! Page-scoped edits can also reuse an explicit, unambiguous ToUnicode mapping;
//! glyph programs, metrics and shaping are not verified. Supported page streams are encoded
//! once per changed stream; explicit page-scoped commands may clone a shared
//! stream for one owner or update all proven direct page owners. Indirect Length,
//! ambiguous/nested Form ownership and appearance-bearing annotations are refused. Text annotation
//! Contents use the existing PDF string encoder. No appearance or conformance
//! claim follows from these graph checks. Serialization occurs only at export.
//!
//! Sources, decode work and immutable object-store bases remain shared. Managed
//! reservations below cover planned replacement decoded bytes only; native object
//! maps, encoder scratch and other allocations retain existing PDF limits and are
//! not whole-session/process memory accounting. Legacy inspection/encoding loops
//! are not all cooperatively interruptible; bounded checkpoints surround them.

mod annotation_keys;
mod catalog_keys;
mod object_roles;
mod page_keys;
mod stream_keys;
mod text_contract;
pub use text_contract::{
    PdfPreparedTextEdit, PdfTextEditCapability, PdfTextEditError, PdfTextEditReceipt,
    PdfTextEditRefusal,
};

use crate::{
    Annotation, AnnotationContentsMutationRequest, DecodeMetrics, InspectResult, PdfDocument,
    PdfError, PdfReferenceIndex, PdfReferenceTarget, SurgicalTextEditRequest, TextSpan,
    annotations::{encode_text, list_annotations},
    edit::{encode_hex, encode_literal},
    filters::encode_pdf_stream,
    fonts::{EncodedFontText, encode_existing_cmap_text},
    forms::{checkbox, flatten, refuse_unsafe_interactive_edit, text as text_fields},
    object::{IndirectObject, ObjectRef, Value},
    ownership::{PageOwnedReference, PageOwnedRole, PageOwnedSlot, direct_page_owners},
    parser::{ParseBudget, decode_stream},
    preservation::{delete, duplicate, geometry, labels, pages},
    store::ObjectStore,
    writer::refuse_security_boundaries_with_context,
};
use binas_core::{
    OperationContext, Reservation, RevisionId, SnapshotId, Span,
    transaction::{
        CommitReceipt, PlanEnvelope, RevisionState, StagedChange, TransactionError, execute_plan,
    },
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct PdfRevisionRequest {
    pub text: Vec<SurgicalTextEditRequest>,
    /// Match indices are relative to the selected page. Sharing behavior must
    /// be explicit; the original `text` commands still require a unique owner.
    pub page_text: Vec<PdfPageTextEditRequest>,
    pub annotations: Vec<AnnotationContentsMutationRequest>,
    /// Checkbox state changes with proven field/widget ownership and existing
    /// Off/on normal appearances. This does not generate new appearances.
    pub checkboxes: Vec<crate::CheckboxFieldMutationRequest>,
    /// Paint selected terminal fields' existing normal appearances and remove
    /// their active field/widget registrations. Detached source objects remain
    /// retained; this is not secure data removal.
    pub flatten_fields: Vec<crate::PdfFieldFlattenRequest>,
    /// Fill supported fixed text-field templates using existing appearance
    /// fonts and owner-local appearance copies. No font substitution or reflow.
    pub text_fields: Vec<crate::PdfTextFieldFillRequest>,
    /// Every current zero-based page index exactly once. Selections are bound
    /// to stable page identities when planning. Explicit labels follow pages;
    /// absent labels remain absent. Logical structure-tree order is unchanged.
    pub reorder_pages: Option<Vec<usize>>,
    /// Absolute rotation and page boxes on pages selected in the starting
    /// revision. These compose with reordering; scale/translation are unsupported.
    pub page_transforms: Vec<crate::PdfPageTransformRequest>,
    /// Insert one fresh page occurrence immediately after its source. Supported
    /// Tx fields receive independent explicitly named field/widget identities;
    /// existing local link destinations keep their original target identities.
    /// This initial slice requires a separate batch from other operations.
    pub duplicate_page: Option<crate::PdfPageDuplicateRequest>,
    /// Remove one page from the active page tree, reconciling supported form
    /// registrations and explicitly selected destination behavior. Detached
    /// source objects remain stored; this is not secure erasure or redaction.
    /// This initial slice requires a separate batch from other operations.
    pub delete_page: Option<crate::PdfPageDeleteRequest>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PdfSharedEditMode {
    #[default]
    Reject,
    /// Clone a shared direct Contents stream for the selected page. A stream
    /// with only one owner is changed in place. Repeated slots on one page are
    /// ambiguous selections and refused.
    CloneForPage,
    /// Change the stream for every proven direct page owner. References from
    /// other roles or opaque objects remain unsupported. Every occurrence must
    /// have the requested Unicode meaning and the same replacement encoding.
    AllPageOwners,
}

#[derive(Clone, Debug)]
pub struct PdfPageTextEditRequest {
    pub page_index: usize,
    pub edit: SurgicalTextEditRequest,
    pub shared: PdfSharedEditMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfStreamEditEffect {
    pub source_stream: PdfReferenceTarget,
    pub output_stream: PdfReferenceTarget,
    /// Stable page identities in source page order; each affected page once.
    pub pages: Vec<PdfReferenceTarget>,
    pub shared: PdfSharedEditMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFontTextEffect {
    pub page: PdfReferenceTarget,
    pub source_stream: PdfReferenceTarget,
    /// None denotes a direct font dictionary in the page resources.
    pub font: Option<PdfReferenceTarget>,
    pub cmap: PdfReferenceTarget,
    pub encoded_bytes: usize,
}

pub type PdfRevisionReceipt = CommitReceipt<Vec<PdfReferenceTarget>, String>;

#[derive(Debug)]
pub struct PdfRevision {
    state: RevisionState<PdfDocument>,
}

impl Clone for PdfRevision {
    fn clone(&self) -> Self {
        Self {
            state: RevisionState::new(self.snapshot(), self.state.value().clone()),
        }
    }
}

#[derive(Debug)]
pub struct PdfRevisionPlan {
    envelope: PlanEnvelope<Intent>,
    store: ObjectStore,
    _decoded_reservations: Vec<Reservation>,
}
impl PdfRevisionPlan {
    pub fn snapshot(&self) -> SnapshotId {
        self.envelope.snapshot()
    }
    pub fn required_checks(&self) -> &RequiredChecks {
        self.envelope.required()
    }
    pub fn changed_objects(&self) -> impl Iterator<Item = PdfReferenceTarget> + '_ {
        self.envelope
            .intent()
            .objects
            .keys()
            .map(|reference| public_ref(*reference))
    }
    pub fn stream_effects(&self) -> impl Iterator<Item = &PdfStreamEditEffect> {
        self.envelope
            .intent()
            .streams
            .values()
            .map(|stream| &stream.effect)
    }
    /// Existing mappings used for text replacement, without font-program or
    /// appearance assurance. Font and CMap objects remain unchanged.
    pub fn font_effects(&self) -> impl Iterator<Item = &PdfFontTextEffect> {
        self.envelope.intent().font_effects.iter()
    }
    pub fn geometry_effects(&self) -> impl Iterator<Item = &crate::PdfPageGeometryEffect> {
        self.envelope
            .intent()
            .geometry
            .iter()
            .flat_map(|plan| plan.effects())
    }
    pub fn checkbox_effects(&self) -> impl Iterator<Item = &crate::PdfCheckboxEffect> {
        self.envelope
            .intent()
            .checkboxes
            .iter()
            .flat_map(|plan| plan.effects())
    }
    pub fn flatten_effects(&self) -> impl Iterator<Item = &crate::PdfFieldFlattenEffect> {
        self.envelope
            .intent()
            .flatten
            .iter()
            .flat_map(|plan| plan.effects())
    }
    pub fn text_field_effects(&self) -> impl Iterator<Item = &crate::PdfTextFieldFillEffect> {
        self.envelope
            .intent()
            .text_fields
            .iter()
            .flat_map(|plan| plan.effects())
    }
    pub fn duplicate_effect(&self) -> Option<&crate::PdfPageDuplicateEffect> {
        self.envelope
            .intent()
            .duplicate
            .as_ref()
            .map(|plan| plan.effects())
    }
    pub fn delete_effect(&self) -> Option<&crate::PdfPageDeleteEffect> {
        self.envelope
            .intent()
            .delete
            .as_ref()
            .map(|plan| plan.effects())
    }
}

#[derive(Debug)]
struct TextChange {
    span: Span,
    replacement: Vec<u8>,
}
#[derive(Debug)]
struct StreamIntent {
    expected: Vec<u8>,
    owners: Vec<PageOwnedReference>,
    effect: PdfStreamEditEffect,
}
#[derive(Debug)]
struct Intent {
    objects: BTreeMap<ObjectRef, IndirectObject>,
    streams: BTreeMap<ObjectRef, StreamIntent>,
    annotations: BTreeMap<ObjectRef, Vec<u8>>,
    reorder: Option<pages::PageReorderPlan>,
    labels: Option<labels::LabelReorder>,
    text_pages: BTreeMap<ObjectRef, Vec<TextMeaning>>,
    font_effects: Vec<PdfFontTextEffect>,
    geometry: Option<geometry::GeometryPlan>,
    checkboxes: Option<checkbox::CheckboxPlan>,
    flatten: Option<flatten::FlattenPlan>,
    text_fields: Option<text_fields::TextFillPlan>,
    duplicate: Option<duplicate::DuplicatePlan>,
    delete: Option<delete::DeletePlan>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TextMeaning {
    text: String,
    font_name: Option<String>,
    to_unicode: bool,
    operator: String,
}
impl From<&TextSpan> for TextMeaning {
    fn from(span: &TextSpan) -> Self {
        Self {
            text: span.text.clone(),
            font_name: span.font_name.clone(),
            to_unicode: span.to_unicode,
            operator: span.operator.clone(),
        }
    }
}

#[derive(Default)]
struct TextInventory {
    source: BTreeMap<usize, Vec<TextSpan>>,
    expected: BTreeMap<ObjectRef, Vec<TextMeaning>>,
    fonts: Vec<PdfFontTextEffect>,
}
impl TextInventory {
    fn page(
        &mut self,
        document: &PdfDocument,
        index: usize,
        page: ObjectRef,
    ) -> Result<&[TextSpan], PdfError> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.source.entry(index) {
            let extraction = document.extract_text_for_pages([(index, page)])?;
            if !extraction.warnings.is_empty() {
                return Err(PdfError::unsupported(
                    "text edits require complete supported text analysis for every affected page",
                ));
            }
            entry.insert(extraction.spans);
        }
        Ok(&self.source[&index])
    }

    fn replace(
        &mut self,
        page: ObjectRef,
        index: usize,
        reference: ObjectRef,
        span: Span,
        replacement: &str,
    ) -> Result<(), PdfError> {
        let source = &self.source[&index];
        let mut matches = source.iter().enumerate().filter(|(_, item)| {
            item.object_number == reference.number
                && item.generation == reference.generation
                && item.decoded_span == span
        });
        let (position, _) = matches.next().ok_or_else(|| {
            PdfError::unsupported("selected operand has no affected page occurrence")
        })?;
        if matches.next().is_some() {
            return Err(PdfError::unsupported(
                "text selection cannot disambiguate repeated physical operands on one page",
            ));
        }
        let expected = self
            .expected
            .entry(page)
            .or_insert_with(|| source.iter().map(TextMeaning::from).collect());
        expected[position].text = replacement.into();
        Ok(())
    }
}

impl PdfRevision {
    pub fn new(document: PdfDocument) -> Self {
        let snapshot = SnapshotId::new(document.source_id(), RevisionId::INITIAL);
        Self {
            state: RevisionState::new(snapshot, document),
        }
    }
    pub fn snapshot(&self) -> SnapshotId {
        self.state.snapshot()
    }
    pub fn inspect(&self) -> Result<InspectResult, PdfError> {
        self.state.value().inspect()
    }
    /// Text-only view: deliberately does not expose byte spans into original
    /// input, which need not describe staged stream payloads after a commit.
    pub fn text(&self) -> Result<Vec<String>, PdfError> {
        Ok(self
            .state
            .value()
            .with_cancellation(binas_core::CancellationToken::new())
            .extract_text_spans()?
            .spans
            .into_iter()
            .map(|span| span.text)
            .collect())
    }
    pub fn annotations(&self) -> Result<Vec<Annotation>, PdfError> {
        list_annotations(self.state.value())
    }
    /// Extract a committed page with the caller's cancellation token. One coarse
    /// admission unit is charged; PDF decode limits remain separate accounting.
    /// Source spans retain their original provenance after staged stream changes.
    pub fn extract_page_text_spans(
        &self,
        page_index: usize,
        context: &OperationContext,
    ) -> Result<crate::TextExtraction, PdfError> {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let extraction = self
            .state
            .value()
            .with_cancellation(context.cancellation().clone())
            .extract_page_text_spans(page_index)?;
        context.checkpoint().map_err(PdfError::from_context)?;
        Ok(extraction)
    }
    /// Stable indirect page identities in the current revision's physical order.
    pub fn page_ids(&self) -> Result<Vec<PdfReferenceTarget>, PdfError> {
        Ok(self
            .state
            .value()
            .page_refs()?
            .into_iter()
            .map(public_ref)
            .collect())
    }
    pub fn page_labels(&self) -> Result<Vec<crate::PageLabel>, PdfError> {
        crate::read_page_labels(self.state.value())
    }
    pub fn page_geometry(&self, page_index: usize) -> Result<crate::PageGeometry, PdfError> {
        self.state.value().page_geometry(page_index)
    }
    pub fn decode_metrics(&self) -> DecodeMetrics {
        self.state.value().decode_metrics()
    }

    /// Resource traversal bound to this committed revision, including its COW deltas.
    pub fn resource_usage(
        &self,
        context: &OperationContext,
    ) -> Result<crate::PdfResourceUsage, PdfError> {
        crate::resource_usage::census(self.state.value(), self.snapshot(), None, context)
    }

    pub fn page_resource_usage(
        &self,
        page_index: usize,
        context: &OperationContext,
    ) -> Result<crate::PdfResourceUsage, PdfError> {
        crate::resource_usage::census(
            self.state.value(),
            self.snapshot(),
            Some(page_index),
            context,
        )
    }

    pub fn plan(
        &self,
        request: PdfRevisionRequest,
        context: &OperationContext,
    ) -> Result<PdfRevisionPlan, PdfError> {
        context.checkpoint().map_err(PdfError::from_context)?;
        let document = self
            .state
            .value()
            .with_cancellation(context.cancellation().clone());
        let count = request
            .text
            .len()
            .checked_add(request.page_text.len())
            .and_then(|count| count.checked_add(request.annotations.len()))
            .and_then(|count| count.checked_add(request.checkboxes.len()))
            .and_then(|count| count.checked_add(request.flatten_fields.len()))
            .and_then(|count| count.checked_add(request.text_fields.len()))
            .and_then(|count| count.checked_add(request.page_transforms.len()))
            .and_then(|count| count.checked_add(usize::from(request.reorder_pages.is_some())))
            .and_then(|count| count.checked_add(usize::from(request.duplicate_page.is_some())))
            .and_then(|count| count.checked_add(usize::from(request.delete_page.is_some())))
            .ok_or_else(|| PdfError::limit("revision request count overflows"))?;
        if count == 0 {
            return Err(PdfError::unsafe_rewrite(
                "revision requires a nonempty edit batch",
            ));
        }
        if count > document.parsed().limits.max_container_items {
            return Err(PdfError::limit(
                "revision request count exceeds container limit",
            ));
        }
        context
            .charge_work(count as u64)
            .map_err(PdfError::from_context)?;
        if request.duplicate_page.is_some() && count != 1 {
            return Err(PdfError::unsupported(
                "page duplication requires a separate committed plan",
            ));
        }
        if request.delete_page.is_some() && count != 1 {
            return Err(PdfError::unsupported(
                "page deletion requires a separate committed plan",
            ));
        }
        if !request.checkboxes.is_empty() && !request.flatten_fields.is_empty() {
            return Err(PdfError::unsupported(
                "checkbox state changes and field flattening require separate committed plans",
            ));
        }
        if !request.text_fields.is_empty()
            && (!request.checkboxes.is_empty() || !request.flatten_fields.is_empty())
        {
            return Err(PdfError::unsupported(
                "text-field filling with checkbox changes or flattening requires separate committed plans",
            ));
        }
        refuse_security_boundaries_with_context(document.parsed(), Some(context))?;
        if !request.annotations.is_empty() {
            refuse_unsafe_interactive_edit(&document)?;
        }
        let index = PdfReferenceIndex::build_for_revision(&self.state, context)?;
        let pages = document.page_refs()?;
        context
            .charge_work(pages.len() as u64)
            .map_err(PdfError::from_context)?;
        let page_positions: BTreeMap<_, _> = pages
            .iter()
            .enumerate()
            .map(|(index, page)| (*page, index))
            .collect();
        // A None scope changes the existing object; Some(page) creates one
        // stream for that page. Commands for distinct local scopes can compose;
        // mixing local and global intent for the same source is a conflict.
        let mut changes = TextChanges::default();
        let mut text_inventory = TextInventory::default();
        if !request.text.is_empty() {
            let extraction = document.extract_text_spans()?;
            if !extraction.warnings.is_empty() {
                return Err(PdfError::unsupported(
                    "global text match indices require complete supported text analysis",
                ));
            }
            for span in extraction.spans {
                text_inventory
                    .source
                    .entry(span.page_index)
                    .or_default()
                    .push(span);
            }
        }
        for edit in request.text {
            context.checkpoint().map_err(PdfError::from_context)?;
            validate_text_request(&document, &edit, false)?;
            let selected = text_inventory
                .source
                .values()
                .flatten()
                .filter(|span| span.text == edit.old_text)
                .nth(edit.match_index)
                .cloned()
                .ok_or_else(|| PdfError::selection("revision text match not found"))?;
            require_raw_text(
                &selected.text,
                selected.font_name.as_deref(),
                selected.to_unicode,
                &edit,
            )?;
            let reference = ObjectRef {
                number: selected.object_number,
                generation: selected.generation,
            };
            let owners = direct_page_owners(
                &document,
                &index,
                self.snapshot(),
                reference,
                PageOwnedRole::Contents,
                context,
            )?;
            if owners.len() != 1 {
                return Err(PdfError::unsafe_rewrite(
                    "revision text requires one direct page owner",
                ));
            }
            let owner = owners[0];
            let owner_index = page_positions[&owner.page];
            text_inventory.page(&document, owner_index, owner.page)?;
            text_inventory.replace(
                owner.page,
                owner_index,
                reference,
                selected.decoded_span,
                &edit.replacement,
            )?;
            changes.push(
                reference,
                None,
                owners,
                PdfSharedEditMode::Reject,
                encode_text_change(
                    &document,
                    reference,
                    selected.decoded_span,
                    edit.replacement.as_bytes(),
                )?,
            )?;
        }
        for scoped in request.page_text {
            context.checkpoint().map_err(PdfError::from_context)?;
            validate_text_request(&document, &scoped.edit, true)?;
            let page = *pages
                .get(scoped.page_index)
                .ok_or_else(|| PdfError::selection("text page index is out of range"))?;
            let selected = text_inventory
                .page(&document, scoped.page_index, page)?
                .iter()
                .filter(|span| span.text == scoped.edit.old_text)
                .nth(scoped.edit.match_index)
                .cloned()
                .ok_or_else(|| PdfError::selection("text match not found on selected page"))?;
            let reference = ObjectRef {
                number: selected.object_number,
                generation: selected.generation,
            };
            let owners = direct_page_owners(
                &document,
                &index,
                self.snapshot(),
                reference,
                PageOwnedRole::Contents,
                context,
            )?;
            let selected_owners: Vec<_> = owners
                .iter()
                .filter(|owner| owner.page == page)
                .copied()
                .collect();
            if selected_owners.is_empty() {
                return Err(PdfError::unsafe_rewrite(
                    "text selection is not in a direct page stream",
                ));
            }
            let (scope, affected) = match scoped.shared {
                PdfSharedEditMode::Reject if owners.len() != 1 => {
                    return Err(PdfError::unsafe_rewrite(
                        "shared text requires an explicit edit mode",
                    ));
                }
                PdfSharedEditMode::CloneForPage if selected_owners.len() != 1 => {
                    return Err(PdfError::unsupported(
                        "page-local edit cannot disambiguate repeated stream slots",
                    ));
                }
                PdfSharedEditMode::CloneForPage if owners.len() > 1 => {
                    (Some(page), selected_owners)
                }
                _ => (None, owners),
            };
            // Updating all owners requires the same supported token meaning in
            // every page context. Shared bytes alone do not establish that.
            let mut encoded: Option<ScopedEncoding> = None;
            for owner in &affected {
                let owner_index = page_positions[&owner.page];
                let matching: Vec<_> = text_inventory
                    .page(&document, owner_index, owner.page)?
                    .iter()
                    .filter(|span| {
                        span.object_number == reference.number
                            && span.generation == reference.generation
                            && span.decoded_span == selected.decoded_span
                    })
                    .cloned()
                    .collect();
                if matching.len() != 1 || matching[0].text != scoped.edit.old_text {
                    return Err(PdfError::unsupported(
                        "shared stream token has ambiguous or different meaning in another owner",
                    ));
                }
                let owner_encoding =
                    encode_scoped_text(&document, owner.page, &matching[0], &scoped.edit, context)?;
                if encoded
                    .as_ref()
                    .is_some_and(|encoded| owner_encoding.bytes() != encoded.bytes())
                {
                    return Err(PdfError::unsupported(
                        "shared stream replacement requires different font codes in another owner",
                    ));
                }
                if let ScopedEncoding::Font(font) = &owner_encoding {
                    text_inventory.fonts.push(PdfFontTextEffect {
                        page: public_ref(owner.page),
                        source_stream: public_ref(reference),
                        font: font.font.map(public_ref),
                        cmap: public_ref(font.cmap),
                        encoded_bytes: font.bytes.len(),
                    });
                }
                text_inventory.replace(
                    owner.page,
                    owner_index,
                    reference,
                    selected.decoded_span,
                    &scoped.edit.replacement,
                )?;
                encoded.get_or_insert(owner_encoding);
            }
            changes.push(
                reference,
                scope,
                affected,
                scoped.shared,
                encode_text_change(
                    &document,
                    reference,
                    selected.decoded_span,
                    encoded
                        .as_ref()
                        .expect("proved at least one page owner")
                        .bytes(),
                )?,
            )?;
        }
        let mut intent = Intent {
            objects: BTreeMap::new(),
            streams: BTreeMap::new(),
            annotations: BTreeMap::new(),
            reorder: None,
            labels: None,
            text_pages: text_inventory.expected,
            font_effects: text_inventory.fonts,
            geometry: None,
            checkboxes: None,
            flatten: None,
            text_fields: None,
            duplicate: None,
            delete: None,
        };
        let mut reservations = Vec::new();
        let mut allocator = None;
        for ((source_reference, scope), mut stream_changes) in changes.streams {
            let reference = if scope.is_some() {
                if allocator.is_none() {
                    allocator = Some(ObjectAllocator::new(&document, context)?);
                }
                allocator
                    .as_mut()
                    .expect("initialized allocator")
                    .allocate(&document)?
            } else {
                source_reference
            };
            let edits = &mut stream_changes.edits;
            edits.sort_by_key(|edit| edit.span.start());
            if edits
                .windows(2)
                .any(|pair| pair[0].span.end() > pair[1].span.start())
            {
                return Err(PdfError::unsafe_rewrite("revision text spans overlap"));
            }
            let before = document.parsed().object(source_reference)?;
            let Value::Dict(dictionary) = &before.value else {
                return Err(PdfError::unsafe_rewrite("stream requires dictionary"));
            };
            if scope.is_some()
                && dictionary.keys().any(|key| {
                    !matches!(
                        key.as_slice(),
                        b"Length" | b"Filter" | b"DecodeParms" | b"DL"
                    )
                })
            {
                return Err(PdfError::unsupported(
                    "owner-local stream cloning requires known encoding dictionary keys",
                ));
            }
            if matches!(dictionary.get(b"Length".as_slice()), Some(Value::Ref(_))) {
                return Err(PdfError::unsupported(
                    "revision stream requires direct Length",
                ));
            }
            let mut budget = ParseBudget::for_document(document.parsed());
            let decoded = decode_stream(
                &before.value,
                before
                    .stream
                    .as_deref()
                    .ok_or_else(|| PdfError::unsafe_rewrite("text object is not a stream"))?,
                &document.parsed().limits,
                &mut budget,
            )?;
            let mut length = decoded.len();
            for edit in edits.iter() {
                length = length
                    .checked_sub(edit.span.len() as usize)
                    .and_then(|n| n.checked_add(edit.replacement.len()))
                    .ok_or_else(|| PdfError::limit("revision stream length overflows"))?;
            }
            if length > document.parsed().limits.max_stream_bytes {
                return Err(PdfError::limit("revision stream exceeds max_stream_bytes"));
            }
            reservations.push(
                context
                    .reserve(length as u64)
                    .map_err(PdfError::from_context)?,
            );
            context
                .charge_work(length as u64)
                .map_err(PdfError::from_context)?;
            let mut expected = Vec::new();
            expected
                .try_reserve_exact(length)
                .map_err(|_| PdfError::limit("revision stream allocation failed"))?;
            let mut cursor = 0;
            for edit in edits.iter() {
                expected.extend_from_slice(&decoded[cursor..edit.span.start() as usize]);
                expected.extend_from_slice(&edit.replacement);
                cursor = edit.span.end() as usize;
            }
            expected.extend_from_slice(&decoded[cursor..]);
            let encoded = encode_pdf_stream(&before.value, &expected, &document.parsed().limits)?;
            let mut after = before.clone();
            let Value::Dict(dictionary) = &mut after.value else {
                unreachable!()
            };
            dictionary.insert(
                b"Length".to_vec(),
                Value::Integer(
                    i64::try_from(encoded.len())
                        .map_err(|_| PdfError::limit("encoded length exceeds i64"))?,
                ),
            );
            match dictionary.get(b"DL".as_slice()) {
                None | Some(Value::Null) => {}
                Some(Value::Integer(value)) if *value >= 0 => {
                    dictionary.insert(
                        b"DL".to_vec(),
                        Value::Integer(
                            i64::try_from(expected.len())
                                .map_err(|_| PdfError::limit("decoded length exceeds i64"))?,
                        ),
                    );
                }
                _ => {
                    return Err(PdfError::unsafe_rewrite(
                        "decoded length hint requires a nonnegative integer",
                    ));
                }
            }
            after.stream = Some(encoded.into());
            if let Some(page) = scope {
                after.offset = 0;
                after.stream_offset = 0;
                replace_page_content_slot(
                    &document,
                    &mut intent.objects,
                    page,
                    stream_changes.owners[0].slot,
                    source_reference,
                    reference,
                )?;
            }
            intent.objects.insert(reference, after);
            let mut affected_pages = Vec::new();
            for owner in &stream_changes.owners {
                let page = public_ref(owner.page);
                if affected_pages.last() != Some(&page) {
                    affected_pages.push(page);
                }
            }
            intent.streams.insert(
                reference,
                StreamIntent {
                    expected,
                    owners: stream_changes.owners,
                    effect: PdfStreamEditEffect {
                        source_stream: public_ref(source_reference),
                        output_stream: public_ref(reference),
                        pages: affected_pages,
                        shared: stream_changes.shared,
                    },
                },
            );
        }
        let annotations = if request.annotations.is_empty() {
            Vec::new()
        } else {
            list_annotations(&document)?
        };
        for edit in request.annotations {
            context.checkpoint().map_err(PdfError::from_context)?;
            let selected = annotations
                .get(edit.annotation_index)
                .ok_or_else(|| PdfError::selection("revision annotation selection not found"))?;
            if selected.subtype != "Text" {
                return Err(PdfError::unsupported(
                    "revision annotations require Text subtype",
                ));
            }
            let reference = match (selected.object_number, selected.object_generation) {
                (Some(number), Some(generation)) => ObjectRef { number, generation },
                _ => {
                    return Err(PdfError::unsupported(
                        "revision annotation requires indirect dictionary",
                    ));
                }
            };
            let owners = direct_page_owners(
                &document,
                &index,
                self.snapshot(),
                reference,
                PageOwnedRole::Annots,
                context,
            )?;
            if owners.len() != 1 {
                return Err(PdfError::unsafe_rewrite(
                    "annotation requires one direct page owner",
                ));
            }
            let page_owner = owners[0].page;
            if intent.objects.contains_key(&reference) {
                return Err(PdfError::unsafe_rewrite(
                    "revision changes conflict on annotation object",
                ));
            }
            let before = document.parsed().object(reference)?;
            let Value::Dict(dictionary) = &before.value else {
                return Err(PdfError::unsafe_rewrite("annotation requires dictionary"));
            };
            if dictionary
                .get(b"P".as_slice())
                .is_some_and(|value| value != &Value::Ref(page_owner))
            {
                return Err(PdfError::unsafe_rewrite(
                    "annotation P contradicts its direct page owner",
                ));
            }
            if before.stream.is_some() || dictionary.contains_key(b"AP".as_slice()) {
                return Err(PdfError::unsupported(
                    "revision annotation with stream or appearance is unsupported",
                ));
            }
            if edit.contents.len() > document.parsed().limits.max_token_bytes {
                return Err(PdfError::limit("annotation input exceeds max_token_bytes"));
            }
            let contents = encode_text(&edit.contents);
            if contents.len() > document.parsed().limits.max_token_bytes {
                return Err(PdfError::limit(
                    "annotation contents exceeds max_token_bytes",
                ));
            }
            let mut after = before.clone();
            let Value::Dict(dictionary) = &mut after.value else {
                unreachable!()
            };
            dictionary.insert(b"Contents".to_vec(), Value::String(contents.clone()));
            intent.objects.insert(reference, after);
            intent.annotations.insert(reference, contents);
        }
        if let Some(order) = request.reorder_pages {
            if order.len() != pages.len()
                || order.len() > document.parsed().limits.max_container_items
            {
                return Err(PdfError::selection(
                    "reorder requires every current page exactly once",
                ));
            }
            context
                .charge_work(order.len() as u64)
                .map_err(PdfError::from_context)?;
            let requested = order
                .iter()
                .map(|index| {
                    pages
                        .get(*index)
                        .copied()
                        .ok_or_else(|| PdfError::selection("reorder page index is out of range"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut reorder = pages::plan_reorder(&document, &requested, context)?;
            let mut labels = labels::plan_reordered_labels(&document, &order, context)?;
            for (reference, object) in std::mem::take(&mut reorder.replacements) {
                merge_page_changes(&document, &mut intent.objects, reference, object)?;
            }
            for (reference, object) in std::mem::take(&mut labels.changes) {
                if intent.objects.insert(reference, object).is_some() {
                    return Err(PdfError::unsafe_rewrite(
                        "revision operations conflict on one object",
                    ));
                }
            }
            intent.reorder = Some(reorder);
            intent.labels = Some(labels);
        }
        if !request.page_transforms.is_empty() {
            // Compile intent from the source revision. Apply geometry last so
            // explicit requests override only reorder's synthesized page keys,
            // while selections continue to identify their original stable pages.
            let mut geometry = geometry::plan(&document, &request.page_transforms, context)?;
            geometry::apply(&mut geometry, &document, &mut intent.objects, context)?;
            intent.geometry = Some(geometry);
        }
        if !request.checkboxes.is_empty() {
            let checkboxes = checkbox::plan(
                &document,
                self.snapshot(),
                &index,
                &request.checkboxes,
                context,
            )?;
            checkbox::apply(&document, &checkboxes, &mut intent.objects, context)?;
            intent.checkboxes = Some(checkboxes);
        }
        if !request.flatten_fields.is_empty() {
            let flatten = flatten::plan(
                &document,
                self.snapshot(),
                &index,
                &request.flatten_fields,
                &intent.objects,
                context,
            )?;
            // Flattening adds appearance text to page content. The existing
            // text obligation covers source runs plus token replacements only.
            for stream in intent.streams.values() {
                for owner in &stream.owners {
                    context.charge_work(1).map_err(PdfError::from_context)?;
                    if flatten.allows_page_key(owner.page, b"Contents") {
                        return Err(PdfError::unsupported(
                            "text edits and field flattening on the same page require separate committed plans",
                        ));
                    }
                }
            }
            flatten::apply(&document, &flatten, &mut intent.objects, context)?;
            intent.flatten = Some(flatten);
        }
        if !request.text_fields.is_empty() {
            let text_fields = text_fields::plan(
                &document,
                self.snapshot(),
                &index,
                &request.text_fields,
                &intent.objects,
                context,
            )?;
            text_fields::apply(&document, &text_fields, &mut intent.objects, context)?;
            intent.text_fields = Some(text_fields);
        }
        if let Some(request) = request.duplicate_page {
            let duplicate = duplicate::plan(
                &document,
                self.snapshot(),
                &index,
                &request,
                &intent.objects,
                context,
            )?;
            duplicate::apply(&document, &duplicate, &mut intent.objects, context)?;
            let mut labels =
                labels::plan_occurrence_labels(&document, duplicate.source_indices(), context)?;
            for (reference, object) in std::mem::take(&mut labels.changes) {
                // A direct PageLabels dictionary can share the catalog with
                // direct AcroForm registration. Merge disjoint source keys;
                // each family independently checks its exact permitted effects.
                merge_catalog_labels(
                    &document,
                    &mut intent.objects,
                    reference,
                    object,
                    &[b"AcroForm"],
                    context,
                )?;
            }
            intent.labels = Some(labels);
            intent.duplicate = Some(duplicate);
        }
        if let Some(request) = request.delete_page {
            let delete = delete::plan(
                &document,
                self.snapshot(),
                &index,
                &request,
                &intent.objects,
                context,
            )?;
            delete::apply(&document, &delete, &mut intent.objects, context)?;
            let mut labels =
                labels::plan_deleted_labels(&document, delete.source_indices(), context)?;
            let catalog_keys: Vec<_> = delete.catalog_keys().collect();
            for (reference, object) in std::mem::take(&mut labels.changes) {
                merge_catalog_labels(
                    &document,
                    &mut intent.objects,
                    reference,
                    object,
                    &catalog_keys,
                    context,
                )?;
            }
            intent.labels = Some(labels);
            intent.delete = Some(delete);
        }
        context.checkpoint().map_err(PdfError::from_context)?;
        let checks = required(&intent);
        Ok(PdfRevisionPlan {
            envelope: PlanEnvelope::new(self.snapshot(), intent, checks),
            store: self.state.value().parsed().objects.clone(),
            _decoded_reservations: reservations,
        })
    }

    pub fn commit(
        &mut self,
        plan: &PdfRevisionPlan,
        context: &OperationContext,
    ) -> Result<PdfRevisionReceipt, TransactionError<PdfError>> {
        if plan.snapshot() != self.snapshot() {
            return Err(TransactionError::StalePlan {
                planned: plan.snapshot(),
                current: self.snapshot(),
            });
        }
        if !self
            .state
            .value()
            .parsed()
            .objects
            .shares_snapshot_with(&plan.store)
        {
            return Err(TransactionError::Stage(PdfError::unsafe_rewrite(
                "revision plan object-store snapshot mismatch",
            )));
        }
        execute_plan(
            &mut self.state,
            &plan.envelope,
            context,
            |original, intent, context| {
                let mut parsed = original.parsed().clone();
                parsed.decode_cancellation = context.cancellation().clone();
                for (reference, object) in &intent.objects {
                    context.charge_work(1).map_err(PdfError::from_context)?;
                    parsed.objects.insert(*reference, object.clone());
                }
                Ok(StagedChange {
                    candidate: original.with_parsed(parsed),
                    changes: intent
                        .objects
                        .keys()
                        .map(|reference| public_ref(*reference))
                        .collect(),
                })
            },
            |original, candidate, intent, _, context| {
                verify_candidate(original, candidate, intent, context)
            },
        )
    }

    /// Plans a bounded Flate-to-Flate storage change against the current revision.
    /// Stream savings are not a claim about the final exported PDF size.
    pub fn plan_flate_recompression(
        &self,
        context: &OperationContext,
    ) -> Result<crate::PdfFlatePlan, PdfError> {
        crate::optimize::structure::plan(self.state.value(), self.snapshot(), context)
    }

    /// Publishes checked stream changes once; an empty plan creates no revision.
    pub fn commit_flate_recompression(
        &mut self,
        plan: &crate::PdfFlatePlan,
        context: &OperationContext,
    ) -> Result<Option<crate::PdfFlateReceipt>, TransactionError<PdfError>> {
        crate::optimize::structure::commit(&mut self.state, plan, context)
    }

    /// Plans exact-sample PNG Up storage for supported Flate image XObjects.
    /// Preserved mask/color dictionaries and stream savings do not establish
    /// final PDF size, appearance coverage or standards conformance.
    pub fn plan_lossless_images(
        &self,
        context: &OperationContext,
    ) -> Result<crate::PdfLosslessImagePlan, PdfError> {
        crate::optimize::images::plan(self.state.value(), self.snapshot(), context)
    }

    /// Publishes checked image storage changes once; an empty plan is a no-op.
    pub fn commit_lossless_images(
        &mut self,
        plan: &crate::PdfLosslessImagePlan,
        context: &OperationContext,
    ) -> Result<Option<crate::PdfLosslessImageReceipt>, TransactionError<PdfError>> {
        crate::optimize::images::commit(&mut self.state, plan, context)
    }

    pub fn export(
        &self,
        mode: crate::PdfWriterMode,
        context: &OperationContext,
    ) -> Result<crate::PdfRevisionExport, PdfError> {
        crate::writer::export_staged_document(self.state.value(), self.snapshot(), mode, context)
    }
}

fn public_ref(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}

type TextScope = (ObjectRef, Option<ObjectRef>);
#[derive(Default)]
struct TextChanges {
    streams: BTreeMap<TextScope, ScopedTextChanges>,
    local: BTreeMap<ObjectRef, bool>,
}
struct ScopedTextChanges {
    edits: Vec<TextChange>,
    owners: Vec<PageOwnedReference>,
    shared: PdfSharedEditMode,
}
impl TextChanges {
    fn push(
        &mut self,
        source: ObjectRef,
        scope: Option<ObjectRef>,
        owners: Vec<PageOwnedReference>,
        shared: PdfSharedEditMode,
        change: TextChange,
    ) -> Result<(), PdfError> {
        if let Some(local) = self.local.insert(source, scope.is_some())
            && local != scope.is_some()
        {
            return Err(PdfError::unsafe_rewrite(
                "local and global edits conflict on a shared stream",
            ));
        }
        let changes = self
            .streams
            .entry((source, scope))
            .or_insert_with(|| ScopedTextChanges {
                edits: Vec::new(),
                owners: owners.clone(),
                shared,
            });
        if changes.owners != owners || changes.shared != shared {
            return Err(PdfError::unsafe_rewrite(
                "text commands disagree on shared owner semantics",
            ));
        }
        changes.edits.push(change);
        Ok(())
    }
}

fn validate_text_request(
    document: &PdfDocument,
    edit: &SurgicalTextEditRequest,
    allow_unicode: bool,
) -> Result<(), PdfError> {
    if edit.old_text.is_empty()
        || edit.replacement.is_empty()
        || (!allow_unicode && (!edit.old_text.is_ascii() || !edit.replacement.is_ascii()))
    {
        return Err(PdfError::unsafe_rewrite(
            "revision text requires nonempty whole tokens; legacy requests require ASCII",
        ));
    }
    if edit.old_text.len() > document.parsed().limits.max_token_bytes
        || edit.replacement.len() > document.parsed().limits.max_token_bytes
    {
        return Err(PdfError::limit(
            "revision replacement exceeds max_token_bytes",
        ));
    }
    Ok(())
}

enum ScopedEncoding {
    Raw(Vec<u8>),
    Font(EncodedFontText),
}
impl ScopedEncoding {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Raw(bytes) => bytes,
            Self::Font(font) => &font.bytes,
        }
    }
}

fn encode_scoped_text(
    document: &PdfDocument,
    page: ObjectRef,
    selected: &TextSpan,
    edit: &SurgicalTextEditRequest,
    context: &OperationContext,
) -> Result<ScopedEncoding, PdfError> {
    if selected.text != edit.old_text {
        return Err(PdfError::unsupported(
            "selected token has different Unicode meaning",
        ));
    }
    match (&selected.font_name, selected.to_unicode) {
        (Some(name), true) => {
            encode_existing_cmap_text(document, page, name.as_bytes(), &edit.replacement, context)
                .map(ScopedEncoding::Font)
        }
        (None, false) if edit.old_text.is_ascii() && edit.replacement.is_ascii() => {
            Ok(ScopedEncoding::Raw(edit.replacement.as_bytes().to_vec()))
        }
        _ => Err(PdfError::unsupported(
            "page text requires an explicit ToUnicode font mapping or an ASCII token without font interpretation",
        )),
    }
}

fn require_raw_text(
    text: &str,
    font: Option<&str>,
    to_unicode: bool,
    edit: &SurgicalTextEditRequest,
) -> Result<(), PdfError> {
    if text != edit.old_text || font.is_some() || to_unicode {
        return Err(PdfError::unsupported(
            "revision text requires a complete token without font interpretation",
        ));
    }
    Ok(())
}

fn encode_text_change(
    document: &PdfDocument,
    reference: ObjectRef,
    span: Span,
    encoded_text: &[u8],
) -> Result<TextChange, PdfError> {
    let object = document.parsed().object(reference)?;
    let mut budget = ParseBudget::for_document(document.parsed());
    let decoded = decode_stream(
        &object.value,
        object
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::unsafe_rewrite("text object is not a stream"))?,
        &document.parsed().limits,
        &mut budget,
    )?;
    let token = span
        .slice(&decoded)
        .ok_or_else(|| PdfError::unsafe_rewrite("selected token outside decoded stream"))?;
    let replacement = match token {
        [b'(', .., b')'] => encode_literal(encoded_text),
        [b'<', .., b'>'] => encode_hex(encoded_text),
        _ => {
            return Err(PdfError::unsupported(
                "revision text requires literal or hex token",
            ));
        }
    };
    if replacement.len() > document.parsed().limits.max_token_bytes {
        return Err(PdfError::limit(
            "encoded revision token exceeds max_token_bytes",
        ));
    }
    Ok(TextChange { span, replacement })
}

/// Never reuse free numbers or an earlier generation. `source_size` is the
/// immutable allocation floor; subsequent commits also consider staged objects.
struct ObjectAllocator {
    next: u64,
    allocated: usize,
}
impl ObjectAllocator {
    fn new(document: &PdfDocument, context: &OperationContext) -> Result<Self, PdfError> {
        let mut next = document.parsed().source_size.unwrap_or(1).max(1) as u64;
        for reference in document
            .parsed()
            .objects
            .keys()
            .chain(document.parsed().objects.base_keys())
        {
            context.charge_work(1).map_err(PdfError::from_context)?;
            next = next.max(u64::from(reference.number) + 1);
        }
        Ok(Self { next, allocated: 0 })
    }
    fn allocate(&mut self, document: &PdfDocument) -> Result<ObjectRef, PdfError> {
        if document.parsed().source_size.is_none_or(|size| size == 0) {
            return Err(PdfError::unsupported(
                "new revision objects require an original direct trailer Size",
            ));
        }
        let count = document
            .parsed()
            .objects
            .len()
            .checked_add(self.allocated)
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| PdfError::limit("revision object count overflows"))?;
        if count > document.parsed().limits.max_objects
            || self.next >= document.parsed().limits.max_xref_entries as u64
        {
            return Err(PdfError::limit(
                "revision object allocation exceeds object/xref limits",
            ));
        }
        let number = u32::try_from(self.next)
            .map_err(|_| PdfError::limit("revision object number exceeds u32"))?;
        self.next += 1;
        self.allocated += 1;
        Ok(ObjectRef {
            number,
            generation: 0,
        })
    }
}

fn replace_page_content_slot(
    document: &PdfDocument,
    objects: &mut BTreeMap<ObjectRef, IndirectObject>,
    page: ObjectRef,
    slot: PageOwnedSlot,
    source: ObjectRef,
    output: ObjectRef,
) -> Result<(), PdfError> {
    if let std::collections::btree_map::Entry::Vacant(entry) = objects.entry(page) {
        entry.insert(document.parsed().object(page)?.clone());
    }
    let object = objects.get_mut(&page).expect("inserted page");
    let Value::Dict(dictionary) = &mut object.value else {
        return Err(PdfError::unsafe_rewrite("page requires a dictionary"));
    };
    let value = match (slot, dictionary.get_mut(b"Contents".as_slice())) {
        (PageOwnedSlot::Scalar, value) => value,
        (PageOwnedSlot::ArrayIndex(index), Some(Value::Array(values))) => values.get_mut(index),
        _ => None,
    }
    .ok_or_else(|| PdfError::unsafe_rewrite("page Contents slot disappeared"))?;
    if value != &Value::Ref(source) {
        return Err(PdfError::unsafe_rewrite("page Contents edits conflict"));
    }
    *value = Value::Ref(output);
    Ok(())
}

/// Compose page Contents changes with reorder's independent dictionary keys.
/// Other overlapping object roles remain a conflict, even for equal values.
fn merge_page_changes(
    document: &PdfDocument,
    objects: &mut BTreeMap<ObjectRef, IndirectObject>,
    reference: ObjectRef,
    incoming: IndirectObject,
) -> Result<(), PdfError> {
    let Some(current) = objects.get_mut(&reference) else {
        objects.insert(reference, incoming);
        return Ok(());
    };
    let before = document.parsed().object(reference)?;
    let (Value::Dict(base), Value::Dict(current_dict), Value::Dict(incoming_dict)) =
        (&before.value, &mut current.value, &incoming.value)
    else {
        return Err(PdfError::unsafe_rewrite(
            "revision operations conflict on one object",
        ));
    };
    if base.get(b"Type".as_slice()) != Some(&Value::Name(b"Page".to_vec()))
        || before.stream.is_some()
        || current.stream.is_some()
        || incoming.stream.is_some()
    {
        return Err(PdfError::unsafe_rewrite(
            "only disjoint page dictionary changes can compose",
        ));
    }
    let all_keys: BTreeSet<_> = base
        .keys()
        .chain(current_dict.keys())
        .chain(incoming_dict.keys())
        .cloned()
        .collect();
    for key in all_keys {
        let original = base.get(&key);
        if current_dict.get(&key) != original && key != b"Contents" {
            return Err(PdfError::unsafe_rewrite(
                "unexpected page change before reorder",
            ));
        }
        if incoming_dict.get(&key) == original {
            continue;
        }
        if current_dict.get(&key) != original {
            return Err(PdfError::unsafe_rewrite(
                "revision operations change the same page key",
            ));
        }
        match incoming_dict.get(&key) {
            Some(value) => {
                current_dict.insert(key, value.clone());
            }
            None => {
                current_dict.remove(&key);
            }
        }
    }
    Ok(())
}

fn merge_catalog_labels(
    document: &PdfDocument,
    objects: &mut BTreeMap<ObjectRef, IndirectObject>,
    reference: ObjectRef,
    incoming: IndirectObject,
    current_keys: &[&[u8]],
    context: &OperationContext,
) -> Result<(), PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    let Some(current) = objects.get_mut(&reference) else {
        objects.insert(reference, incoming);
        return Ok(());
    };
    let Value::Dict(trailer) = &document.parsed().trailer else {
        return Err(PdfError::unsafe_rewrite("label merge requires a catalog"));
    };
    let before = document.parsed().object(reference)?;
    let (Value::Dict(base), Value::Dict(current_dict), Value::Dict(incoming_dict)) =
        (&before.value, &mut current.value, &incoming.value)
    else {
        return Err(PdfError::unsafe_rewrite(
            "label merge requires dictionaries",
        ));
    };
    if trailer.get(b"Root".as_slice()) != Some(&Value::Ref(reference))
        || before.stream.is_some()
        || current.stream.is_some()
        || incoming.stream.is_some()
    {
        return Err(PdfError::unsafe_rewrite(
            "page operation and label changes conflict",
        ));
    }
    for (actual, permitted) in [
        (&*current_dict, current_keys),
        (incoming_dict, &[b"PageLabels".as_slice()]),
    ] {
        let mut left = base
            .iter()
            .filter(|(key, _)| !permitted.contains(&key.as_slice()));
        let mut right = actual
            .iter()
            .filter(|(key, _)| !permitted.contains(&key.as_slice()));
        loop {
            match (left.next(), right.next()) {
                (None, None) => break,
                (Some((left_key, left)), Some((right_key, right))) => {
                    context
                        .charge_work((left_key.len() + right_key.len()) as u64 + 1)
                        .map_err(PdfError::from_context)?;
                    if left_key == right_key
                        && page_keys::value_equal(
                            left,
                            right,
                            0,
                            document.parsed().limits.max_parser_depth,
                            context,
                        )?
                    {
                        continue;
                    }
                }
                _ => {}
            }
            return Err(PdfError::unsafe_rewrite(
                "page operation and label changes conflict",
            ));
        }
    }
    match incoming_dict.get(b"PageLabels".as_slice()) {
        Some(value) => {
            current_dict.insert(b"PageLabels".to_vec(), value.clone());
        }
        None => {
            current_dict.remove(b"PageLabels".as_slice());
        }
    }
    Ok(())
}

const CHECKS: [&str; 9] = [
    "expected_changes",
    "untouched_objects",
    "page_relationships",
    "requested_payloads",
    "page_dictionary_effects",
    "annotation_dictionary_effects",
    "stream_dictionary_effects",
    "authorized_objects",
    "catalog_dictionary_effects",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.revision", name).expect("static revision code")
}
fn required(intent: &Intent) -> RequiredChecks {
    let mut codes: Vec<_> = CHECKS.iter().map(|name| code(name)).collect();
    if !intent.streams.is_empty() {
        codes.push(code("stream_owners"));
    }
    if !intent.text_pages.is_empty() {
        codes.push(code("requested_text"));
    }
    if intent.reorder.is_some() {
        codes.extend(pages::required_checks().codes().cloned());
    }
    if intent.labels.is_some() {
        codes.push(code("page_labels"));
    }
    if intent.geometry.is_some() {
        codes.extend(geometry::required_checks().codes().cloned());
    }
    if intent.checkboxes.is_some() {
        codes.extend(checkbox::required_checks().codes().cloned());
    }
    if intent.flatten.is_some() {
        codes.extend(flatten::required_checks().codes().cloned());
    }
    if intent.text_fields.is_some() {
        codes.extend(text_fields::required_checks().codes().cloned());
    }
    if intent.duplicate.is_some() {
        codes.extend(duplicate::required_checks().codes().cloned());
    }
    if intent.delete.is_some() {
        codes.extend(delete::required_checks().codes().cloned());
    }
    RequiredChecks::new(codes).expect("static disjoint obligations")
}
fn same_object(left: &IndirectObject, right: &IndirectObject) -> bool {
    left.value == right.value
        && left.stream.as_deref() == right.stream.as_deref()
        && left.offset == right.offset
        && left.stream_offset == right.stream_offset
}
fn verify_candidate(
    original: &PdfDocument,
    candidate: &PdfDocument,
    intent: &Intent,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    let additions = intent
        .objects
        .keys()
        .filter(|reference| !original.parsed().objects.contains_key(reference))
        .count();
    let expected_count = original
        .parsed()
        .objects
        .len()
        .checked_add(additions)
        .ok_or_else(|| PdfError::limit("candidate object count overflows"))?;
    let mut untouched = expected_count == candidate.parsed().objects.len()
        && original.parsed().trailer == candidate.parsed().trailer
        && original.parsed().version == candidate.parsed().version
        && original.parsed().xref_revisions == candidate.parsed().xref_revisions
        && original.parsed().source_size == candidate.parsed().source_size
        && original.source_id() == candidate.source_id();
    let mut changed = true;
    let changed_refs: BTreeSet<_> = intent.objects.keys().copied().collect();
    for (reference, before) in &original.parsed().objects {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let after = candidate.parsed().object(*reference)?;
        if changed_refs.contains(reference) {
            changed &= same_object(after, &intent.objects[reference]);
        } else {
            untouched &= std::ptr::eq(before, after) && same_object(before, after);
        }
    }
    for (reference, expected) in &intent.objects {
        context.charge_work(1).map_err(PdfError::from_context)?;
        changed &= same_object(candidate.parsed().object(*reference)?, expected);
        if !original.parsed().objects.contains_key(reference) {
            changed &= reference.generation == 0
                && original
                    .parsed()
                    .source_size
                    .is_some_and(|size| u64::from(reference.number) >= size as u64);
        }
    }
    changed &= expected_count <= candidate.parsed().limits.max_objects;
    let mut payloads = true;
    let mut owners_match = true;
    let candidate_index = if intent.streams.is_empty() {
        None
    } else {
        Some(PdfReferenceIndex::build(candidate, context)?)
    };
    let mut budget = ParseBudget::for_document(candidate.parsed());
    for (reference, stream) in &intent.streams {
        context.checkpoint().map_err(PdfError::from_context)?;
        let object = candidate.parsed().object(*reference)?;
        let decoded = decode_stream(
            &object.value,
            object
                .stream
                .as_deref()
                .ok_or_else(|| PdfError::verification("candidate stream missing"))?,
            &candidate.parsed().limits,
            &mut budget,
        )?;
        payloads &= &*decoded == stream.expected.as_slice();
        let index = candidate_index.as_ref().expect("stream ownership index");
        let mut actual = direct_page_owners(
            candidate,
            index,
            index.snapshot(),
            *reference,
            PageOwnedRole::Contents,
            context,
        )?;
        let mut expected = stream.owners.clone();
        actual.sort_unstable_by_key(|owner| (owner.page, owner.slot));
        expected.sort_unstable_by_key(|owner| (owner.page, owner.slot));
        owners_match &= actual == expected;
    }
    for (reference, expected) in &intent.annotations {
        let object = candidate.parsed().object(*reference)?;
        payloads &= matches!(&object.value, Value::Dict(dictionary) if matches!(dictionary.get(b"Contents".as_slice()), Some(Value::String(bytes)) if bytes == expected));
    }
    let expected_pages = match (&intent.reorder, &intent.duplicate, &intent.delete) {
        (Some(reorder), None, None) => reorder.expected_order.clone(),
        (None, Some(duplicate), None) => duplicate.expected_order().to_vec(),
        (None, None, Some(delete)) => delete.expected_order().to_vec(),
        (None, None, None) => original.page_refs()?,
        _ => {
            return Err(PdfError::verification(
                "conflicting page occurrence intents",
            ));
        }
    };
    let page_order = expected_pages == candidate.page_refs()?;
    let page_keys_match = page_keys::verify(original, candidate, intent, context)?;
    let annotation_keys_match =
        annotation_keys::verify(original, candidate, &intent.annotations, context)?;
    let stream_keys_match = stream_keys::verify(original, candidate, &intent.streams, context)?;
    let authorized_objects = object_roles::verify(original, candidate, intent, context)?;
    let catalog_keys_match = catalog_keys::verify(original, candidate, intent, context)?;
    let mut checks: Vec<_> = CHECKS
        .into_iter()
        .zip([
            changed,
            untouched,
            page_order,
            payloads,
            page_keys_match,
            annotation_keys_match,
            stream_keys_match,
            authorized_objects,
            catalog_keys_match,
        ])
        .map(|(name, passed)| CheckEvidence {
            code: code(name),
            status: if passed {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            },
            detail:
                "Scoped requested graph/payload checks; appearance and conformance not evaluated"
                    .into(),
        })
        .collect();
    if !intent.streams.is_empty() {
        checks.push(CheckEvidence { code: code("stream_owners"),
            status: if owners_match { CheckStatus::Passed } else { CheckStatus::Failed },
            detail: "Changed streams remain in precisely the requested direct page slots; local clones leave source streams covered by untouched-object checks.".into(),
        });
    }
    if !intent.text_pages.is_empty() {
        let mut text_matches = true;
        for (page, expected) in &intent.text_pages {
            context.checkpoint().map_err(PdfError::from_context)?;
            // The physical page index can change in the same transaction. Page
            // identities and the source-derived expected inventory stay stable.
            let position = expected_pages
                .iter()
                .position(|value| value == page)
                .ok_or_else(|| PdfError::verification("requested text page is missing"))?;
            let extraction = candidate.extract_text_for_pages([(position, *page)])?;
            context
                .charge_work(extraction.spans.len() as u64)
                .map_err(PdfError::from_context)?;
            text_matches &= extraction.warnings.is_empty()
                && extraction.spans.len() == expected.len()
                && extraction
                    .spans
                    .iter()
                    .map(TextMeaning::from)
                    .eq(expected.iter().cloned());
        }
        checks.push(CheckEvidence {
            code: code("requested_text"),
            status: if text_matches { CheckStatus::Passed } else { CheckStatus::Failed },
            detail: "Every supported text run on each affected stable page matches source meaning plus requested whole-token replacements. Font names, ToUnicode use and operators are retained; glyph programs, metrics, appearance and conformance are not evaluated.".into(),
        });
    }
    if let Some(reorder) = &intent.reorder {
        checks.extend_from_slice(
            pages::verify_reorder(
                original,
                candidate,
                reorder,
                &intent.objects,
                intent.geometry.as_ref(),
                intent.flatten.as_ref(),
                context,
            )?
            .checks(),
        );
    }
    if let Some(geometry) = &intent.geometry {
        checks.extend_from_slice(
            geometry::verify(original, candidate, geometry, &intent.objects, context)?.checks(),
        );
    }
    if let Some(checkboxes) = &intent.checkboxes {
        checks.extend_from_slice(
            checkbox::verify(original, candidate, checkboxes, context)?.checks(),
        );
    }
    if let Some(flatten) = &intent.flatten {
        checks.extend_from_slice(flatten::verify(original, candidate, flatten, context)?.checks());
    }
    if let Some(text_fields) = &intent.text_fields {
        checks.extend_from_slice(
            text_fields::verify(original, candidate, text_fields, context)?.checks(),
        );
    }
    if let Some(duplicate) = &intent.duplicate {
        checks.extend_from_slice(
            duplicate::verify(original, candidate, duplicate, context)?.checks(),
        );
    }
    if let Some(delete) = &intent.delete {
        checks.extend_from_slice(delete::verify(original, candidate, delete, context)?.checks());
    }
    if let Some(labels) = &intent.labels {
        context
            .charge_work(
                candidate.parsed().objects.len() as u64 + original.parsed().objects.len() as u64,
            )
            .map_err(PdfError::from_context)?;
        let mapping_bytes = expected_pages
            .len()
            .checked_mul(
                std::mem::size_of::<ObjectRef>()
                    + std::mem::size_of::<(ObjectRef, usize)>()
                    + std::mem::size_of::<usize>(),
            )
            .ok_or_else(|| PdfError::limit("label verification page mapping overflows"))?;
        let _mapping = context
            .reserve(mapping_bytes as u64)
            .map_err(PdfError::from_context)?;
        let old_indices = if let Some(duplicate) = &intent.duplicate {
            // The duplication checker independently derives this map from its
            // selected source page and fresh output page identity.
            context
                .charge_work(duplicate.source_indices().len() as u64)
                .map_err(PdfError::from_context)?;
            duplicate.source_indices().to_vec()
        } else if let Some(delete) = &intent.delete {
            // The deletion checker independently derives the retained mapping.
            context
                .charge_work(delete.source_indices().len() as u64)
                .map_err(PdfError::from_context)?;
            delete.source_indices().to_vec()
        } else {
            let original_pages = original.page_refs()?;
            if original_pages.len() != expected_pages.len() {
                return Err(PdfError::verification(
                    "label verification requires the original page identities",
                ));
            }
            context
                .charge_work(original_pages.len() as u64)
                .map_err(PdfError::from_context)?;
            let positions: BTreeMap<_, _> = original_pages
                .into_iter()
                .enumerate()
                .map(|(index, page)| (page, index))
                .collect();
            let mut old_indices = Vec::with_capacity(expected_pages.len());
            for page in &expected_pages {
                context.charge_work(1).map_err(PdfError::from_context)?;
                old_indices.push(*positions.get(page).ok_or_else(|| {
                    PdfError::verification("label verification contains a foreign page")
                })?);
            }
            old_indices
        };
        let raw_values = if let Some(duplicate) = &intent.duplicate {
            labels::verify_occurrence_mapping(
                original,
                candidate,
                &old_indices,
                duplicate.changes_catalog_acroform(),
                context,
            )?
        } else if let Some(delete) = &intent.delete {
            let catalog_keys: Vec<_> = delete.catalog_keys().collect();
            labels::verify_deleted_labels(
                original,
                candidate,
                &old_indices,
                &catalog_keys,
                context,
            )?
        } else if intent
            .flatten
            .as_ref()
            .is_some_and(|plan| plan.changes_catalog_acroform())
        {
            labels::verify_with_acroform_change(original, candidate, &old_indices, context)?
        } else {
            labels::verify(original, candidate, &old_indices, context)?
        };
        let actual = crate::read_page_labels(candidate)?;
        let passed = raw_values && actual == labels.expected_labels;
        checks.push(CheckEvidence {
            code: code("page_labels"),
            status: if passed { CheckStatus::Passed } else { CheckStatus::Failed },
            detail: "Explicit labels and raw specifications follow source page identities. Catalog keys, label-tree topology and opaque values are checked against the source independently of staged replacement dictionaries.".into(),
        });
    }
    Ok(EvidenceReport::new(checks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};

    fn fixture() -> PdfRevision {
        let mut input = include_bytes!("../tests/fixtures/preservation/metadata.pdf").to_vec();
        // Original mandatory fixture, with the font-selection operator replaced
        // by whitespace of equal length to create the supported raw-token case.
        let start = input
            .windows(9)
            .position(|value| value == b"/F1 12 Tf")
            .unwrap();
        input[start..start + 9].fill(b' ');
        PdfRevision::new(
            crate::PdfEngine::default()
                .open_owned(input, crate::OpenOptions::default())
                .unwrap(),
        )
    }
    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1_000_000,
                max_work_units: 1_000_000,
            }),
            CancellationToken::new(),
        )
    }
    fn request() -> PdfRevisionRequest {
        PdfRevisionRequest {
            text: vec![SurgicalTextEditRequest {
                old_text: "A".into(),
                replacement: "B".into(),
                match_index: 0,
            }],
            annotations: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn commits_share_untouched_objects_payloads_and_decode_work_state() {
        let mut revision = fixture();
        let original = revision.clone();
        let context = context();
        let plan = revision.plan(request(), &context).unwrap();
        revision.commit(&plan, &context).unwrap();
        let before = original.state.value();
        let after = revision.state.value();
        assert!(
            before
                .input_source()
                .shares_storage_with(after.input_source())
        );
        for (reference, object) in &before.parsed().objects {
            let changed = plan.envelope.intent().objects.contains_key(reference);
            assert_eq!(
                std::ptr::eq(object, after.parsed().object(*reference).unwrap()),
                !changed
            );
            if !changed && let Some(stream) = object.stream.as_deref() {
                assert!(std::ptr::eq(
                    stream.as_ptr(),
                    after
                        .parsed()
                        .object(*reference)
                        .unwrap()
                        .stream
                        .as_deref()
                        .unwrap()
                        .as_ptr()
                ));
            }
        }
        assert_eq!(before.decode_metrics(), after.decode_metrics());
        assert_eq!(revision.text().unwrap(), ["B"]);
        assert_eq!(original.text().unwrap(), ["A"]);
        assert_eq!(before.decode_metrics(), after.decode_metrics());
    }

    #[test]
    fn independent_geometry_intent_rejects_self_consistent_wrong_rotation_before_commit() {
        let mut revision = fixture();
        let ctx = context();
        let snapshot = revision.snapshot();
        let original = revision.state.value();
        let page = original.page_refs().unwrap()[0];
        let mut geometry = geometry::plan(
            original,
            &[crate::PdfPageTransformRequest {
                page_index: 0,
                transform: crate::PageTransform {
                    rotation_degrees: Some(90),
                    ..Default::default()
                },
            }],
            &ctx,
        )
        .unwrap();
        let mut objects = BTreeMap::new();
        geometry::apply(&mut geometry, original, &mut objects, &ctx).unwrap();
        let Value::Dict(dictionary) = &mut objects.get_mut(&page).unwrap().value else {
            panic!("page dictionary");
        };
        // Corrupt the proposed object and its ordinary expected-object record
        // together. Only the independently source-derived intent can catch this.
        dictionary.insert(b"Rotate".to_vec(), Value::Integer(180));
        let intent = Intent {
            objects,
            streams: BTreeMap::new(),
            annotations: BTreeMap::new(),
            reorder: None,
            labels: None,
            text_pages: BTreeMap::new(),
            font_effects: Vec::new(),
            geometry: Some(geometry),
            checkboxes: None,
            flatten: None,
            text_fields: None,
            duplicate: None,
            delete: None,
        };
        let mut parsed = original.parsed().clone();
        for (reference, object) in &intent.objects {
            parsed.objects.insert(*reference, object.clone());
        }
        let evidence =
            verify_candidate(original, &original.with_parsed(parsed), &intent, &ctx).unwrap();
        assert!(
            evidence
                .checks()
                .iter()
                .any(|check| check.code.code() == "expected_changes"
                    && check.status == CheckStatus::Passed)
        );
        let required = required(&intent);
        assert!(evidence.validate(&required).is_err());
        let plan = PdfRevisionPlan {
            envelope: PlanEnvelope::new(snapshot, intent, required),
            store: original.parsed().objects.clone(),
            _decoded_reservations: Vec::new(),
        };
        assert!(matches!(
            revision.commit(&plan, &ctx),
            Err(TransactionError::Evidence(_))
        ));
        assert_eq!(revision.snapshot(), snapshot);
        assert_eq!(revision.page_geometry(0).unwrap().rotation_degrees, 0);
        assert_eq!(revision.text().unwrap(), ["A"]);
    }

    #[test]
    fn source_to_candidate_check_detects_injected_unrequested_catalog_loss() {
        let revision = fixture();
        let context = context();
        let plan = revision.plan(request(), &context).unwrap();
        let original = revision.state.value();
        let mut parsed = original.parsed().clone();
        for (reference, object) in &plan.envelope.intent().objects {
            parsed.objects.insert(*reference, object.clone());
        }
        let catalog = parsed
            .objects
            .get_mut(&ObjectRef {
                number: 1,
                generation: 0,
            })
            .unwrap();
        let Value::Dict(dictionary) = &mut catalog.value else {
            panic!("catalog dictionary");
        };
        dictionary.remove(b"Metadata".as_slice());
        let candidate = original.with_parsed(parsed);
        let evidence =
            verify_candidate(original, &candidate, plan.envelope.intent(), &context).unwrap();
        assert!(evidence.validate(plan.required_checks()).is_err());
        assert!(
            evidence
                .checks()
                .iter()
                .any(|check| check.code.code() == "untouched_objects"
                    && check.status == CheckStatus::Failed)
        );
        assert!(
            evidence
                .checks()
                .iter()
                .any(|check| check.code.code() == "page_relationships"
                    && check.status == CheckStatus::Passed)
        );
    }

    #[test]
    fn source_to_candidate_check_detects_header_version_mutation() {
        let revision = fixture();
        let context = context();
        let plan = revision.plan(request(), &context).unwrap();
        let original = revision.state.value();
        let mut parsed = original.parsed().clone();
        for (reference, object) in &plan.envelope.intent().objects {
            parsed.objects.insert(*reference, object.clone());
        }
        parsed.version = "1.6".into();
        let candidate = original.with_parsed(parsed);
        let evidence =
            verify_candidate(original, &candidate, plan.envelope.intent(), &context).unwrap();
        assert!(evidence.validate(plan.required_checks()).is_err());
        assert!(
            evidence
                .checks()
                .iter()
                .any(|check| check.code.code() == "untouched_objects"
                    && check.status == CheckStatus::Failed)
        );
    }

    #[test]
    fn requested_unicode_check_rejects_a_wrong_but_self_consistent_staged_payload() {
        let revision = PdfRevision::new(
            crate::PdfEngine::default()
                .open(
                    include_bytes!("../../../qa/preservation/fixtures/content-sequence.pdf"),
                    crate::OpenOptions::default(),
                )
                .unwrap(),
        );
        let ctx = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    page_text: vec![PdfPageTextEditRequest {
                        page_index: 0,
                        edit: SurgicalTextEditRequest {
                            old_text: "Ω".into(),
                            replacement: "Ψ".into(),
                            match_index: 0,
                        },
                        shared: PdfSharedEditMode::CloneForPage,
                    }],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        let requested = plan.envelope.intent();
        // Simulate a staging bug that chose the old font code, and recorded
        // those same wrong bytes as the expected encoded/decoded payload. A
        // staged-to-reparsed comparison alone would accept this no-op edit.
        let mut intent = Intent {
            objects: requested.objects.clone(),
            streams: requested
                .streams
                .iter()
                .map(|(reference, stream)| {
                    (
                        *reference,
                        StreamIntent {
                            expected: stream.expected.clone(),
                            owners: stream.owners.clone(),
                            effect: stream.effect.clone(),
                        },
                    )
                })
                .collect(),
            annotations: BTreeMap::new(),
            reorder: None,
            labels: None,
            text_pages: requested.text_pages.clone(),
            font_effects: requested.font_effects.clone(),
            geometry: None,
            checkboxes: None,
            flatten: None,
            text_fields: None,
            duplicate: None,
            delete: None,
        };
        let clone = *intent.streams.keys().next().unwrap();
        let payload = &mut intent.streams.get_mut(&clone).unwrap().expected;
        let offset = payload
            .windows(3)
            .position(|value| value == b"(B)")
            .unwrap();
        payload[offset + 1] = b'A';
        intent.objects.get_mut(&clone).unwrap().stream = Some(payload.clone().into());
        let original = revision.state.value();
        let mut parsed = original.parsed().clone();
        for (reference, object) in &intent.objects {
            parsed.objects.insert(*reference, object.clone());
        }
        let evidence =
            verify_candidate(original, &original.with_parsed(parsed), &intent, &ctx).unwrap();
        assert!(evidence.validate(plan.required_checks()).is_err());
        for check in evidence.checks() {
            assert_eq!(
                check.status,
                if check.code.code() == "requested_text" {
                    CheckStatus::Failed
                } else {
                    CheckStatus::Passed
                },
                "{}",
                check.code.code()
            );
        }
        assert_eq!(revision.snapshot().revision(), RevisionId::INITIAL);
        assert_eq!(
            revision.text().unwrap(),
            ["Ω", "Ω", "Ж", "Ψ", "Ж", "Ж", "Ж", "Ю"]
        );
    }

    #[test]
    fn clone_receipt_rejects_injected_loss_extra_objects_and_owner_changes() {
        let mut input = include_bytes!("../tests/fixtures/preservation/type3-shared.pdf").to_vec();
        for index in 0..input.len().saturating_sub(8) {
            if &input[index..index + 9] == b"/F1 12 Tf" {
                input[index..index + 9].fill(b' ');
            }
        }
        let offset = input
            .windows(15)
            .position(|value| value == b"/Contents 6 0 R")
            .unwrap();
        input[offset + 10] = b'5';
        let revision = PdfRevision::new(
            crate::PdfEngine::default()
                .open_owned(input, crate::OpenOptions::default())
                .unwrap(),
        );
        let ctx = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    page_text: vec![PdfPageTextEditRequest {
                        page_index: 0,
                        shared: PdfSharedEditMode::CloneForPage,
                        edit: SurgicalTextEditRequest {
                            old_text: "A".into(),
                            replacement: "replacement".into(),
                            match_index: 0,
                        },
                    }],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        let original = revision.state.value();
        let intent = plan.envelope.intent();
        let clone = *intent.streams.keys().next().unwrap();
        assert_eq!(
            clone,
            ObjectRef {
                number: 10,
                generation: 0
            }
        );
        let mut parsed = original.parsed().clone();
        for (reference, object) in &intent.objects {
            parsed.objects.insert(*reference, object.clone());
        }
        let candidate = original.with_parsed(parsed.clone());
        verify_candidate(original, &candidate, intent, &ctx)
            .unwrap()
            .validate(plan.required_checks())
            .unwrap();
        assert_eq!(
            candidate
                .extract_text_spans()
                .unwrap()
                .spans
                .iter()
                .filter(|span| span.object_number == clone.number)
                .map(|span| span.source_span)
                .collect::<Vec<_>>(),
            [None]
        );
        for fault in 0..6 {
            let mut damaged = parsed.clone();
            match fault {
                0 => {
                    damaged.objects.remove(&clone);
                }
                1 => {
                    let Value::Dict(dictionary) =
                        &mut damaged.objects.get_mut(&clone).unwrap().value
                    else {
                        unreachable!()
                    };
                    dictionary.insert(b"DL".to_vec(), Value::Integer(999));
                }
                2 => {
                    let Value::Dict(dictionary) = &mut damaged
                        .objects
                        .get_mut(&ObjectRef {
                            number: 4,
                            generation: 0,
                        })
                        .unwrap()
                        .value
                    else {
                        unreachable!()
                    };
                    dictionary.insert(b"Contents".to_vec(), Value::Ref(clone));
                }
                3 => {
                    damaged
                        .objects
                        .get_mut(&ObjectRef {
                            number: 5,
                            generation: 0,
                        })
                        .unwrap()
                        .stream = Some(b"BT (LOSS) Tj ET".to_vec().into());
                }
                4 => {
                    damaged.objects.insert(
                        ObjectRef {
                            number: 11,
                            generation: 0,
                        },
                        IndirectObject {
                            value: Value::Null,
                            stream: None,
                            offset: 0,
                            stream_offset: 0,
                        },
                    );
                }
                5 => {
                    damaged.objects.remove(&ObjectRef {
                        number: 8,
                        generation: 0,
                    });
                }
                _ => unreachable!(),
            }
            let result = verify_candidate(original, &original.with_parsed(damaged), intent, &ctx);
            assert!(
                result.is_err() || result.unwrap().validate(plan.required_checks()).is_err(),
                "fault {fault}"
            );
        }
        assert_eq!(revision.text().unwrap(), ["A", "A"]);
    }
}
