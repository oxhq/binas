//! Exact 8-bit image samples with deterministic PNG Up/Flate storage.
//! Original encoded data remains the candidate when the new payload is not
//! smaller. Stream savings are not final PDF size, color validation or a codec
//! quality policy. This pass never changes image interpretation or masks.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SnapshotId,
    transaction::{
        CommitReceipt, PlanEnvelope, RevisionState, StagedChange, TransactionError, execute_plan,
    },
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};

use super::structure::{
    bytes_equal, reserve, same_metadata, same_object, value_size, values_equal,
};
use crate::{
    DecodedPayload, ImageColorSpace, PdfDocument, PdfError, PdfReferenceTarget, PdfResourceUsage,
    PdfUsageGap,
    filters::encode_png_up_flate_with_context,
    object::{IndirectObject, ObjectRef, Value},
    parser::{ParseBudget, decode_stream_bounded},
    payload::StreamPayload,
    resource_usage::census,
    writer::{refuse_security_boundaries_with_context, refuse_stale_linearization_with_context},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfLosslessImageEffect {
    pub image: PdfReferenceTarget,
    pub width: u32,
    pub height: u32,
    pub color_space: ImageColorSpace,
    pub source_predictor: u8,
    pub output_predictor: u8,
    pub encoded_before: usize,
    pub encoded_after: usize,
    pub decoded_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfLosslessImageSkipReason {
    NotUsed,
    UnsupportedImageLayout,
    UnsupportedColorSpace,
    UnsupportedFilter,
    DecodeParameters,
    ExternalStream,
    DirectLengthRequired,
    ImageMask,
    EmbeddedAlpha,
    NotSmaller,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfLosslessImageSkip {
    pub image: PdfReferenceTarget,
    pub reason: PdfLosslessImageSkipReason,
}

pub type PdfLosslessImageReceipt = CommitReceipt<Vec<PdfLosslessImageEffect>, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Layout {
    width: u32,
    height: u32,
    colors: usize,
    color_space: ImageColorSpace,
    predictor: u8,
    row_bytes: usize,
    sample_bytes: usize,
}

#[derive(Debug)]
struct Replacement {
    object: IndirectObject,
    samples: DecodedPayload,
    effect: PdfLosslessImageEffect,
}

#[derive(Debug)]
struct Intent {
    replacements: BTreeMap<ObjectRef, Replacement>,
}

/// Retains the immutable current revision and original encoded candidates.
/// Decoded payloads retain their shared document-budget reservations; encoded
/// outputs and copied plan metadata have operation-budget reservations. Codec
/// scratch, allocator spare capacity and parsed source storage are separate.
#[derive(Debug)]
pub struct PdfLosslessImagePlan {
    envelope: PlanEnvelope<Intent>,
    origin: PdfDocument,
    effects: Vec<PdfLosslessImageEffect>,
    skips: Vec<PdfLosslessImageSkip>,
    encoded_bytes_saved: u64,
    usage: PdfResourceUsage,
    _reservations: Vec<Reservation>,
}

impl PdfLosslessImagePlan {
    pub fn snapshot(&self) -> SnapshotId {
        self.envelope.snapshot()
    }
    pub fn effects(&self) -> &[PdfLosslessImageEffect] {
        &self.effects
    }
    pub fn skips(&self) -> &[PdfLosslessImageSkip] {
        &self.skips
    }
    /// Selected encoded payload reductions, excluding serialized dictionaries,
    /// xrefs and retained incremental history. This is not final PDF savings.
    pub fn encoded_bytes_saved(&self) -> u64 {
        self.encoded_bytes_saved
    }
    pub fn required_checks(&self) -> &RequiredChecks {
        self.envelope.required()
    }
    pub fn census_complete(&self) -> bool {
        self.usage.complete()
    }
    pub fn census_gaps(&self) -> &[PdfUsageGap] {
        &self.usage.gaps
    }
}

pub(crate) fn plan(
    document: &PdfDocument,
    snapshot: SnapshotId,
    context: &OperationContext,
) -> Result<PdfLosslessImagePlan, PdfError> {
    work(context, 1)?;
    if snapshot.source() != document.source_id() {
        return Err(PdfError::selection(
            "lossless image plan belongs to another source",
        ));
    }
    refuse_security_boundaries_with_context(document.parsed(), Some(context))?;
    refuse_stale_linearization_with_context(document.parsed(), Some(context))?;
    let mut reservations = Vec::new();
    reserve(
        &mut reservations,
        value_size(&document.parsed().trailer, document, context)?
            .checked_add(document.parsed().version.len())
            .ok_or_else(|| PdfError::limit("image plan metadata overflows"))?,
        context,
    )?;
    let document = document.with_cancellation(context.cancellation().clone());
    let usage = census(&document, snapshot, None, context)?;
    let (used, _used_reservations) = used_images(&usage, context)?;
    let mut budget = ParseBudget::for_document(document.parsed());
    let mut replacements = BTreeMap::new();
    let mut effects = Vec::new();
    let mut skips = Vec::new();
    let mut encoded_bytes_saved = 0_u64;
    for (reference, object) in &document.parsed().objects {
        work(context, 1)?;
        if !image_object(object) {
            continue;
        }
        if effects
            .len()
            .checked_add(skips.len())
            .is_none_or(|count| count >= document.parsed().limits.max_container_items)
        {
            return Err(PdfError::limit(
                "lossless image inventory exceeds container limit",
            ));
        }
        let layout = if used.contains(reference) {
            match eligibility(object, &document, context)? {
                Ok(layout) => layout,
                Err(reason) => {
                    push_skip(&mut skips, &mut reservations, *reference, reason, context)?;
                    continue;
                }
            }
        } else {
            push_skip(
                &mut skips,
                &mut reservations,
                *reference,
                PdfLosslessImageSkipReason::NotUsed,
                context,
            )?;
            continue;
        };
        let encoded = object.stream.as_ref().expect("eligible image stream");
        predictor_size(layout, document.parsed().limits.max_stream_bytes)?;
        let samples = decode_samples(&document, object, layout, &mut budget)?;
        let (candidate, candidate_reservations) = encode_png_up_flate_with_context(
            &samples,
            layout.row_bytes,
            layout.height as usize,
            document.parsed().limits.max_stream_bytes,
            context,
        )?;
        if candidate.len() >= encoded.len() {
            push_skip(
                &mut skips,
                &mut reservations,
                *reference,
                PdfLosslessImageSkipReason::NotSmaller,
                context,
            )?;
            continue;
        }
        let effect = effect(*reference, layout, encoded.len(), candidate.len());
        let record_bytes = value_size(&object.value, &document, context)?
            .checked_add(
                size_of::<Replacement>()
                    + size_of::<ObjectRef>()
                    + size_of::<PdfLosslessImageEffect>() * 2
                    + size_of::<Value>() * 6
                    + 64,
            )
            .ok_or_else(|| PdfError::limit("image candidate metadata overflows"))?;
        reserve(&mut reservations, record_bytes, context)?;
        let mut after = object.clone();
        let Value::Dict(values) = &mut after.value else {
            unreachable!("eligible image dictionary");
        };
        values.insert(b"DecodeParms".to_vec(), predictor_dictionary(layout));
        values.insert(
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(candidate.len())
                    .map_err(|_| PdfError::limit("image encoded length exceeds i64"))?,
            ),
        );
        after.stream = Some(StreamPayload::from_reserved(
            candidate,
            candidate_reservations,
        )?);
        // Check the generated filter declaration against every original sample
        // before retaining a candidate; commit independently repeats this proof.
        let after_layout = Layout {
            predictor: 12,
            ..layout
        };
        let reconstructed = decode_samples(&document, &after, after_layout, &mut budget)?;
        if !bytes_equal(&samples, &reconstructed, context)? {
            return Err(PdfError::verification(
                "PNG Up candidate changed image samples",
            ));
        }
        encoded_bytes_saved = encoded_bytes_saved
            .checked_add((effect.encoded_before - effect.encoded_after) as u64)
            .ok_or_else(|| PdfError::limit("image encoded savings overflow"))?;
        effects.push(effect.clone());
        replacements.insert(
            *reference,
            Replacement {
                object: after,
                samples,
                effect,
            },
        );
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(PdfLosslessImagePlan {
        envelope: PlanEnvelope::new(snapshot, Intent { replacements }, required()),
        origin: document,
        effects,
        skips,
        encoded_bytes_saved,
        usage,
        _reservations: reservations,
    })
}

pub(crate) fn commit(
    state: &mut RevisionState<PdfDocument>,
    plan: &PdfLosslessImagePlan,
    context: &OperationContext,
) -> Result<Option<PdfLosslessImageReceipt>, TransactionError<PdfError>> {
    if plan.snapshot() != state.snapshot() {
        return Err(TransactionError::StalePlan {
            planned: plan.snapshot(),
            current: state.snapshot(),
        });
    }
    context.checkpoint().map_err(TransactionError::Context)?;
    if !state
        .value()
        .parsed()
        .objects
        .shares_snapshot_with(&plan.origin.parsed().objects)
        || !same_metadata(state.value(), &plan.origin, context).map_err(TransactionError::Stage)?
    {
        return Err(TransactionError::Stage(PdfError::unsafe_rewrite(
            "lossless image plan source/object-store mismatch",
        )));
    }
    if plan.envelope.intent().replacements.is_empty() {
        return Ok(None);
    }
    execute_plan(
        state,
        &plan.envelope,
        context,
        |original, intent, context| {
            let mut parsed = original.parsed().clone();
            parsed.decode_cancellation = context.cancellation().clone();
            let mut changes = Vec::new();
            for (reference, replacement) in &intent.replacements {
                work(context, 1)?;
                parsed
                    .objects
                    .insert(*reference, replacement.object.clone());
                changes.push(replacement.effect.clone());
            }
            Ok(StagedChange {
                candidate: original.with_parsed(parsed),
                changes,
            })
        },
        |original, candidate, intent, _, context| {
            verify_candidate(
                &original.with_cancellation(context.cancellation().clone()),
                candidate,
                intent,
                context,
            )
        },
    )
    .map(Some)
}

fn eligibility(
    object: &IndirectObject,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<Result<Layout, PdfLosslessImageSkipReason>, PdfError> {
    use PdfLosslessImageSkipReason as Skip;
    work(context, 1)?;
    let Value::Dict(values) = &object.value else {
        return Ok(Err(Skip::UnsupportedImageLayout));
    };
    let Some(encoded) = &object.stream else {
        return Ok(Err(Skip::UnsupportedImageLayout));
    };
    if values.get(b"Subtype".as_slice()) != Some(&Value::Name(b"Image".to_vec()))
        || values
            .get(b"Type".as_slice())
            .is_some_and(|value| *value != Value::Name(b"XObject".to_vec()))
    {
        return Ok(Err(Skip::UnsupportedImageLayout));
    }
    if [b"F".as_slice(), b"FFilter", b"FDecodeParms", b"OPI", b"Ref"]
        .iter()
        .any(|key| values.contains_key(*key))
    {
        return Ok(Err(Skip::ExternalStream));
    }
    if !matches!(
        values.get(b"ImageMask".as_slice()),
        None | Some(Value::Null | Value::Bool(false))
    ) {
        return Ok(Err(Skip::ImageMask));
    }
    if values.contains_key(b"SMaskInData".as_slice()) {
        return Ok(Err(Skip::EmbeddedAlpha));
    }
    let (Some(width), Some(height)) = (
        positive_u32(values.get(b"Width".as_slice())),
        positive_u32(values.get(b"Height".as_slice())),
    ) else {
        return Ok(Err(Skip::UnsupportedImageLayout));
    };
    if values.get(b"BitsPerComponent".as_slice()) != Some(&Value::Integer(8)) {
        return Ok(Err(Skip::UnsupportedImageLayout));
    }
    let (color_space, colors) = match values.get(b"ColorSpace".as_slice()) {
        Some(Value::Name(name)) if name == b"DeviceGray" => (ImageColorSpace::DeviceGray, 1usize),
        Some(Value::Name(name)) if name == b"DeviceRGB" => (ImageColorSpace::DeviceRgb, 3usize),
        _ => return Ok(Err(Skip::UnsupportedColorSpace)),
    };
    if !matches!(values.get(b"Filter".as_slice()), Some(Value::Name(name)) if name == b"FlateDecode")
    {
        return Ok(Err(Skip::UnsupportedFilter));
    }
    if !matches!(values.get(b"Length".as_slice()), Some(Value::Integer(length)) if usize::try_from(*length).ok() == Some(encoded.len()))
    {
        return Ok(Err(Skip::DirectLengthRequired));
    }
    let mut predictor = 1;
    match values.get(b"DecodeParms".as_slice()) {
        None | Some(Value::Null) => {}
        Some(Value::Dict(params)) => {
            for (key, value) in params {
                work(context, key.len() + 1)?;
                let Value::Integer(value) = value else {
                    return Ok(Err(Skip::DecodeParameters));
                };
                let valid = match key.as_slice() {
                    b"Predictor" if matches!(*value, 1 | 12) => {
                        predictor = *value as u8;
                        true
                    }
                    b"Colors" => usize::try_from(*value).ok() == Some(colors),
                    b"BitsPerComponent" => *value == 8,
                    b"Columns" => u32::try_from(*value).ok() == Some(width),
                    b"EarlyChange" => *value == 1,
                    _ => false,
                };
                if !valid {
                    return Ok(Err(Skip::DecodeParameters));
                }
            }
            if predictor == 12 {
                // Missing values use PDF defaults. Those defaults must match
                // the actual image layout before a row-bound proof is possible.
                if !params.contains_key(b"Colors".as_slice()) && colors != 1
                    || !params.contains_key(b"Columns".as_slice()) && width != 1
                {
                    return Ok(Err(Skip::DecodeParameters));
                }
            }
        }
        _ => return Ok(Err(Skip::DecodeParameters)),
    }
    let row_bytes = usize::try_from(width)
        .ok()
        .and_then(|width| width.checked_mul(colors))
        .ok_or_else(|| PdfError::limit("image row size overflows"))?;
    let sample_bytes = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| PdfError::limit("image sample size overflows"))?;
    if sample_bytes > document.parsed().limits.max_stream_bytes {
        return Err(PdfError::limit("image samples exceed max_stream_bytes"));
    }
    Ok(Ok(Layout {
        width,
        height,
        colors,
        color_space,
        predictor,
        row_bytes,
        sample_bytes,
    }))
}

fn decode_samples(
    document: &PdfDocument,
    object: &IndirectObject,
    layout: Layout,
    budget: &mut ParseBudget,
) -> Result<DecodedPayload, PdfError> {
    let max_intermediate = if layout.predictor == 12 {
        predictor_size(layout, document.parsed().limits.max_stream_bytes)?
    } else {
        layout.sample_bytes
    };
    let samples = decode_stream_bounded(
        &object.value,
        object
            .stream
            .as_deref()
            .ok_or_else(|| PdfError::verification("image candidate lost its stream"))?,
        &document.parsed().limits,
        budget,
        max_intermediate,
    )?;
    if samples.len() != layout.sample_bytes {
        return Err(PdfError::syntax(
            "image decoded sample length disagrees with width, height and components",
            0,
        ));
    }
    Ok(samples)
}

fn predictor_size(layout: Layout, limit: usize) -> Result<usize, PdfError> {
    let bytes = layout
        .sample_bytes
        .checked_add(layout.height as usize)
        .ok_or_else(|| PdfError::limit("image predictor intermediate overflows"))?;
    if bytes > limit {
        return Err(PdfError::limit(
            "image predictor intermediate exceeds max_stream_bytes",
        ));
    }
    Ok(bytes)
}

fn predictor_dictionary(layout: Layout) -> Value {
    Value::Dict(BTreeMap::from([
        (b"Predictor".to_vec(), Value::Integer(12)),
        (b"Colors".to_vec(), Value::Integer(layout.colors as i64)),
        (b"BitsPerComponent".to_vec(), Value::Integer(8)),
        (b"Columns".to_vec(), Value::Integer(i64::from(layout.width))),
    ]))
}

fn effect(
    reference: ObjectRef,
    layout: Layout,
    before: usize,
    after: usize,
) -> PdfLosslessImageEffect {
    PdfLosslessImageEffect {
        image: public_ref(reference),
        width: layout.width,
        height: layout.height,
        color_space: layout.color_space,
        source_predictor: layout.predictor,
        output_predictor: 12,
        encoded_before: before,
        encoded_after: after,
        decoded_bytes: layout.sample_bytes,
    }
}

const CHECKS: [&str; 6] = [
    "source_graph",
    "image_interpretation",
    "exact_samples",
    "encoded_candidates",
    "untouched_objects",
    "page_relationships",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.optimize.images", name).expect("static image check")
}
fn required() -> RequiredChecks {
    RequiredChecks::new(CHECKS.into_iter().map(code).collect()).expect("static image obligations")
}

fn verify_candidate(
    original: &PdfDocument,
    candidate: &PdfDocument,
    intent: &Intent,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    let graph = same_metadata(original, candidate, context)?
        && original.parsed().objects.len() == candidate.parsed().objects.len();
    // Independently obtain source roles; a forged replacement map must not
    // authorize a stream that was unused or never an image in the source.
    let usage = census(
        original,
        SnapshotId::new(original.source_id(), binas_core::RevisionId::INITIAL),
        None,
        context,
    )?;
    let (used, _reservations) = used_images(&usage, context)?;
    let mut interpretation = true;
    let mut samples_equal = true;
    let mut encoded = true;
    let mut untouched = true;
    let mut before_budget = ParseBudget::for_document(original.parsed());
    let mut after_budget = ParseBudget::for_document(candidate.parsed());
    for (reference, before) in &original.parsed().objects {
        work(context, 1)?;
        let Some(after) = candidate.parsed().objects.get(reference) else {
            untouched = false;
            encoded = false;
            continue;
        };
        let Some(replacement) = intent.replacements.get(reference) else {
            untouched &=
                std::ptr::eq(before, after) && same_object(before, after, original, context)?;
            continue;
        };
        let layout = match eligibility(before, original, context)? {
            Ok(layout) if used.contains(reference) => layout,
            _ => {
                interpretation = false;
                encoded = false;
                samples_equal = false;
                continue;
            }
        };
        let after_layout = match eligibility(after, candidate, context)? {
            Ok(layout) if layout.predictor == 12 => layout,
            _ => {
                interpretation = false;
                encoded = false;
                samples_equal = false;
                continue;
            }
        };
        interpretation &= after_layout
            == Layout {
                predictor: 12,
                ..layout
            }
            && same_image_dictionary(&before.value, &after.value, layout, original, context)?;
        let before_bytes = before.stream.as_deref().expect("eligible source image");
        let after_bytes = after.stream.as_deref().expect("eligible candidate image");
        encoded &= same_object(after, &replacement.object, original, context)?
            && before.offset == after.offset
            && before.stream_offset == after.stream_offset
            && after_bytes.len() < before_bytes.len()
            && replacement.effect
                == effect(*reference, layout, before_bytes.len(), after_bytes.len());
        let before_samples = decode_samples(original, before, layout, &mut before_budget)?;
        let after_samples = decode_samples(candidate, after, after_layout, &mut after_budget)?;
        samples_equal &= bytes_equal(&before_samples, &after_samples, context)?
            && bytes_equal(&before_samples, &replacement.samples, context)?;
    }
    for reference in intent.replacements.keys() {
        work(context, 1)?;
        encoded &= original.parsed().objects.contains_key(reference)
            && candidate.parsed().objects.contains_key(reference);
    }
    let pages = original.page_refs()? == candidate.page_refs()?;
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(EvidenceReport::new(CHECKS.into_iter().zip([graph,interpretation,samples_equal,encoded,untouched,pages]).map(|(name, passed)| CheckEvidence {
        code:code(name),status:if passed {CheckStatus::Passed}else{CheckStatus::Failed},
        detail:"Every selected 8-bit image sample byte matches the current source. Color, Decode, mask references, private dictionary data, source graph and unrelated payloads remain exact. Encoded image savings only; no color-conversion, renderer, conformance or final-PDF-size claim.".into(),
    }).collect()))
}

fn same_image_dictionary(
    before: &Value,
    after: &Value,
    layout: Layout,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    let (Value::Dict(before), Value::Dict(after)) = (before, after) else {
        return Ok(false);
    };
    for (key, value) in before {
        work(context, key.len() + 1)?;
        if matches!(key.as_slice(), b"Length" | b"DecodeParms") {
            continue;
        }
        let Some(actual) = after.get(key) else {
            return Ok(false);
        };
        if !values_equal(value, actual, 1, document, context)? {
            return Ok(false);
        }
    }
    for key in after.keys() {
        work(context, key.len() + 1)?;
        if !matches!(key.as_slice(), b"Length" | b"DecodeParms") && !before.contains_key(key) {
            return Ok(false);
        }
    }
    Ok(after
        .get(b"DecodeParms".as_slice())
        .is_some_and(|value| *value == predictor_dictionary(layout)))
}

fn used_images(
    usage: &PdfResourceUsage,
    context: &OperationContext,
) -> Result<(BTreeSet<ObjectRef>, Vec<Reservation>), PdfError> {
    let mut references = BTreeSet::new();
    let mut reservations = Vec::new();
    for item in &usage.uses {
        work(context, 1)?;
        if item.is_image
            && let Some(reference) = item.target
        {
            let reference = ObjectRef {
                number: reference.number,
                generation: reference.generation,
            };
            if !references.contains(&reference) {
                reserve(&mut reservations, size_of::<ObjectRef>(), context)?;
                references.insert(reference);
            }
        }
    }
    Ok((references, reservations))
}

fn image_object(object: &IndirectObject) -> bool {
    matches!(&object.value, Value::Dict(values) if matches!(values.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Image"))
}
fn positive_u32(value: Option<&Value>) -> Option<u32> {
    match value {
        Some(Value::Integer(value)) => u32::try_from(*value).ok().filter(|value| *value > 0),
        _ => None,
    }
}
fn public_ref(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}
fn work(context: &OperationContext, units: usize) -> Result<(), PdfError> {
    context
        .charge_work(units as u64)
        .map_err(PdfError::from_context)
}
fn push_skip(
    skips: &mut Vec<PdfLosslessImageSkip>,
    reservations: &mut Vec<Reservation>,
    reference: ObjectRef,
    reason: PdfLosslessImageSkipReason,
    context: &OperationContext,
) -> Result<(), PdfError> {
    reserve(reservations, size_of::<PdfLosslessImageSkip>(), context)?;
    skips.push(PdfLosslessImageSkip {
        image: public_ref(reference),
        reason,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfErrorCode};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget, RevisionId};

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 32 * 1024 * 1024,
                max_work_units: 128 * 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }
    fn fixture() -> PdfDocument {
        PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/lossless-images.pdf"),
                OpenOptions::default(),
            )
            .unwrap()
    }
    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn state(document: PdfDocument) -> RevisionState<PdfDocument> {
        RevisionState::new(
            SnapshotId::new(document.source_id(), RevisionId::INITIAL),
            document,
        )
    }
    fn copied_intent(plan: &PdfLosslessImagePlan) -> Intent {
        Intent {
            replacements: plan
                .envelope
                .intent()
                .replacements
                .iter()
                .map(|(reference, replacement)| {
                    (
                        *reference,
                        Replacement {
                            object: replacement.object.clone(),
                            samples: replacement.samples.clone(),
                            effect: replacement.effect.clone(),
                        },
                    )
                })
                .collect(),
        }
    }
    fn staged(document: &PdfDocument, intent: &Intent) -> PdfDocument {
        let mut parsed = document.parsed().clone();
        for (reference, replacement) in &intent.replacements {
            parsed
                .objects
                .insert(*reference, replacement.object.clone());
        }
        document.with_parsed(parsed)
    }
    fn dictionary(object: &mut IndirectObject) -> &mut BTreeMap<Vec<u8>, Value> {
        let Value::Dict(values) = &mut object.value else {
            panic!("expected dictionary");
        };
        values
    }
    fn passed(report: &EvidenceReport<String>, name: &str) -> bool {
        report
            .checks()
            .iter()
            .any(|check| check.code == code(name) && check.status == CheckStatus::Passed)
    }
    #[test]
    fn up_images_preserve_samples_and_repeat_as_no_op() {
        let source = fixture();
        let mut state = state(source.clone());
        let context = context();
        let planned = plan(state.value(), state.snapshot(), &context).unwrap();
        assert_eq!(
            planned
                .effects()
                .iter()
                .map(|effect| effect.image.number)
                .collect::<Vec<_>>(),
            [7, 8]
        );
        assert!(
            planned.skips().iter().any(|skip| skip.image.number == 13
                && skip.reason == PdfLosslessImageSkipReason::NotSmaller)
        );
        assert!(commit(&mut state, &planned, &context).unwrap().is_some());
        for number in [5, 6, 9, 10, 12, 13] {
            assert!(
                same_object(
                    source.parsed().object(reference(number)).unwrap(),
                    state.value().parsed().object(reference(number)).unwrap(),
                    &source,
                    &context
                )
                .unwrap()
            );
        }
        let next = plan(state.value(), state.snapshot(), &context).unwrap();
        assert!(next.effects().is_empty());
        let snapshot = state.snapshot();
        assert!(commit(&mut state, &next, &context).unwrap().is_none());
        assert_eq!(state.snapshot(), snapshot);
    }

    #[test]
    fn image_plans_bind_source_revision_store_metadata_and_cancellation() {
        let source = fixture();
        let mut current = state(source.clone());
        let context = context();
        let planned = plan(&source, current.snapshot(), &context).unwrap();

        let mut bytes = source.input_source().as_bytes().to_vec();
        let position = bytes
            .windows(6)
            .position(|bytes| bytes == b"before")
            .unwrap();
        bytes[position] = b'B';
        let other = PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap();
        assert_eq!(other.input_source().len(), source.input_source().len());
        assert!(matches!(
            commit(&mut state(other.clone()), &planned, &context),
            Err(TransactionError::StalePlan { .. })
        ));
        assert!(plan(&other, current.snapshot(), &context).is_err());
        let later = SnapshotId::new(
            source.source_id(),
            RevisionId::INITIAL.checked_next().unwrap(),
        );
        assert!(matches!(
            commit(
                &mut RevisionState::new(later, source.clone()),
                &planned,
                &context
            ),
            Err(TransactionError::StalePlan { .. })
        ));

        for change_metadata in [false, true] {
            let mut parsed = source.parsed().clone();
            if change_metadata {
                let Value::Dict(values) = &mut parsed.trailer else {
                    unreachable!();
                };
                values.insert(b"PrivateBranch".to_vec(), Value::Bool(true));
            } else {
                dictionary(parsed.objects.get_mut(&reference(20)).unwrap())
                    .insert(b"PrivateBranch".to_vec(), Value::Bool(true));
            }
            let mut sibling = RevisionState::new(current.snapshot(), source.with_parsed(parsed));
            assert!(matches!(
                commit(&mut sibling, &planned, &context),
                Err(TransactionError::Stage(PdfError {
                    code: PdfErrorCode::UnsafeRewrite,
                    ..
                }))
            ));
            assert_eq!(sibling.snapshot(), current.snapshot());
        }
        context.cancellation().cancel();
        let before = current.snapshot();
        assert!(matches!(
            commit(&mut current, &planned, &context),
            Err(TransactionError::Context(
                binas_core::ContextError::Cancelled
            ))
        ));
        assert_eq!(current.snapshot(), before);
        assert!(
            current
                .value()
                .parsed()
                .objects
                .shares_snapshot_with(&source.parsed().objects)
        );
        assert!(
            commit(&mut current, &planned, &self::context())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn self_consistent_changed_samples_fail_source_proof_and_publish_nothing() {
        let source = fixture();
        let mut current = state(source.clone());
        let context = context();
        let mut planned = plan(&source, current.snapshot(), &context).unwrap();
        let mut intent = copied_intent(&planned);
        let replacement = intent.replacements.get_mut(&reference(7)).unwrap();
        let layout = eligibility(&replacement.object, &source, &context)
            .unwrap()
            .unwrap();
        let mut samples = replacement.samples.to_vec();
        samples[0] ^= 0xff;
        let (encoded, reservations) = encode_png_up_flate_with_context(
            &samples,
            layout.row_bytes,
            layout.height as usize,
            source.parsed().limits.max_stream_bytes,
            &context,
        )
        .unwrap();
        assert!(encoded.len() < replacement.effect.encoded_before);
        replacement.effect.encoded_after = encoded.len();
        dictionary(&mut replacement.object)
            .insert(b"Length".to_vec(), Value::Integer(encoded.len() as i64));
        replacement.object.stream =
            Some(StreamPayload::from_reserved(encoded, reservations).unwrap());
        replacement.samples = decode_samples(
            &source,
            &replacement.object,
            layout,
            &mut ParseBudget::for_document(source.parsed()),
        )
        .unwrap();
        let candidate = staged(&source, &intent);
        let report = verify_candidate(&source, &candidate, &intent, &context).unwrap();
        assert!(passed(&report, "encoded_candidates"));
        assert!(passed(&report, "image_interpretation"));
        assert!(!passed(&report, "exact_samples"));
        assert!(report.validate(&required()).is_err());
        planned.envelope = PlanEnvelope::new(current.snapshot(), intent, required());
        let snapshot = current.snapshot();
        assert!(matches!(
            commit(&mut current, &planned, &context),
            Err(TransactionError::Evidence(_))
        ));
        assert_eq!(current.snapshot(), snapshot);
        assert!(
            current
                .value()
                .parsed()
                .objects
                .shares_snapshot_with(&source.parsed().objects)
        );
    }

    #[test]
    fn interpretation_changes_fail_even_when_candidate_and_effect_agree() {
        let source = fixture();
        let context = context();
        let snapshot = SnapshotId::new(source.source_id(), RevisionId::INITIAL);
        let planned = plan(&source, snapshot, &context).unwrap();
        let changes = [
            (b"SMask".as_slice(), Value::Null),
            (
                b"Decode",
                Value::Array(vec![
                    Value::Integer(1),
                    Value::Integer(0),
                    Value::Integer(0),
                    Value::Integer(1),
                    Value::Integer(0),
                    Value::Integer(1),
                ]),
            ),
            (b"Interpolate", Value::Bool(true)),
            (b"PrivateImageData", Value::Ref(reference(20))),
        ];
        for (key, value) in changes {
            let mut intent = copied_intent(&planned);
            dictionary(&mut intent.replacements.get_mut(&reference(7)).unwrap().object)
                .insert(key.to_vec(), value);
            let candidate = staged(&source, &intent);
            let report = verify_candidate(&source, &candidate, &intent, &context).unwrap();
            assert!(passed(&report, "encoded_candidates"));
            assert!(passed(&report, "exact_samples"));
            assert!(!passed(&report, "image_interpretation"), "accepted {key:?}");
            assert!(report.validate(&required()).is_err());
        }
    }

    #[test]
    fn private_image_keys_and_reachable_mask_metadata_are_retained_exactly() {
        let source = fixture();
        let mut parsed = source.parsed().clone();
        let values = dictionary(parsed.objects.get_mut(&reference(7)).unwrap());
        values.insert(b"PrivateImageData".to_vec(), Value::Ref(reference(20)));
        values.insert(
            b"PrivateArray".to_vec(),
            Value::Array(vec![
                Value::String(vec![0, 0xff, b'(']),
                Value::Ref(reference(14)),
            ]),
        );
        let source = source.with_parsed(parsed);
        let mut current = state(source.clone());
        let context = context();
        let planned = plan(&source, current.snapshot(), &context).unwrap();
        assert_eq!(planned.effects().len(), 2);
        let receipt = commit(&mut current, &planned, &context).unwrap().unwrap();
        receipt.evidence.validate(&receipt.required).unwrap();
        let retained_bytes: u64 = receipt
            .changes
            .iter()
            .map(|effect| effect.encoded_after as u64)
            .sum();
        for number in [9, 14, 19, 20] {
            assert!(std::ptr::eq(
                source.parsed().object(reference(number)).unwrap(),
                current.value().parsed().object(reference(number)).unwrap()
            ));
        }
        let before = source.parsed().object(reference(7)).unwrap();
        let after = current.value().parsed().object(reference(7)).unwrap();
        let layout = eligibility(before, &source, &context).unwrap().unwrap();
        assert!(
            same_image_dictionary(&before.value, &after.value, layout, &source, &context).unwrap()
        );
        drop(planned);
        // Only committed encoded payloads remain charged to the operation.
        assert_eq!(context.budget().snapshot().live_bytes, retained_bytes);
        drop(current);
        assert_eq!(context.budget().snapshot().live_bytes, 0);
    }

    #[test]
    fn masks_placements_metadata_and_inventory_loss_fail_untouched_proof() {
        let source = fixture();
        let context = context();
        let planned = plan(
            &source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &context,
        )
        .unwrap();
        let candidate = staged(&source, planned.envelope.intent());
        for number in [6, 9, 14, 19] {
            let mut parsed = candidate.parsed().clone();
            let object = parsed.objects.get_mut(&reference(number)).unwrap();
            let mut bytes = object.stream.as_ref().unwrap().to_vec();
            bytes[0] ^= 1;
            object.stream = Some(bytes.into());
            let corrupted = candidate.with_parsed(parsed);
            let report =
                verify_candidate(&source, &corrupted, planned.envelope.intent(), &context).unwrap();
            assert!(passed(&report, "exact_samples"));
            assert!(
                !passed(&report, "untouched_objects"),
                "accepted changed object {number}"
            );
            assert!(report.validate(&required()).is_err());
        }
        let mut parsed = candidate.parsed().clone();
        parsed.objects.remove(&reference(19));
        let report = verify_candidate(
            &source,
            &candidate.with_parsed(parsed),
            planned.envelope.intent(),
            &context,
        )
        .unwrap();
        assert!(!passed(&report, "source_graph"));
        assert!(!passed(&report, "untouched_objects"));
    }

    #[test]
    fn forged_unused_nonimage_and_missing_source_roles_cannot_authorize_replacements() {
        let source = fixture();
        let mut parsed = source.parsed().clone();
        let unused = parsed.object(reference(7)).unwrap().clone();
        parsed.objects.insert(reference(23), unused);
        let source = source.with_parsed(parsed);
        let context = context();
        let planned = plan(
            &source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &context,
        )
        .unwrap();
        assert!(
            planned.skips().iter().any(|skip| skip.image.number == 23
                && skip.reason == PdfLosslessImageSkipReason::NotUsed)
        );
        for number in [23, 14, 100] {
            let mut intent = copied_intent(&planned);
            let mut forged = intent.replacements.remove(&reference(7)).unwrap();
            forged.effect.image = public_ref(reference(number));
            intent.replacements.insert(reference(number), forged);
            let report =
                verify_candidate(&source, &staged(&source, &intent), &intent, &context).unwrap();
            assert!(
                !passed(&report, "encoded_candidates"),
                "accepted role {number}"
            );
            assert!(report.validate(&required()).is_err());
            if number != 100 {
                assert!(!passed(&report, "image_interpretation"));
                assert!(!passed(&report, "exact_samples"));
            } else {
                assert!(!passed(&report, "source_graph"));
            }
        }
    }

    #[test]
    fn forged_effect_measurements_and_filter_geometry_fail_checks() {
        let source = fixture();
        let context = context();
        let planned = plan(
            &source,
            SnapshotId::new(source.source_id(), RevisionId::INITIAL),
            &context,
        )
        .unwrap();
        for field in 0..7 {
            let mut intent = copied_intent(&planned);
            let effect = &mut intent.replacements.get_mut(&reference(7)).unwrap().effect;
            match field {
                0 => effect.width += 1,
                1 => effect.height += 1,
                2 => effect.encoded_before += 1,
                3 => effect.encoded_after += 1,
                4 => effect.decoded_bytes += 1,
                5 => effect.source_predictor = 12,
                _ => effect.output_predictor = 1,
            }
            let report =
                verify_candidate(&source, &staged(&source, &intent), &intent, &context).unwrap();
            assert!(!passed(&report, "encoded_candidates"));
            assert!(passed(&report, "exact_samples"));
        }
        let mut intent = copied_intent(&planned);
        let values = dictionary(&mut intent.replacements.get_mut(&reference(7)).unwrap().object);
        let Value::Dict(parameters) = values.get_mut(b"DecodeParms".as_slice()).unwrap() else {
            unreachable!()
        };
        parameters.insert(b"Columns".to_vec(), Value::Integer(95));
        let report =
            verify_candidate(&source, &staged(&source, &intent), &intent, &context).unwrap();
        assert!(!passed(&report, "image_interpretation"));
        assert!(!passed(&report, "exact_samples"));
        assert!(report.validate(&required()).is_err());
    }

    #[test]
    fn unsupported_image_interpretations_have_typed_eligibility_reasons() {
        use PdfLosslessImageSkipReason as Skip;
        let source = fixture();
        let image = source.parsed().object(reference(7)).unwrap();
        let context = context();
        let cases = [
            (
                b"Width".as_slice(),
                Value::Integer(0),
                Skip::UnsupportedImageLayout,
            ),
            (b"Height", Value::Integer(-1), Skip::UnsupportedImageLayout),
            (
                b"BitsPerComponent",
                Value::Integer(16),
                Skip::UnsupportedImageLayout,
            ),
            (
                b"ColorSpace",
                Value::Name(b"DeviceCMYK".to_vec()),
                Skip::UnsupportedColorSpace,
            ),
            (
                b"ColorSpace",
                Value::Ref(reference(20)),
                Skip::UnsupportedColorSpace,
            ),
            (
                b"Filter",
                Value::Array(vec![Value::Name(b"FlateDecode".to_vec())]),
                Skip::UnsupportedFilter,
            ),
            (
                b"Filter",
                Value::Name(b"DCTDecode".to_vec()),
                Skip::UnsupportedFilter,
            ),
            (
                b"Length",
                Value::Ref(reference(20)),
                Skip::DirectLengthRequired,
            ),
            (b"Length", Value::Integer(1), Skip::DirectLengthRequired),
            (b"ImageMask", Value::Bool(true), Skip::ImageMask),
            (b"ImageMask", Value::Ref(reference(20)), Skip::ImageMask),
            (b"SMaskInData", Value::Integer(0), Skip::EmbeddedAlpha),
            (b"F", Value::Null, Skip::ExternalStream),
            (b"FDecodeParms", Value::Null, Skip::ExternalStream),
            (
                b"DecodeParms",
                Value::Ref(reference(20)),
                Skip::DecodeParameters,
            ),
            (
                b"DecodeParms",
                Value::Dict(BTreeMap::from([(
                    b"Predictor".to_vec(),
                    Value::Integer(15),
                )])),
                Skip::DecodeParameters,
            ),
            (
                b"DecodeParms",
                Value::Dict(BTreeMap::from([(
                    b"Predictor".to_vec(),
                    Value::Integer(12),
                )])),
                Skip::DecodeParameters,
            ),
            (
                b"DecodeParms",
                Value::Dict(BTreeMap::from([(b"Colors".to_vec(), Value::Integer(1))])),
                Skip::DecodeParameters,
            ),
            (
                b"DecodeParms",
                Value::Dict(BTreeMap::from([(
                    b"EarlyChange".to_vec(),
                    Value::Integer(0),
                )])),
                Skip::DecodeParameters,
            ),
            (
                b"DecodeParms",
                Value::Dict(BTreeMap::from([(
                    b"PrivateParameter".to_vec(),
                    Value::Integer(1),
                )])),
                Skip::DecodeParameters,
            ),
        ];
        for (key, value, reason) in cases {
            let mut changed = image.clone();
            dictionary(&mut changed).insert(key.to_vec(), value);
            assert_eq!(
                eligibility(&changed, &source, &context).unwrap(),
                Err(reason),
                "key {key:?}"
            );
        }
        let mut explicit = image.clone();
        dictionary(&mut explicit).insert(
            b"DecodeParms".to_vec(),
            Value::Dict(BTreeMap::from([
                (b"Predictor".to_vec(), Value::Integer(1)),
                (b"Colors".to_vec(), Value::Integer(3)),
                (b"BitsPerComponent".to_vec(), Value::Integer(8)),
                (b"Columns".to_vec(), Value::Integer(96)),
                (b"EarlyChange".to_vec(), Value::Integer(1)),
            ])),
        );
        assert_eq!(
            eligibility(&explicit, &source, &context)
                .unwrap()
                .unwrap()
                .predictor,
            1
        );
    }

    #[test]
    fn image_geometry_and_predictor_intermediate_fail_before_decode_allocation() {
        let source = fixture();
        let context = context();
        let image = source.parsed().object(reference(7)).unwrap();
        let layout = eligibility(image, &source, &context).unwrap().unwrap();
        let before = source.decode_metrics();
        assert_eq!(
            predictor_size(layout, layout.sample_bytes)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let mut parsed = source.parsed().clone();
        parsed.limits.max_stream_bytes = layout.sample_bytes;
        let limited = source.with_parsed(parsed);
        let after_layout = Layout {
            predictor: 12,
            ..layout
        };
        assert_eq!(
            decode_samples(
                &limited,
                image,
                after_layout,
                &mut ParseBudget::for_document(limited.parsed())
            )
            .unwrap_err()
            .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(source.decode_metrics(), before);
        assert_eq!(context.budget().snapshot().live_bytes, 0);

        for height in [63, 65] {
            let mut changed = image.clone();
            dictionary(&mut changed).insert(b"Height".to_vec(), Value::Integer(height));
            let layout = eligibility(&changed, &source, &context).unwrap().unwrap();
            assert!(
                decode_samples(
                    &source,
                    &changed,
                    layout,
                    &mut ParseBudget::for_document(source.parsed())
                )
                .is_err()
            );
        }
        assert!(
            predictor_size(
                Layout {
                    sample_bytes: usize::MAX,
                    ..layout
                },
                usize::MAX
            )
            .is_err()
        );
    }

    #[test]
    fn image_work_live_quotas_and_failed_verification_leave_revision_usable() {
        let source = fixture();
        let mut current = state(source.clone());
        let context = context();
        let planned = plan(&source, current.snapshot(), &context).unwrap();
        let probe = self::context();
        assert!(same_metadata(&source, &planned.origin, &probe).unwrap());
        let staging_units = probe.budget().snapshot().work_units + planned.effects().len() as u64;
        let cases = [
            BudgetLimits {
                max_live_bytes: 32 * 1024 * 1024,
                max_work_units: staging_units,
            },
            BudgetLimits {
                max_live_bytes: 0,
                max_work_units: 128 * 1024 * 1024,
            },
        ];
        let before = current.snapshot();
        for limits in cases {
            let limited =
                OperationContext::new(ResourceBudget::new(limits), CancellationToken::new());
            assert!(matches!(
                commit(&mut current, &planned, &limited),
                Err(TransactionError::Check(PdfError {
                    code: PdfErrorCode::ResourceLimit,
                    ..
                }))
            ));
            assert_eq!(current.snapshot(), before);
            assert!(
                current
                    .value()
                    .parsed()
                    .objects
                    .shares_snapshot_with(&source.parsed().objects)
            );
            assert_eq!(limited.budget().snapshot().live_bytes, 0);
            assert!(limited.budget().snapshot().work_units > 0);
        }
        assert!(commit(&mut current, &planned, &context).unwrap().is_some());
    }

    #[test]
    fn dropping_plans_or_evicting_cache_never_refunds_spent_work() {
        let source = fixture();
        let context = context();
        let snapshot = SnapshotId::new(source.source_id(), RevisionId::INITIAL);
        let planned = plan(&source, snapshot, &context).unwrap();
        let spent = context.budget().snapshot().work_units;
        assert!(spent > 0);
        assert!(context.budget().snapshot().live_bytes > 0);
        drop(planned);
        assert_eq!(context.budget().snapshot().live_bytes, 0);
        assert_eq!(context.budget().snapshot().work_units, spent);
        let decode_spent = source.decode_metrics().work_bytes;
        source.clear_decode_cache();
        assert_eq!(source.decode_metrics().work_bytes, decode_spent);
        assert_eq!(source.decode_metrics().live_decoded_bytes, 0);
        let next = plan(&source, snapshot, &context).unwrap();
        assert!(context.budget().snapshot().work_units > spent);
        assert!(source.decode_metrics().work_bytes > decode_spent);
        drop(next);
        let limited = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 32 * 1024 * 1024,
                max_work_units: spent,
            }),
            CancellationToken::new(),
        );
        let first = plan(&source, snapshot, &limited).unwrap();
        assert_eq!(limited.budget().snapshot().work_units, spent);
        drop(first);
        assert_eq!(
            plan(&source, snapshot, &limited).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(limited.budget().snapshot().work_units, spent);
        assert_eq!(limited.budget().snapshot().live_bytes, 0);
    }
}
