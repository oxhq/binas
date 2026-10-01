//! An original Flate-to-Flate pass over supported page/Form streams.
//!
//! This changes encoded storage only. It neither prunes resources nor changes
//! decoded operators, pixels, fonts, metadata, quality policy or standards. The
//! current source/revision is the baseline, including all prior committed edits.
//! Encoded-stream savings are distinct from final serialized PDF size: callers
//! export through the common writer and evaluate final bytes separately.

use std::collections::{BTreeMap, BTreeSet};

use binas_core::{
    OperationContext, Reservation, SnapshotId,
    transaction::{
        CommitReceipt, PlanEnvelope, RevisionState, StagedChange, TransactionError, execute_plan,
    },
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};

use crate::{
    DecodedPayload, PdfDocument, PdfError, PdfErrorCode, PdfReferenceTarget, PdfResourceKind,
    PdfResourceUsage, PdfUsageGap,
    filters::encode_flate_with_context,
    object::{IndirectObject, ObjectRef, Value},
    parser::{ParseBudget, decode_stream},
    payload::StreamPayload,
    resource_usage::census,
    writer::{refuse_security_boundaries_with_context, refuse_stale_linearization_with_context},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfFlateStreamRole {
    PageContent,
    Form,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFlateEffect {
    pub stream: PdfReferenceTarget,
    pub role: PdfFlateStreamRole,
    pub encoded_before: usize,
    pub encoded_after: usize,
    pub decoded_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PdfFlateSkipReason {
    UnsupportedRole,
    NotScanned,
    UnsupportedFilter,
    DecodeParameters,
    ExternalStream,
    DirectLengthRequired,
    OpaqueDictionaryEntry(Vec<u8>),
    NotSmaller,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfFlateSkip {
    pub stream: PdfReferenceTarget,
    pub reason: PdfFlateSkipReason,
}

pub type PdfFlateReceipt = CommitReceipt<Vec<PdfFlateEffect>, String>;

#[derive(Clone, Debug)]
struct Replacement {
    object: IndirectObject,
    decoded: DecodedPayload,
    effect: PdfFlateEffect,
}

#[derive(Debug)]
struct Intent {
    replacements: BTreeMap<ObjectRef, Replacement>,
}

/// Immutable source/branch-bound recompression instructions. Managed encoded
/// payload reservations survive a successful commit through shared storage.
/// Planned metadata records are accounted here; existing parsed objects, codec
/// scratch, allocator overhead and spare capacity remain outside that accounting.
#[derive(Debug)]
pub struct PdfFlatePlan {
    envelope: PlanEnvelope<Intent>,
    origin: PdfDocument,
    effects: Vec<PdfFlateEffect>,
    skips: Vec<PdfFlateSkip>,
    encoded_bytes_saved: u64,
    usage: PdfResourceUsage,
    _reservations: Vec<Reservation>,
}

impl PdfFlatePlan {
    pub fn snapshot(&self) -> SnapshotId {
        self.envelope.snapshot()
    }
    pub fn effects(&self) -> &[PdfFlateEffect] {
        &self.effects
    }
    pub fn skips(&self) -> &[PdfFlateSkip] {
        &self.skips
    }
    /// Sum of reductions in selected encoded streams; not final PDF savings.
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
) -> Result<PdfFlatePlan, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if snapshot.source() != document.source_id() {
        return Err(PdfError::selection("Flate plan belongs to another source"));
    }
    refuse_security_boundaries_with_context(document.parsed(), Some(context))?;
    refuse_stale_linearization_with_context(document.parsed(), Some(context))?;
    let mut reservations = Vec::new();
    let metadata_bytes = value_size(&document.parsed().trailer, document, context)?
        .checked_add(document.parsed().version.len())
        .ok_or_else(|| PdfError::limit("Flate plan source metadata overflows"))?;
    reserve(&mut reservations, metadata_bytes, context)?;
    let document = document.with_cancellation(context.cancellation().clone());
    let usage = census(&document, snapshot, None, context)?;
    let workspace_bytes = document
        .parsed()
        .objects
        .len()
        .checked_mul(size_of::<(ObjectRef, PdfFlateStreamRole)>() + 2 * size_of::<ObjectRef>())
        .ok_or_else(|| PdfError::limit("Flate stream-selection workspace overflows"))?;
    let _workspace = context
        .reserve(workspace_bytes as u64)
        .map_err(PdfError::from_context)?;
    let roles = stream_roles(&document, &usage, context)?;
    let mut scanned = BTreeSet::new();
    for reference in &usage.scanned_streams {
        context.charge_work(1).map_err(PdfError::from_context)?;
        scanned.insert(object_ref(*reference));
    }
    let mut budget = ParseBudget::for_document(document.parsed());
    let mut replacements = BTreeMap::new();
    let mut effects = Vec::new();
    let mut skips = Vec::new();
    let mut encoded_bytes_saved = 0_u64;
    // Ordered stream inventory makes pass order and skip reporting deterministic.
    for (reference, before) in &document.parsed().objects {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Some(encoded) = before.stream.as_ref() else {
            continue;
        };
        if skips.len() + effects.len() >= document.parsed().limits.max_container_items {
            return Err(PdfError::limit("Flate plan stream inventory exceeds limit"));
        }
        let role = roles.get(reference).copied();
        let skip = if let Some(role) = role {
            eligibility(before, role, &mut reservations, context)?.or_else(|| {
                (!scanned.contains(reference)).then_some(PdfFlateSkipReason::NotScanned)
            })
        } else {
            Some(PdfFlateSkipReason::UnsupportedRole)
        };
        if let Some(reason) = skip {
            push_skip(&mut skips, &mut reservations, *reference, reason, context)?;
            continue;
        }
        let decoded = decode_stream(
            &before.value,
            encoded,
            &document.parsed().limits,
            &mut budget,
        )?;
        let (encoded_candidate, encoded_reservations) = encode_flate_with_context(
            &decoded,
            document.parsed().limits.max_stream_bytes,
            context,
        )?;
        if encoded_candidate.len() >= encoded.len() {
            push_skip(
                &mut skips,
                &mut reservations,
                *reference,
                PdfFlateSkipReason::NotSmaller,
                context,
            )?;
            continue;
        }
        let encoded_after = encoded_candidate.len();
        let effect = PdfFlateEffect {
            stream: public_ref(*reference),
            role: role.expect("eligible role checked above"),
            encoded_before: encoded.len(),
            encoded_after,
            decoded_bytes: decoded.len(),
        };
        let record_bytes = value_size(&before.value, &document, context)?
            .checked_add(
                size_of::<Replacement>() + size_of::<ObjectRef>() + size_of::<PdfFlateEffect>(),
            )
            .ok_or_else(|| PdfError::limit("Flate plan replacement metadata overflows"))?;
        reserve(&mut reservations, record_bytes, context)?;
        let mut object = before.clone();
        let Value::Dict(dictionary) = &mut object.value else {
            unreachable!("eligibility requires a stream dictionary")
        };
        dictionary.insert(
            b"Length".to_vec(),
            Value::Integer(
                i64::try_from(encoded_after)
                    .map_err(|_| PdfError::limit("encoded length exceeds i64"))?,
            ),
        );
        object.stream = Some(StreamPayload::from_reserved(
            encoded_candidate,
            encoded_reservations,
        )?);
        encoded_bytes_saved = encoded_bytes_saved
            .checked_add((encoded.len() - encoded_after) as u64)
            .ok_or_else(|| PdfError::limit("Flate encoded savings overflow"))?;
        effects.push(effect.clone());
        replacements.insert(
            *reference,
            Replacement {
                object,
                decoded,
                effect,
            },
        );
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(PdfFlatePlan {
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
    plan: &PdfFlatePlan,
    context: &OperationContext,
) -> Result<Option<PdfFlateReceipt>, TransactionError<PdfError>> {
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
            "Flate plan object-store/source snapshot mismatch",
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
                context.charge_work(1).map_err(PdfError::from_context)?;
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
            // A prior operation may have cancelled its token. New work changes
            // the token while retaining the same spent document decode budget.
            let original = original.with_cancellation(context.cancellation().clone());
            verify_candidate(&original, candidate, intent, context)
        },
    )
    .map(Some)
}

fn stream_roles(
    document: &PdfDocument,
    usage: &PdfResourceUsage,
    context: &OperationContext,
) -> Result<BTreeMap<ObjectRef, PdfFlateStreamRole>, PdfError> {
    let mut roles = BTreeMap::new();
    for page in document.page_refs()? {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let references = match document.page_content_refs(page) {
            Ok(references) => references,
            Err(error)
                if matches!(
                    error.code,
                    PdfErrorCode::InvalidSyntax | PdfErrorCode::UnsupportedFeature
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        for reference in references {
            context.charge_work(1).map_err(PdfError::from_context)?;
            if document.parsed().objects.contains_key(&reference) {
                roles.insert(reference, PdfFlateStreamRole::PageContent);
            }
        }
    }
    let mut excluded = BTreeSet::new();
    for usage in &usage.uses {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Some(target) = usage.target else {
            continue;
        };
        let target = object_ref(target);
        if !document.parsed().objects.contains_key(&target) {
            continue;
        }
        match usage.kind {
            PdfResourceKind::Form if !usage.appearance => {
                roles.entry(target).or_insert(PdfFlateStreamRole::Form);
            }
            PdfResourceKind::Form => {
                excluded.insert(target);
            }
            PdfResourceKind::Image
            | PdfResourceKind::Appearance
            | PdfResourceKind::Type3Glyph
            | PdfResourceKind::SoftMask
            | PdfResourceKind::Pattern => {
                excluded.insert(target);
            }
            _ => {}
        }
    }
    for reference in excluded {
        roles.remove(&reference);
    }
    Ok(roles)
}

fn eligibility(
    object: &IndirectObject,
    role: PdfFlateStreamRole,
    reservations: &mut Vec<Reservation>,
    context: &OperationContext,
) -> Result<Option<PdfFlateSkipReason>, PdfError> {
    let Value::Dict(dictionary) = &object.value else {
        return Ok(Some(PdfFlateSkipReason::UnsupportedRole));
    };
    if [b"F".as_slice(), b"FFilter", b"FDecodeParms", b"Ref"]
        .iter()
        .any(|key| dictionary.contains_key(*key))
    {
        return Ok(Some(PdfFlateSkipReason::ExternalStream));
    }
    if !matches!(dictionary.get(b"Filter".as_slice()), Some(Value::Name(name)) if name == b"FlateDecode")
    {
        return Ok(Some(PdfFlateSkipReason::UnsupportedFilter));
    }
    if !matches!(
        dictionary.get(b"DecodeParms".as_slice()),
        None | Some(Value::Null)
    ) {
        return Ok(Some(PdfFlateSkipReason::DecodeParameters));
    }
    if !matches!(dictionary.get(b"Length".as_slice()), Some(Value::Integer(length))
        if usize::try_from(*length).ok() == object.stream.as_ref().map(|stream| stream.len()))
    {
        return Ok(Some(PdfFlateSkipReason::DirectLengthRequired));
    }
    if role == PdfFlateStreamRole::Form
        && (dictionary
            .get(b"Type".as_slice())
            .is_some_and(|value| !matches!(value, Value::Name(name) if name == b"XObject"))
            || !matches!(dictionary.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Form"))
    {
        return Ok(Some(PdfFlateSkipReason::UnsupportedRole));
    }
    for key in dictionary.keys() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let common = matches!(key.as_slice(), b"Length" | b"Filter" | b"DecodeParms");
        let form = role == PdfFlateStreamRole::Form
            && matches!(
                key.as_slice(),
                b"Type"
                    | b"Subtype"
                    | b"FormType"
                    | b"BBox"
                    | b"Matrix"
                    | b"Resources"
                    | b"Group"
                    | b"StructParent"
                    | b"StructParents"
                    | b"OC"
            );
        if !common && !form {
            reserve(reservations, key.len(), context)?;
            return Ok(Some(PdfFlateSkipReason::OpaqueDictionaryEntry(key.clone())));
        }
    }
    Ok(None)
}

const CHECKS: [&str; 6] = [
    "source_graph",
    "untouched_objects",
    "requested_encoded_payloads",
    "decoded_streams_unchanged",
    "encoded_streams_smaller",
    "page_relationships",
];
fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.optimize.flate", name).expect("static Flate check code")
}
fn required() -> RequiredChecks {
    RequiredChecks::new(CHECKS.into_iter().map(code).collect()).expect("static Flate obligations")
}

fn verify_candidate(
    original: &PdfDocument,
    candidate: &PdfDocument,
    intent: &Intent,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    let graph = same_metadata(original, candidate, context)?
        && original.parsed().objects.len() == candidate.parsed().objects.len();
    let mut untouched = true;
    let mut requested = true;
    let mut decoded_equal = true;
    let mut smaller = true;
    let mut source_budget = ParseBudget::for_document(original.parsed());
    let mut candidate_budget = ParseBudget::for_document(candidate.parsed());
    for (reference, before) in &original.parsed().objects {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Some(after) = candidate.parsed().objects.get(reference) else {
            untouched = false;
            requested = false;
            continue;
        };
        let Some(replacement) = intent.replacements.get(reference) else {
            untouched &=
                std::ptr::eq(before, after) && same_object(before, after, original, context)?;
            continue;
        };
        requested &= same_object(after, &replacement.object, original, context)?
            && same_stream_dictionary(&before.value, &after.value, original, context)?;
        let (Some(before_bytes), Some(after_bytes)) = (&before.stream, &after.stream) else {
            requested = false;
            smaller = false;
            decoded_equal = false;
            continue;
        };
        requested &= matches!(&after.value, Value::Dict(dictionary)
            if matches!(dictionary.get(b"Length".as_slice()), Some(Value::Integer(length))
                if usize::try_from(*length).ok() == Some(after_bytes.len())));
        smaller &= after_bytes.len() < before_bytes.len()
            && replacement.effect.encoded_before == before_bytes.len()
            && replacement.effect.encoded_after == after_bytes.len();
        let before_decoded = decode_stream(
            &before.value,
            before_bytes,
            &original.parsed().limits,
            &mut source_budget,
        )?;
        let after_decoded = decode_stream(
            &after.value,
            after_bytes,
            &candidate.parsed().limits,
            &mut candidate_budget,
        )?;
        decoded_equal &= bytes_equal(&before_decoded, &after_decoded, context)?
            && bytes_equal(&before_decoded, &replacement.decoded, context)?
            && replacement.effect.decoded_bytes == before_decoded.len();
    }
    for reference in intent.replacements.keys() {
        context.charge_work(1).map_err(PdfError::from_context)?;
        requested &= original.parsed().objects.contains_key(reference)
            && candidate.parsed().objects.contains_key(reference);
    }
    let pages = original.page_refs()? == candidate.page_refs()?;
    let outcomes = [graph, untouched, requested, decoded_equal, smaller, pages];
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(EvidenceReport::new(CHECKS.into_iter().zip(outcomes).map(|(name, passed)| CheckEvidence {
        code: code(name),
        status: if passed { CheckStatus::Passed } else { CheckStatus::Failed },
        detail: "Exact current-source graph and decoded-stream checks. Encoded stream savings only; final PDF size, rendering and conformance are not evaluated.".into(),
    }).collect()))
}

pub(super) fn same_metadata(
    left: &PdfDocument,
    right: &PdfDocument,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    Ok(left.source_id() == right.source_id()
        && left.parsed().version == right.parsed().version
        && left.parsed().source_size == right.parsed().source_size
        && left.parsed().xref_revisions == right.parsed().xref_revisions
        && values_equal(
            &left.parsed().trailer,
            &right.parsed().trailer,
            0,
            left,
            context,
        )?)
}

pub(super) fn same_object(
    left: &IndirectObject,
    right: &IndirectObject,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    let streams = match (&left.stream, &right.stream) {
        (None, None) => true,
        (Some(left), Some(right)) => bytes_equal(left, right, context)?,
        _ => false,
    };
    Ok(streams
        && left.offset == right.offset
        && left.stream_offset == right.stream_offset
        && values_equal(&left.value, &right.value, 0, document, context)?)
}

fn same_stream_dictionary(
    left: &Value,
    right: &Value,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    let (Value::Dict(left), Value::Dict(right)) = (left, right) else {
        return Ok(false);
    };
    if left.len() != right.len() {
        return Ok(false);
    }
    for (key, value) in left {
        context.charge_work(1).map_err(PdfError::from_context)?;
        let Some(other) = right.get(key) else {
            return Ok(false);
        };
        if key != b"Length" && !values_equal(value, other, 1, document, context)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn values_equal(
    left: &Value,
    right: &Value,
    depth: usize,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context.charge_work(1).map_err(PdfError::from_context)?;
    if depth > document.parsed().limits.max_parser_depth {
        return Err(PdfError::limit("Flate comparison depth exceeds limit"));
    }
    match (left, right) {
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                return Ok(false);
            }
            for (left, right) in left.iter().zip(right) {
                if !values_equal(left, right, depth + 1, document, context)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Dict(left), Value::Dict(right)) => {
            if left.len() != right.len() {
                return Ok(false);
            }
            for ((key, value), (other_key, other_value)) in left.iter().zip(right) {
                if !bytes_equal(key, other_key, context)?
                    || !values_equal(value, other_value, depth + 1, document, context)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Value::Name(left), Value::Name(right)) | (Value::String(left), Value::String(right)) => {
            bytes_equal(left, right, context)
        }
        _ => Ok(left == right),
    }
}

pub(super) fn bytes_equal(
    left: &[u8],
    right: &[u8],
    context: &OperationContext,
) -> Result<bool, PdfError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    for (left, right) in left.chunks(8192).zip(right.chunks(8192)) {
        context
            .charge_work(left.len() as u64)
            .map_err(PdfError::from_context)?;
        if left != right {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn value_size(
    value: &Value,
    document: &PdfDocument,
    context: &OperationContext,
) -> Result<usize, PdfError> {
    fn walk(
        value: &Value,
        depth: usize,
        document: &PdfDocument,
        context: &OperationContext,
    ) -> Result<usize, PdfError> {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if depth > document.parsed().limits.max_parser_depth {
            return Err(PdfError::limit("Flate metadata depth exceeds limit"));
        }
        let mut size = size_of::<Value>();
        let mut add = |bytes: usize| -> Result<(), PdfError> {
            size = size
                .checked_add(bytes)
                .ok_or_else(|| PdfError::limit("Flate metadata size overflows"))?;
            Ok(())
        };
        match value {
            Value::Name(bytes) | Value::String(bytes) => {
                context
                    .charge_work(bytes.len() as u64)
                    .map_err(PdfError::from_context)?;
                add(bytes.len())?;
            }
            Value::Array(values) => {
                for value in values {
                    add(walk(value, depth + 1, document, context)?)?;
                }
            }
            Value::Dict(values) => {
                for (key, value) in values {
                    context
                        .charge_work(key.len() as u64)
                        .map_err(PdfError::from_context)?;
                    add(size_of::<Vec<u8>>())?;
                    add(key.len())?;
                    add(walk(value, depth + 1, document, context)?)?;
                }
            }
            _ => {}
        }
        Ok(size)
    }
    walk(value, 0, document, context)
}

pub(super) fn reserve(
    reservations: &mut Vec<Reservation>,
    bytes: usize,
    context: &OperationContext,
) -> Result<(), PdfError> {
    reservations.push(
        context
            .reserve(bytes as u64)
            .map_err(PdfError::from_context)?,
    );
    Ok(())
}

fn push_skip(
    skips: &mut Vec<PdfFlateSkip>,
    reservations: &mut Vec<Reservation>,
    reference: ObjectRef,
    reason: PdfFlateSkipReason,
    context: &OperationContext,
) -> Result<(), PdfError> {
    reserve(reservations, size_of::<PdfFlateSkip>(), context)?;
    skips.push(PdfFlateSkip {
        stream: public_ref(reference),
        reason,
    });
    Ok(())
}

fn public_ref(reference: ObjectRef) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number: reference.number,
        generation: reference.generation,
    }
}
fn object_ref(reference: PdfReferenceTarget) -> ObjectRef {
    ObjectRef {
        number: reference.number,
        generation: reference.generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfWriterMode, writer::export_staged_document};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget, RevisionId};
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 10_000_000,
                max_work_units: 100_000_000,
            }),
            CancellationToken::new(),
        )
    }

    fn weak_flate(input: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::none());
        encoder.write_all(input).unwrap();
        encoder.finish().unwrap()
    }

    fn stream(dictionary: &str, bytes: &[u8]) -> Vec<u8> {
        [
            format!("<< /Length {} {dictionary} >>\nstream\n", bytes.len()).as_bytes(),
            bytes,
            b"\nendstream",
        ]
        .concat()
    }

    fn fixture() -> (PdfDocument, Vec<u8>, Vec<u8>) {
        let page = [b"q /Fm Do Q\n".as_slice(), b"q Q\n".repeat(1024).as_slice()].concat();
        let form = b"0 0 m 10 10 l S\n".repeat(256);
        let objects = [
            b"<< /Type /Catalog /Pages 2 0 R /Metadata 6 0 R /Private 7 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 100 100] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /XObject << /Fm 5 0 R >> >> >>".to_vec(),
            stream("/Filter /FlateDecode", &weak_flate(&page)),
            stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << >> /Filter /FlateDecode", &weak_flate(&form)),
            stream("/Type /Metadata /Subtype /XML /Filter /FlateDecode", &weak_flate(b"<original>Keep exact metadata</original>")),
            b"<< /Title <FEFF03A9> /PrivateValue (preserve) >>".to_vec(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(format!("trailer\n<< /Size 8 /Root 1 0 R /Info 7 0 R /PrivateFlag true >>\nstartxref\n{xref}\n%%EOF\n").as_bytes());
        let document = PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap();
        (document, page, form)
    }

    fn state(document: PdfDocument) -> RevisionState<PdfDocument> {
        RevisionState::new(
            SnapshotId::new(document.source_id(), RevisionId::INITIAL),
            document,
        )
    }

    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }

    fn decoded(document: &PdfDocument, number: u32) -> DecodedPayload {
        let object = document.parsed().object(reference(number)).unwrap();
        decode_stream(
            &object.value,
            object.stream.as_ref().unwrap(),
            &document.parsed().limits,
            &mut ParseBudget::for_document(document.parsed()),
        )
        .unwrap()
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

    fn passed(report: &EvidenceReport<String>, name: &str) -> bool {
        report
            .checks()
            .iter()
            .any(|check| check.code == code(name) && check.status == CheckStatus::Passed)
    }

    #[test]
    fn flate_commit_recompresses_page_and_form_and_preserves_final_graph() {
        let (document, page, form) = fixture();
        let original = document.clone();
        let mut current = state(document);
        let context = context();
        let plan = plan(current.value(), current.snapshot(), &context).unwrap();
        assert_eq!(
            plan.effects()
                .iter()
                .map(|effect| effect.stream.number)
                .collect::<Vec<_>>(),
            [4, 5]
        );
        assert_eq!(plan.effects()[0].role, PdfFlateStreamRole::PageContent);
        assert_eq!(plan.effects()[1].role, PdfFlateStreamRole::Form);
        assert!(
            plan.effects()
                .iter()
                .all(|effect| effect.encoded_after < effect.encoded_before)
        );
        assert!(plan.encoded_bytes_saved() > 0);
        assert!(
            plan.skips().iter().any(|skip| skip.stream.number == 6
                && skip.reason == PdfFlateSkipReason::UnsupportedRole)
        );
        let after_bytes = plan
            .effects()
            .iter()
            .map(|effect| effect.encoded_after as u64)
            .sum::<u64>();
        let receipt = commit(&mut current, &plan, &context).unwrap().unwrap();
        assert_ne!(receipt.before, receipt.after);
        receipt.evidence.validate(&receipt.required).unwrap();
        assert_eq!(&*decoded(current.value(), 4), page);
        assert_eq!(&*decoded(current.value(), 5), form);
        assert!(std::ptr::eq(
            original.parsed().objects.get(&reference(6)).unwrap(),
            current.value().parsed().objects.get(&reference(6)).unwrap()
        ));
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            let exported =
                export_staged_document(current.value(), current.snapshot(), mode, &context)
                    .unwrap();
            let reopened = PdfEngine::default()
                .open_source(exported.output.clone(), OpenOptions::default())
                .unwrap();
            assert_eq!(&*decoded(&reopened, 4), page);
            assert_eq!(&*decoded(&reopened, 5), form);
            assert_eq!(
                original.parsed().object(reference(6)).unwrap().stream,
                reopened.parsed().object(reference(6)).unwrap().stream
            );
            assert_eq!(
                original.parsed().object(reference(7)).unwrap().value,
                reopened.parsed().object(reference(7)).unwrap().value
            );
            if mode == PdfWriterMode::Incremental {
                assert!(
                    exported
                        .output
                        .as_bytes()
                        .starts_with(original.input_source().as_bytes())
                );
            }
        }
        drop(plan);
        // Only committed encoded allocations retain this operation's charges.
        assert_eq!(context.budget().snapshot().live_bytes, after_bytes);
        drop(current);
        assert_eq!(context.budget().snapshot().live_bytes, 0);
    }

    #[test]
    fn flate_repeat_is_a_noop_without_advancing_revision() {
        let (document, _, _) = fixture();
        let mut current = state(document);
        let context = context();
        let first = plan(current.value(), current.snapshot(), &context).unwrap();
        commit(&mut current, &first, &context).unwrap().unwrap();
        let before = current.snapshot();
        let repeat = plan(current.value(), before, &context).unwrap();
        assert!(repeat.effects().is_empty());
        assert_eq!(repeat.encoded_bytes_saved(), 0);
        assert_eq!(
            repeat
                .skips()
                .iter()
                .filter(|skip| skip.reason == PdfFlateSkipReason::NotSmaller)
                .count(),
            2
        );
        assert!(commit(&mut current, &repeat, &context).unwrap().is_none());
        assert_eq!(current.snapshot(), before);
    }

    #[test]
    fn flate_plan_binds_source_branch_metadata_and_cancellation() {
        let (document, _, _) = fixture();
        let mut current = state(document.clone());
        let context = context();
        let planned = plan(current.value(), current.snapshot(), &context).unwrap();
        let mut changed = document.parsed().clone();
        let Value::Dict(dictionary) = &mut changed.objects.get_mut(&reference(7)).unwrap().value
        else {
            unreachable!()
        };
        dictionary.insert(b"OtherBranch".to_vec(), Value::Bool(true));
        let mut sibling = RevisionState::new(current.snapshot(), document.with_parsed(changed));
        assert!(matches!(
            commit(&mut sibling, &planned, &context),
            Err(TransactionError::Stage(PdfError {
                code: PdfErrorCode::UnsafeRewrite,
                ..
            }))
        ));
        let mut changed = document.parsed().clone();
        let Value::Dict(dictionary) = &mut changed.trailer else {
            unreachable!()
        };
        dictionary.insert(b"OtherBranch".to_vec(), Value::Bool(true));
        let mut metadata_sibling =
            RevisionState::new(current.snapshot(), document.with_parsed(changed));
        assert!(matches!(
            commit(&mut metadata_sibling, &planned, &context),
            Err(TransactionError::Stage(PdfError {
                code: PdfErrorCode::UnsafeRewrite,
                ..
            }))
        ));
        let mut other_bytes = document.input_source().as_bytes().to_vec();
        let position = other_bytes
            .windows(b"preserve".len())
            .position(|value| value == b"preserve")
            .unwrap();
        other_bytes[position] = b'P';
        let other = PdfEngine::default()
            .open_owned(other_bytes, OpenOptions::default())
            .unwrap();
        let mut other = state(other);
        assert!(matches!(
            commit(&mut other, &planned, &context),
            Err(TransactionError::StalePlan { .. })
        ));
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
            commit(&mut current, &planned, &self::context())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn flate_uses_current_edited_payload_as_its_baseline() {
        let (document, page, _) = fixture();
        let edited = [page.as_slice(), b"% required previous edit\nq Q\n"].concat();
        let mut parsed = document.parsed().clone();
        let object = parsed.objects.get_mut(&reference(4)).unwrap();
        let bytes = weak_flate(&edited);
        let Value::Dict(dictionary) = &mut object.value else {
            unreachable!()
        };
        dictionary.insert(b"Length".to_vec(), Value::Integer(bytes.len() as i64));
        object.stream = Some(bytes.into());
        let snapshot = SnapshotId::new(
            document.source_id(),
            RevisionId::INITIAL.checked_next().unwrap(),
        );
        let mut current = RevisionState::new(snapshot, document.with_parsed(parsed));
        let context = context();
        let planned = plan(current.value(), snapshot, &context).unwrap();
        commit(&mut current, &planned, &context).unwrap().unwrap();
        assert_eq!(&*decoded(current.value(), 4), edited);
        let exported = export_staged_document(
            current.value(),
            current.snapshot(),
            PdfWriterMode::Compact,
            &context,
        )
        .unwrap();
        let reopened = PdfEngine::default()
            .open_source(exported.output, OpenOptions::default())
            .unwrap();
        assert_eq!(&*decoded(&reopened, 4), edited);
    }

    #[test]
    fn flate_verifier_rejects_self_consistent_payload_loss_and_unrequested_changes() {
        let (document, _, _) = fixture();
        let context = context();
        let snapshot = SnapshotId::new(document.source_id(), RevisionId::INITIAL);
        let planned = plan(&document, snapshot, &context).unwrap();
        let mut intent = Intent {
            replacements: planned.envelope.intent().replacements.clone(),
        };
        let replacement = intent.replacements.get_mut(&reference(4)).unwrap();
        let false_bytes = weak_flate(b"q Q\n");
        let Value::Dict(dictionary) = &mut replacement.object.value else {
            unreachable!()
        };
        dictionary.insert(b"Length".to_vec(), Value::Integer(false_bytes.len() as i64));
        replacement.object.stream = Some(false_bytes.into());
        replacement.decoded = decode_stream(
            &replacement.object.value,
            replacement.object.stream.as_ref().unwrap(),
            &document.parsed().limits,
            &mut ParseBudget::for_document(document.parsed()),
        )
        .unwrap();
        replacement.effect.encoded_after = replacement.object.stream.as_ref().unwrap().len();
        replacement.effect.decoded_bytes = replacement.decoded.len();
        let bad = staged(&document, &intent);
        let report = verify_candidate(&document, &bad, &intent, &context).unwrap();
        assert!(passed(&report, "requested_encoded_payloads"));
        assert!(passed(&report, "encoded_streams_smaller"));
        assert!(!passed(&report, "decoded_streams_unchanged"));
        assert!(report.validate(&required()).is_err());
        let candidate = staged(&document, planned.envelope.intent());
        let mut missing = candidate.parsed().clone();
        missing.objects.remove(&reference(7));
        let report = verify_candidate(
            &document,
            &candidate.with_parsed(missing),
            planned.envelope.intent(),
            &context,
        )
        .unwrap();
        assert!(!passed(&report, "source_graph"));
        assert!(!passed(&report, "untouched_objects"));
        let mut intent = Intent {
            replacements: planned.envelope.intent().replacements.clone(),
        };
        let Value::Dict(dictionary) = &mut intent
            .replacements
            .get_mut(&reference(4))
            .unwrap()
            .object
            .value
        else {
            unreachable!()
        };
        dictionary.insert(b"Unrequested".to_vec(), Value::Bool(true));
        let bad = staged(&document, &intent);
        let report = verify_candidate(&document, &bad, &intent, &context).unwrap();
        assert!(!passed(&report, "requested_encoded_payloads"));
        assert!(passed(&report, "decoded_streams_unchanged"));
    }

    #[test]
    fn flate_limits_fail_without_a_partial_commit() {
        let (document, _, _) = fixture();
        let mut current = state(document);
        let planned = plan(current.value(), current.snapshot(), &context()).unwrap();
        let before = current.snapshot();
        let zero = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: 0,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            plan(current.value(), before, &zero).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert!(commit(&mut current, &planned, &zero).is_err());
        assert_eq!(current.snapshot(), before);
        assert!(
            current
                .value()
                .parsed()
                .objects
                .shares_snapshot_with(&planned.origin.parsed().objects)
        );
        assert_eq!(zero.budget().snapshot().live_bytes, 0);
    }

    #[test]
    fn flate_preserves_explicit_skip_reasons_for_unsupported_stream_dictionaries() {
        let (document, _, _) = fixture();
        let cases = [
            (
                b"Filter".as_slice(),
                Value::Array(vec![Value::Name(b"FlateDecode".to_vec())]),
                PdfFlateSkipReason::UnsupportedFilter,
            ),
            (
                b"Filter",
                Value::Name(b"DCTDecode".to_vec()),
                PdfFlateSkipReason::UnsupportedFilter,
            ),
            (
                b"DecodeParms",
                Value::Dict(BTreeMap::from([(b"Predictor".to_vec(), Value::Integer(1))])),
                PdfFlateSkipReason::DecodeParameters,
            ),
            (
                b"F",
                Value::String(b"external-file".to_vec()),
                PdfFlateSkipReason::ExternalStream,
            ),
            (
                b"Length",
                Value::Ref(reference(7)),
                PdfFlateSkipReason::DirectLengthRequired,
            ),
            (
                b"PrivateStreamKey",
                Value::Bool(true),
                PdfFlateSkipReason::OpaqueDictionaryEntry(b"PrivateStreamKey".to_vec()),
            ),
        ];
        for (key, value, reason) in cases {
            let mut parsed = document.parsed().clone();
            let Value::Dict(dictionary) = &mut parsed.objects.get_mut(&reference(4)).unwrap().value
            else {
                unreachable!()
            };
            dictionary.insert(key.to_vec(), value);
            let document = document.with_parsed(parsed);
            let snapshot = SnapshotId::new(document.source_id(), RevisionId::INITIAL);
            let planned = plan(&document, snapshot, &context()).unwrap();
            assert!(
                planned
                    .effects()
                    .iter()
                    .all(|effect| effect.stream.number != 4)
            );
            assert!(
                planned
                    .skips()
                    .iter()
                    .any(|skip| skip.stream.number == 4 && skip.reason == reason),
                "missing skip: {reason:?}"
            );
        }
    }
}
