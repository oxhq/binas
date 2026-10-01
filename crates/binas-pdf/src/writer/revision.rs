//! One final-byte boundary for retained PDF object revisions. These checks
//! compare typed values and encoded payloads; they do not validate appearance,
//! accessibility, signatures or standards, or interpret opaque extension data.

use std::collections::BTreeMap;

use super::{
    Output, append_object_revisions_with_context, is_xref_layout_key,
    refuse_security_boundaries_with_context, refuse_stale_linearization_with_context,
    require_classic_offset, write_classic_trailer, write_object,
};
use crate::{
    OpenOptions, PdfDocument, PdfEngine, PdfError,
    object::{IndirectObject, Value},
};
use binas_core::{
    OperationContext, SnapshotId, Source,
    verification::{CheckCode, CheckEvidence, CheckStatus, EvidenceReport, RequiredChecks},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdfWriterMode {
    /// Reserved for plans whose complete changes have exact replaceable spans.
    Surgical,
    Incremental,
    Compact,
}

#[derive(Debug)]
pub struct PdfRevisionExport {
    pub snapshot: SnapshotId,
    /// Immutable final bytes and their cached SHA-256 identity.
    pub output: Source,
    pub mode: PdfWriterMode,
    pub engine_version: &'static str,
    pub evidence: EvidenceReport<String>,
}

pub(crate) fn export_staged_document(
    document: &PdfDocument,
    snapshot: SnapshotId,
    mode: PdfWriterMode,
    context: &OperationContext,
) -> Result<PdfRevisionExport, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if snapshot.source() != document.source_id() {
        return Err(PdfError::selection(
            "writer snapshot belongs to another source",
        ));
    }
    if mode == PdfWriterMode::Surgical {
        return Err(PdfError::unsupported(
            "retained object deltas have no surgical byte-range writer plan",
        ));
    }
    // The writer enforces max_output_bytes before extending its buffer. This
    // context adds coarse byte/object work; it is not complete allocator accounting.
    context
        .charge_work(document.source_len() as u64)
        .map_err(PdfError::from_context)?;
    let output = match mode {
        PdfWriterMode::Incremental => {
            let mut replacements = Vec::new();
            for (reference, object) in document.parsed().objects.changes() {
                context.charge_work(1).map_err(PdfError::from_context)?;
                let object = object.ok_or_else(|| {
                    PdfError::unsupported(
                        "incremental export cannot remove historical objects; select compact",
                    )
                })?;
                replacements.push((reference, &object.value, object.stream.as_deref()));
            }
            if replacements.is_empty() {
                refuse_security_boundaries_with_context(document.parsed(), Some(context))?;
                if document.source_len() > document.engine_config().limits.max_output_bytes {
                    return Err(PdfError::limit("output exceeds max_output_bytes"));
                }
                document.input_source().clone()
            } else {
                source_from_output(
                    document,
                    append_object_revisions_with_context(document, &replacements, Some(context))?,
                    context,
                )?
            }
        }
        PdfWriterMode::Compact => source_from_output(
            document,
            write_compact_document(document, Some(context))?.0,
            context,
        )?,
        PdfWriterMode::Surgical => unreachable!("refused above"),
    };
    context
        .charge_work(output.len() as u64)
        .map_err(PdfError::from_context)?;
    let evidence = verify_serialized_output(document, &output, mode, context)?;
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(PdfRevisionExport {
        snapshot,
        output,
        mode,
        engine_version: env!("CARGO_PKG_VERSION"),
        evidence,
    })
}

