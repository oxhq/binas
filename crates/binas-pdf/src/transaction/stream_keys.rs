//! Text replacement changes encoded bytes and length hints, not stream semantics.

use std::collections::BTreeMap;

use binas_core::OperationContext;

use super::{StreamIntent, page_keys::value_equal};
use crate::{
    PdfDocument, PdfError,
    object::{ObjectRef, Value},
};

pub(super) fn verify(
    original: &PdfDocument,
    candidate: &PdfDocument,
    streams: &BTreeMap<ObjectRef, StreamIntent>,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if original.source_id() != candidate.source_id() {
        return Ok(false);
    }
    if streams.len() > original.parsed().limits.max_container_items {
        return Err(PdfError::limit("stream preservation count exceeds limit"));
    }
    let limit = original.parsed().limits.max_parser_depth;
    for (output, intent) in streams {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if output.number != intent.effect.output_stream.number
            || output.generation != intent.effect.output_stream.generation
        {
            return Ok(false);
        }
        let source = ObjectRef {
            number: intent.effect.source_stream.number,
            generation: intent.effect.source_stream.generation,
        };
        let before = original.parsed().object(source)?;
        let after = candidate.parsed().object(*output)?;
        if before.stream.is_none() {
            return Ok(false);
        }
        let (Value::Dict(before), Value::Dict(after), Some(encoded)) =
            (&before.value, &after.value, &after.stream)
        else {
            return Ok(false);
        };
        if !matches!(after.get(b"Length".as_slice()), Some(Value::Integer(length))
                if usize::try_from(*length).ok() == Some(encoded.len()))
        {
            return Ok(false);
        }
        for (key, expected) in before {
            context
                .charge_work(key.len() as u64 + 1)
                .map_err(PdfError::from_context)?;
            if key == b"Length" {
                continue;
            }
            let Some(actual) = after.get(key) else {
                return Ok(false);
            };
            if key == b"DL" && matches!(expected, Value::Integer(length) if *length >= 0) {
                if !matches!(actual, Value::Integer(length)
                    if usize::try_from(*length).ok() == Some(intent.expected.len()))
                {
                    return Ok(false);
                }
            } else if !value_equal(expected, actual, 0, limit, context)? {
                return Ok(false);
            }
        }
        for key in after.keys() {
            context
                .charge_work(key.len() as u64 + 1)
                .map_err(PdfError::from_context)?;
            if key != b"Length" && !before.contains_key(key) {
                return Ok(false);
            }
        }
    }
    context.checkpoint().map_err(PdfError::from_context)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OpenOptions, PdfEngine, PdfRevision, PdfRevisionRequest, SurgicalTextEditRequest};
    use binas_core::{BudgetLimits, CancellationToken, ResourceBudget};

    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1_000_000,
                max_work_units: 1_000_000,
            }),
            CancellationToken::new(),
        )
    }

    fn fixture(hint: &str) -> PdfDocument {
        let stream = "BT (before) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>".into(),
            format!(
                "<< /Length {} /Filter [] /DecodeParms null /Private << /Keep (opaque) >> {hint} >>\nstream\n{stream}\nendstream",
                stream.len()
            ),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Root 1 0 R /Size 5 >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    #[test]
    fn source_stream_keys_reject_self_consistent_opaque_and_filter_loss() {
        let source = fixture("/DL 16");
        let revision = PdfRevision::new(source.clone());
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    text: vec![SurgicalTextEditRequest {
                        old_text: "before".into(),
                        replacement: "after".into(),
                        match_index: 0,
                    }],
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        let intent = plan.envelope.intent();
        let mut clean = source.parsed().clone();
        for (reference, object) in &intent.objects {
            clean.objects.insert(*reference, object.clone());
        }
        assert!(
            verify(
                &source,
                &source.with_parsed(clean.clone()),
                &intent.streams,
                &context
            )
            .unwrap()
        );
        let reference = ObjectRef {
            number: 4,
            generation: 0,
        };
        for key in [b"Private".as_slice(), b"Filter", b"DecodeParms"] {
            let mut corrupted = clean.clone();
            let Value::Dict(dictionary) = &mut corrupted.objects.get_mut(&reference).unwrap().value
            else {
                panic!();
            };
            assert!(dictionary.remove(key).is_some());
            assert!(
                !verify(
                    &source,
                    &source.with_parsed(corrupted),
                    &intent.streams,
                    &context
                )
                .unwrap()
            );
        }
        for key in [b"Length".as_slice(), b"DL"] {
            let mut corrupted = clean.clone();
            let Value::Dict(dictionary) = &mut corrupted.objects.get_mut(&reference).unwrap().value
            else {
                panic!();
            };
            dictionary.insert(key.to_vec(), Value::Integer(1));
            assert!(
                !verify(
                    &source,
                    &source.with_parsed(corrupted),
                    &intent.streams,
                    &context
                )
                .unwrap()
            );
        }
    }

    #[test]
    fn text_edits_do_not_invent_or_replace_absent_and_null_decoded_length_hints() {
        for hint in ["", "/DL null"] {
            let source = fixture(hint);
            let revision = PdfRevision::new(source.clone());
            let context = context();
            let plan = revision
                .plan(
                    PdfRevisionRequest {
                        text: vec![SurgicalTextEditRequest {
                            old_text: "before".into(),
                            replacement: "after".into(),
                            match_index: 0,
                        }],
                        ..Default::default()
                    },
                    &context,
                )
                .unwrap();
            let intent = plan.envelope.intent();
            let mut clean = source.parsed().clone();
            for (reference, object) in &intent.objects {
                clean.objects.insert(*reference, object.clone());
            }
            assert!(
                verify(
                    &source,
                    &source.with_parsed(clean.clone()),
                    &intent.streams,
                    &context
                )
                .unwrap()
            );
            let Value::Dict(dictionary) = &mut clean
                .objects
                .get_mut(&ObjectRef {
                    number: 4,
                    generation: 0,
                })
                .unwrap()
                .value
            else {
                panic!();
            };
            dictionary.insert(b"DL".to_vec(), Value::Integer(15));
            assert!(
                !verify(
                    &source,
                    &source.with_parsed(clean),
                    &intent.streams,
                    &context
                )
                .unwrap()
            );
        }
    }
}