fn verify_serialized_output(
    document: &PdfDocument,
    output: &Source,
    mode: PdfWriterMode,
    context: &OperationContext,
) -> Result<EvidenceReport<String>, PdfError> {
    let mut config = document.engine_config().clone();
    // Internal reparse is bounded by the configured OUTPUT limit, since an
    // accepted transformation can legitimately exceed its original input limit.
    config.limits.max_input_bytes = config.limits.max_output_bytes;
    let reopened = PdfEngine::new(config).open_source_with_cancellation(
        output.clone(),
        OpenOptions::default(),
        context.cancellation().clone(),
    )?;
    let objects = document.parsed().objects.len() == reopened.parsed().objects.len()
        && document
            .parsed()
            .objects
            .iter()
            .try_fold(true, |equal, (reference, before)| {
                context.charge_work(1).map_err(PdfError::from_context)?;
                let Some(after) = reopened.parsed().objects.get(reference) else {
                    return Ok(false);
                };
                Ok::<_, PdfError>(
                    equal && same_written_object(before, after, document, &reopened, context)?,
                )
            })?;
    let trailers = same_trailer_values(&document.parsed().trailer, &reopened.parsed().trailer);
    let pages = document.page_refs()? == reopened.page_refs()?;
    let references = crate::PdfReferenceIndex::build(&reopened, context)?
        .records()
        .iter()
        .all(|record| !record.dangling);
    let version = document.parsed().version == reopened.parsed().version;
    let writer_mode = match mode {
        PdfWriterMode::Incremental => {
            output.as_bytes().starts_with(document.source())
                && document
                    .parsed()
                    .xref_revisions
                    .checked_add(usize::from(document.parsed().objects.changes().len() != 0))
                    == Some(reopened.parsed().xref_revisions)
        }
        PdfWriterMode::Compact => reopened.parsed().xref_revisions == 1,
        PdfWriterMode::Surgical => false,
    };
    let outcomes = [
        ("reparse", true),
        ("header_version", version),
        ("objects_and_encoded_payloads", objects),
        ("trailer_values", trailers),
        ("references_resolve", references),
        ("page_relationships", pages),
        ("writer_mode", writer_mode),
    ];
    let required = RequiredChecks::new(outcomes.iter().map(|(name, _)| code(name)).collect())
        .expect("static writer obligations");
    let evidence = EvidenceReport::new(outcomes.into_iter().map(|(name, passed)| CheckEvidence {
        code: code(name), status: if passed { CheckStatus::Passed } else { CheckStatus::Failed },
        detail: "final typed graph/encoded-byte check; opaque extension meaning, appearance and conformance not checked".into(),
    }).collect());
    evidence
        .validate(&required)
        .map_err(|error| PdfError::verification(error.to_string()))?;
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(evidence)
}

fn code(name: &str) -> CheckCode {
    CheckCode::new("pdf.writer", name).expect("static check code")
}
fn source_from_output(
    document: &PdfDocument,
    bytes: Vec<u8>,
    context: &OperationContext,
) -> Result<Source, PdfError> {
    context
        .charge_work(bytes.len() as u64)
        .map_err(PdfError::from_context)?;
    Source::from_owned(bytes, document.engine_config().limits.max_output_bytes)
        .map_err(|error| PdfError::limit(error.to_string()))
}

fn same_written_object(
    before: &IndirectObject,
    after: &IndirectObject,
    document: &PdfDocument,
    reopened: &PdfDocument,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    if before.stream != after.stream {
        return Ok(false);
    }
    Ok(match (&before.value, &after.value, &before.stream) {
        (Value::Dict(left), Value::Dict(right), Some(stream)) => {
            let left_values = left.iter().filter(|(key, _)| key.as_slice() != b"Length");
            let right_values = right.iter().filter(|(key, _)| key.as_slice() != b"Length");
            if !left_values.eq(right_values) {
                return Ok(false);
            }
            match right.get(b"Length".as_slice()) {
                Some(Value::Integer(len)) => usize::try_from(*len).ok() == Some(stream.len()),
                Some(value @ Value::Ref(reference)) => {
                    // Untouched incremental streams retain their original
                    // indirect /Length. New serialized streams use direct
                    // lengths. Resolve the preserved reference in both graphs;
                    // do not authorize changing its identity or scalar value.
                    if left.get(b"Length".as_slice()) != Some(value) {
                        return Ok(false);
                    }
                    context.charge_work(2).map_err(PdfError::from_context)?;
                    let before_length = document.parsed().object(*reference)?;
                    let after_length = reopened.parsed().object(*reference)?;
                    before_length.stream.is_none()
                        && after_length.stream.is_none()
                        && before_length.value == after_length.value
                        && matches!(after_length.value, Value::Integer(len) if usize::try_from(len).ok() == Some(stream.len()))
                }
                _ => false,
            }
        }
        _ => before.value == after.value,
    })
}

pub(crate) fn same_trailer_values(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Dict(a), Value::Dict(b)) => a
            .iter()
            .filter(|(key, _)| !is_xref_layout_key(left, key))
            .eq(b.iter().filter(|(key, _)| !is_xref_layout_key(right, key))),
        _ => false,
    }
}

/// Shared classic compact writer, also used by the legacy canonical facade.
/// Returns final bytes and xref offset. It performs no independent mutation or
/// policy fallback; its caller evaluates the resulting bytes.
pub(crate) fn write_compact_document(
    document: &PdfDocument,
    context: Option<&OperationContext>,
) -> Result<(Vec<u8>, usize), PdfError> {
    refuse_security_boundaries_with_context(document.parsed(), context)?;
    refuse_stale_linearization_with_context(document.parsed(), context)?;
    if document.parsed().objects.len() > document.parsed().limits.max_objects {
        return Err(PdfError::limit("compact object count exceeds max_objects"));
    }
    let mut output =
        Output::new_with_context(document.engine_config().limits.max_output_bytes, context);
    output.push(b"%PDF-")?;
    output.push(document.parsed().version.as_bytes())?;
    output.push(b"\n%\xE2\xE3\xCF\xD3\n")?;
    let mut offsets = BTreeMap::new();
    for (reference, object) in &document.parsed().objects {
        if let Some(context) = context {
            context.charge_work(1).map_err(PdfError::from_context)?;
        }
        if offsets.contains_key(&reference.number) {
            return Err(PdfError::unsafe_rewrite(
                "canonical output cannot represent multiple live generations of one object",
            ));
        }
        if !document.parsed().objects.contains_base_key(reference) {
            let floor = document.parsed().source_size.ok_or_else(|| {
                PdfError::unsafe_rewrite("source trailer has no allocation floor")
            })?;
            if reference.generation != 0 || (reference.number as usize) < floor {
                return Err(PdfError::unsafe_rewrite(
                    "staged new objects require generation zero beyond source allocation floor",
                ));
            }
        }
        let offset = output.len();
        require_classic_offset(offset)?;
        offsets.insert(reference.number, (offset, reference.generation));
        output.formatted(format_args!(
            "{} {} obj\n",
            reference.number, reference.generation
        ))?;
        write_object(
            &mut output,
            object,
            document.parsed().limits.max_parser_depth,
        )?;
        output.push(b"\nendobj\n")?;
    }
    let max_object = offsets.keys().next_back().copied().unwrap_or(0);
    let graph_size = usize::try_from(max_object)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| PdfError::limit("canonical xref size overflows"))?;
    // Compact creates a fresh xref: discarded source free slots need not survive.
    // source_size remains the allocator floor for staged additions, not output layout.
    let size = graph_size.max(1);
    if size > document.engine_config().limits.max_xref_entries {
        return Err(PdfError::limit(
            "canonical xref size exceeds max_xref_entries",
        ));
    }
    let xref_offset = output.len();
    require_classic_offset(xref_offset)?;
    output.formatted(format_args!("xref\n0 {size}\n"))?;
    output.push(b"0000000000 65535 f \n")?;
    for number in 1..size {
        if let Some(context) = context {
            context.charge_work(1).map_err(PdfError::from_context)?;
        }
        match u32::try_from(number).ok().and_then(|n| offsets.get(&n)) {
            Some((offset, generation)) => {
                output.formatted(format_args!("{offset:010} {generation:05} n \n"))?
            }
            None => output.push(b"0000000000 00000 f \n")?,
        }
    }
    write_classic_trailer(
        &mut output,
        &document.parsed().trailer,
        size,
        None,
        document.parsed().limits.max_parser_depth,
    )?;
    output.formatted(format_args!("\nstartxref\n{xref_offset}\n%%EOF\n"))?;
    Ok((output.into_bytes(), xref_offset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PdfErrorCode, object::ObjectRef};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget, RevisionId};

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1_000_000,
                max_work_units: 1_000_000,
            }),
            CancellationToken::new(),
        )
    }

    fn document() -> PdfDocument {
        document_with_indirect_length(false)
    }

    fn document_with_indirect_length(indirect_length: bool) -> PdfDocument {
        let content = b"BT (alpha) Tj ET";
        let length = if indirect_length {
            "6 0 R".to_owned()
        } else {
            content.len().to_string()
        };
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R /Private 5 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
            [
                format!("<< /Length {length} >>\nstream\n").as_bytes(),
                content,
                b"\nendstream",
            ]
            .concat(),
            b"<< /PrivateString <FEFF67714EAC> /PrivateName /Example >>".to_vec(),
        ];
        if indirect_length {
            objects.push(content.len().to_string().into_bytes());
            // A second equal integer is not permission to retarget /Length.
            objects.push(content.len().to_string().into_bytes());
        }
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Custom 5 0 R /PrivateFlag true >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    #[test]
    fn preserved_indirect_stream_lengths_export_without_rewriting_untouched_streams() {
        let original = document_with_indirect_length(true);
        let stream_ref = ObjectRef {
            number: 4,
            generation: 0,
        };
        let length_ref = ObjectRef {
            number: 6,
            generation: 0,
        };
        let snapshot = SnapshotId::new(original.source_id(), RevisionId::INITIAL);
        for edit_other_object in [false, true] {
            let mut parsed = original.parsed().clone();
            if edit_other_object {
                let Value::Dict(values) = &mut parsed
                    .objects
                    .get_mut(&ObjectRef {
                        number: 5,
                        generation: 0,
                    })
                    .unwrap()
                    .value
                else {
                    unreachable!()
                };
                values.insert(b"Requested".to_vec(), Value::Bool(true));
            }
            let staged = original.with_parsed(parsed);
            for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
                let export = export_staged_document(&staged, snapshot, mode, &context()).unwrap();
                let reopened = PdfEngine::default()
                    .open_source(export.output.clone(), OpenOptions::default())
                    .unwrap();
                let before = staged.parsed().object(stream_ref).unwrap();
                let after = reopened.parsed().object(stream_ref).unwrap();
                assert!(
                    same_written_object(before, after, &staged, &reopened, &context()).unwrap()
                );
                assert_eq!(before.stream, after.stream);
                assert_eq!(
                    original.parsed().object(length_ref).unwrap().value,
                    reopened.parsed().object(length_ref).unwrap().value
                );
                let Value::Dict(values) = &after.value else {
                    unreachable!()
                };
                if mode == PdfWriterMode::Incremental {
                    assert!(export.output.as_bytes().starts_with(original.source()));
                    assert_eq!(
                        values.get(b"Length".as_slice()),
                        Some(&Value::Ref(length_ref))
                    );
                    if !edit_other_object {
                        assert_eq!(export.output.as_bytes(), original.source());
                    }
                } else {
                    assert_eq!(
                        values.get(b"Length".as_slice()),
                        Some(&Value::Integer(before.stream.as_ref().unwrap().len() as i64))
                    );
                }
                assert!(
                    export
                        .evidence
                        .checks()
                        .iter()
                        .all(|check| check.status == CheckStatus::Passed)
                );
            }
        }
    }

    #[test]
    fn indirect_lengths_require_the_same_reference_exact_integer_and_actual_payload_length() {
        let original = document_with_indirect_length(true);
        let stream_ref = ObjectRef {
            number: 4,
            generation: 0,
        };
        let length_ref = ObjectRef {
            number: 6,
            generation: 0,
        };
        let before = original.parsed().object(stream_ref).unwrap();
        let length = before.stream.as_ref().unwrap().len() as i64;
        for value in [
            Value::Integer(length - 1),
            Value::Integer(length + 1),
            Value::Integer(-1),
            Value::Real(length as f64),
            Value::Ref(length_ref),
            Value::Null,
        ] {
            let mut parsed = original.parsed().clone();
            parsed.objects.get_mut(&length_ref).unwrap().value = value;
            let wrong = original.with_parsed(parsed);
            assert!(
                !same_written_object(
                    before,
                    wrong.parsed().object(stream_ref).unwrap(),
                    &original,
                    &wrong,
                    &context()
                )
                .unwrap()
            );
            // Even mutually matching forged graph values cannot validate an
            // indirect declaration inconsistent with the actual payload.
            assert!(
                !same_written_object(
                    before,
                    wrong.parsed().object(stream_ref).unwrap(),
                    &wrong,
                    &wrong,
                    &context()
                )
                .unwrap()
            );
        }
        let mut parsed = original.parsed().clone();
        let Value::Dict(values) = &mut parsed.objects.get_mut(&stream_ref).unwrap().value else {
            unreachable!()
        };
        values.insert(
            b"Length".to_vec(),
            Value::Ref(ObjectRef {
                number: 7,
                generation: 0,
            }),
        );
        let retargeted = original.with_parsed(parsed);
        assert!(
            !same_written_object(
                before,
                retargeted.parsed().object(stream_ref).unwrap(),
                &original,
                &retargeted,
                &context()
            )
            .unwrap()
        );
        let mut parsed = original.parsed().clone();
        parsed.objects.remove(&length_ref);
        let missing = original.with_parsed(parsed);
        assert!(
            same_written_object(
                before,
                missing.parsed().object(stream_ref).unwrap(),
                &original,
                &missing,
                &context()
            )
            .is_err()
        );

        let exhausted = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 0,
                max_work_units: 1,
            }),
            CancellationToken::new(),
        );
        assert_eq!(
            same_written_object(before, before, &original, &original, &exhausted)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(exhausted.budget().snapshot().live_bytes, 0);
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert_eq!(
            same_written_object(before, before, &original, &original, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
    }

    #[test]
    fn reparsable_loss_of_indirect_length_object_values_still_fails_final_graph_evidence() {
        let original = document_with_indirect_length(true);
        let export = export_staged_document(
            &original,
            SnapshotId::new(original.source_id(), RevisionId::INITIAL),
            PdfWriterMode::Compact,
            &context(),
        )
        .unwrap();
        // Compact legitimately normalizes the stream's /Length to direct.
        // Its former indirect scalar remains an ordinary preserved object.
        let before = b"6 0 obj\n16\nendobj";
        let after = b"6 0 obj\n17\nendobj";
        let mut bytes = export.output.as_bytes().to_vec();
        let offset = bytes
            .windows(before.len())
            .position(|bytes| bytes == before)
            .unwrap();
        bytes[offset..offset + before.len()].copy_from_slice(after);
        let corrupted = Source::from_owned(bytes, 100_000).unwrap();
        let reopened = PdfEngine::default()
            .open_source(corrupted.clone(), OpenOptions::default())
            .unwrap();
        assert_eq!(reopened.query_text("alpha", 0).unwrap().text, "alpha");
        assert_eq!(
            verify_serialized_output(&original, &corrupted, PdfWriterMode::Compact, &context())
                .unwrap_err()
                .code,
            PdfErrorCode::VerificationFailed
        );
    }

    #[test]
    fn retained_writers_preserve_trailer_and_opaque_values_without_inventing_byte_provenance() {
        let original = document();
        let mut graph = original.parsed().clone();
        graph
            .objects
            .get_mut(&ObjectRef {
                number: 4,
                generation: 0,
            })
            .unwrap()
            .stream = Some(b"BT (omega) Tj ET".to_vec().into());
        let candidate = original.with_parsed(graph);
        assert!(
            candidate
                .query_text("omega", 0)
                .unwrap()
                .source_span
                .is_none()
        );
        assert!(
            original
                .query_text("alpha", 0)
                .unwrap()
                .source_span
                .is_some()
        );
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            let snapshot = SnapshotId::new(
                original.source_id(),
                RevisionId::INITIAL.checked_next().unwrap(),
            );
            let export = export_staged_document(&candidate, snapshot, mode, &context()).unwrap();
            assert_eq!(export.snapshot, snapshot);
            assert_eq!(export.mode, mode);
            let reopened = PdfEngine::default()
                .open_source(export.output.clone(), OpenOptions::default())
                .unwrap();
            assert!(same_trailer_values(
                &original.parsed().trailer,
                &reopened.parsed().trailer
            ));
            assert_eq!(reopened.query_text("omega", 0).unwrap().text, "omega");
            assert_eq!(
                reopened.parsed().objects.len(),
                original.parsed().objects.len()
            );
            assert!(
                export
                    .evidence
                    .checks()
                    .iter()
                    .all(|check| check.status == CheckStatus::Passed)
            );
            if mode == PdfWriterMode::Incremental {
                assert!(export.output.as_bytes().starts_with(original.source()));
            }
        }
        let canonical = original.canonicalize().unwrap();
        let reopened = PdfEngine::default()
            .open_owned(canonical.bytes, OpenOptions::default())
            .unwrap();
        assert!(same_trailer_values(
            &original.parsed().trailer,
            &reopened.parsed().trailer
        ));
    }

    #[test]
    fn staged_additions_accumulate_without_reopening_and_export_in_both_modes() {
        let original = document();
        let mut first = original.parsed().clone();
        let floor = first.source_size.unwrap() as u32;
        let first_ref = ObjectRef {
            number: floor,
            generation: 0,
        };
        let second_ref = ObjectRef {
            number: floor + 2,
            generation: 0,
        };
        let mut object = first
            .object(ObjectRef {
                number: 5,
                generation: 0,
            })
            .unwrap()
            .clone();
        object.offset = 0;
        object.value = Value::Dict(BTreeMap::from([(b"First".to_vec(), Value::Integer(1))]));
        first.objects.insert(first_ref, object.clone());
        let first = original.with_parsed(first);
        let mut later = first.parsed().clone();
        object.value = Value::Dict(BTreeMap::from([(b"Link".to_vec(), Value::Ref(first_ref))]));
        later.objects.insert(second_ref, object);
        if let Value::Dict(values) = &mut later.objects.get_mut(&first_ref).unwrap().value {
            values.insert(b"Later".to_vec(), Value::Integer(2));
        }
        // The current trailer is not an authority for source allocation.
        if let Value::Dict(values) = &mut later.trailer {
            values.insert(b"Size".to_vec(), Value::Integer(1));
        }
        let candidate = original.with_parsed(later);
        let snapshot = SnapshotId::new(original.source_id(), RevisionId::INITIAL);
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            let exported = export_staged_document(&candidate, snapshot, mode, &context()).unwrap();
            let reopened = PdfEngine::default()
                .open_source(exported.output.clone(), OpenOptions::default())
                .unwrap();
            assert_eq!(
                reopened.parsed().objects.len(),
                original.parsed().objects.len() + 2
            );
            assert_eq!(reopened.parsed().source_size, Some((floor + 3) as usize));
            for reference in [first_ref, second_ref] {
                assert!(
                    same_written_object(
                        candidate.parsed().object(reference).unwrap(),
                        reopened.parsed().object(reference).unwrap(),
                        &candidate,
                        &reopened,
                        &context(),
                    )
                    .unwrap()
                );
            }
            if mode == PdfWriterMode::Incremental {
                assert!(exported.output.as_bytes().starts_with(original.source()));
            }
        }
        assert!(!original.parsed().objects.contains_key(&first_ref));
        assert!(!first.parsed().objects.contains_key(&second_ref));
        for reference in [
            ObjectRef {
                number: floor,
                generation: 1,
            },
            ObjectRef {
                number: 5,
                generation: 1,
            },
        ] {
            let mut bad = candidate.parsed().clone();
            bad.objects.insert(
                reference,
                candidate.parsed().object(first_ref).unwrap().clone(),
            );
            let bad = original.with_parsed(bad);
            for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
                assert!(export_staged_document(&bad, snapshot, mode, &context()).is_err());
            }
        }
        let mut limited = candidate.parsed().clone();
        limited.limits.max_objects = original.parsed().objects.len();
        let limited = original.with_parsed(limited);
        assert_eq!(
            export_staged_document(&limited, snapshot, PdfWriterMode::Incremental, &context())
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
    }

    #[test]
    fn compact_size_tracks_current_graph_and_staged_xref_limits_are_enforced() {
        let original = document();
        let snapshot = SnapshotId::new(original.source_id(), RevisionId::INITIAL);
        let mut sparse = original.parsed().clone();
        sparse.source_size = Some(500_000);
        if let Value::Dict(trailer) = &mut sparse.trailer {
            trailer.insert(b"Size".to_vec(), Value::Integer(500_000));
        }
        let sparse = original.with_parsed(sparse);
        let compact =
            export_staged_document(&sparse, snapshot, PdfWriterMode::Compact, &context()).unwrap();
        let reopened = PdfEngine::default()
            .open_source(compact.output, OpenOptions::default())
            .unwrap();
        assert_eq!(reopened.parsed().source_size, original.parsed().source_size);
        let mut staged = original.parsed().clone();
        let added = ObjectRef {
            number: 100,
            generation: 0,
        };
        staged.objects.insert(
            added,
            staged
                .object(ObjectRef {
                    number: 5,
                    generation: 0,
                })
                .unwrap()
                .clone(),
        );
        staged.limits.max_xref_entries = 100;
        let staged = original.with_parsed(staged);
        let object = staged.parsed().object(added).unwrap();
        assert_eq!(
            super::super::append_object_revisions_with_context(
                &staged,
                &[(added, &object.value, None)],
                Some(&context())
            )
            .unwrap_err()
            .code,
            PdfErrorCode::ResourceLimit
        );
        // Generation-collision census participates in the operation quota and cancellation.
        let mut charged = original.parsed().clone();
        charged.objects.insert(added, object.clone());
        let charged = original.with_parsed(charged);
        let probe = context();
        refuse_security_boundaries_with_context(charged.parsed(), Some(&probe)).unwrap();
        refuse_stale_linearization_with_context(charged.parsed(), Some(&probe)).unwrap();
        let work_limit = probe.budget().snapshot().work_units + 2;
        let limited = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1_000_000,
                max_work_units: work_limit,
            }),
            CancellationToken::new(),
        );
        let value = &charged.parsed().object(added).unwrap().value;
        assert_eq!(
            append_object_revisions_with_context(&charged, &[(added, value, None)], Some(&limited))
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(limited.budget().snapshot().work_units, work_limit);
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert_eq!(
            export_staged_document(&charged, snapshot, PdfWriterMode::Incremental, &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        assert!(!original.parsed().objects.contains_key(&added));
    }

    #[test]
    fn reparsable_trailer_and_payload_corruption_fail_final_evidence() {
        let document = document();
        let mode = PdfWriterMode::Compact;
        let export = export_staged_document(
            &document,
            SnapshotId::new(document.source_id(), RevisionId::INITIAL),
            mode,
            &context(),
        )
        .unwrap();
        for (before, after) in [
            (b"/Custom 5 0 R".as_slice(), b"/Custom 4 0 R".as_slice()),
            (b"alpha", b"bravo"),
            (b"FEFF67714EAC", b"FEFF67714EAD"),
            (b"%PDF-1.7", b"%PDF-1.6"),
        ] {
            let mut bytes = export.output.as_bytes().to_vec();
            let offset = bytes
                .windows(before.len())
                .position(|value| value == before)
                .unwrap();
            bytes[offset..offset + before.len()].copy_from_slice(after);
            let tampered = Source::from_owned(bytes, 100_000).unwrap();
            PdfEngine::default()
                .open_source(tampered.clone(), OpenOptions::default())
                .unwrap();
            assert_eq!(
                verify_serialized_output(&document, &tampered, mode, &context())
                    .unwrap_err()
                    .code,
                PdfErrorCode::VerificationFailed
            );
        }
    }

    #[test]
    fn mode_limits_cancellation_and_dangling_references_are_enforced() {
        let document = document();
        let snapshot = SnapshotId::new(document.source_id(), RevisionId::INITIAL);
        assert_eq!(
            export_staged_document(&document, snapshot, PdfWriterMode::Surgical, &context())
                .unwrap_err()
                .code,
            PdfErrorCode::UnsupportedFeature
        );
        let stopped = context();
        stopped.cancellation().cancel();
        assert_eq!(
            export_staged_document(&document, snapshot, PdfWriterMode::Compact, &stopped)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        let mut graph = document.parsed().clone();
        graph.objects.remove(&ObjectRef {
            number: 5,
            generation: 0,
        });
        let candidate = document.with_parsed(graph);
        assert_eq!(
            export_staged_document(&candidate, snapshot, PdfWriterMode::Incremental, &context())
                .unwrap_err()
                .code,
            PdfErrorCode::UnsupportedFeature
        );
        assert_eq!(
            export_staged_document(&candidate, snapshot, PdfWriterMode::Compact, &context())
                .unwrap_err()
                .code,
            PdfErrorCode::VerificationFailed
        );
    }

    #[test]
    fn layout_sensitive_linearization_requires_a_supported_writer_plan() {
        let document = document();
        let mut graph = document.parsed().clone();
        let Value::Dict(dictionary) = &mut graph
            .objects
            .get_mut(&ObjectRef {
                number: 5,
                generation: 0,
            })
            .unwrap()
            .value
        else {
            unreachable!()
        };
        dictionary.insert(b"Linearized".to_vec(), Value::Real(1.0));
        dictionary.insert(b"L".to_vec(), Value::Integer(document.source_len() as i64));
        let candidate = document.with_parsed(graph);
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            assert_eq!(
                export_staged_document(
                    &candidate,
                    SnapshotId::new(document.source_id(), RevisionId::INITIAL),
                    mode,
                    &context()
                )
                .unwrap_err()
                .code,
                PdfErrorCode::UnsupportedFeature
            );
        }
    }
}
