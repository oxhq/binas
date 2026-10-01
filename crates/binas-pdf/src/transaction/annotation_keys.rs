//! Contents-only annotation changes checked against the original dictionary.
//! Replacement objects are never used as preservation expectations.

use std::collections::BTreeMap;

use binas_core::OperationContext;

use super::page_keys::value_equal;
use crate::{
    PdfDocument, PdfError,
    object::{ObjectRef, Value},
};

pub(super) fn verify(
    original: &PdfDocument,
    candidate: &PdfDocument,
    expected: &BTreeMap<ObjectRef, Vec<u8>>,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if original.source_id() != candidate.source_id() {
        return Ok(false);
    }
    let limits = &original.parsed().limits;
    if expected.len() > limits.max_container_items {
        return Err(PdfError::limit(
            "annotation preservation count exceeds limit",
        ));
    }
    for (reference, contents) in expected {
        context.charge_work(1).map_err(PdfError::from_context)?;
        if contents.len() > limits.max_token_bytes {
            return Err(PdfError::limit("annotation Contents exceeds token limit"));
        }
        let before = original.parsed().object(*reference)?;
        let after = candidate.parsed().object(*reference)?;
        let (Value::Dict(before_keys), Value::Dict(after_keys)) = (&before.value, &after.value)
        else {
            return Ok(false);
        };
        // Match this migrated operation's existing planner boundary. /Type is
        // optional; every present unrelated key, including /Type, stays exact.
        if before.stream.is_some()
            || after.stream.is_some()
            || before_keys.contains_key(b"AP".as_slice())
            || !matches!(before_keys.get(b"Subtype".as_slice()), Some(Value::Name(name)) if name == b"Text")
            || !matches!(
                before_keys.get(b"Contents".as_slice()),
                None | Some(Value::Null | Value::String(_))
            )
        {
            return Ok(false);
        }
        context
            .charge_work(contents.len() as u64)
            .map_err(PdfError::from_context)?;
        if !matches!(after_keys.get(b"Contents".as_slice()), Some(Value::String(actual)) if actual == contents)
        {
            return Ok(false);
        }
        for (key, value) in before_keys {
            context
                .charge_work(key.len() as u64 + 1)
                .map_err(PdfError::from_context)?;
            if key == b"Contents" {
                continue;
            }
            let Some(actual) = after_keys.get(key) else {
                return Ok(false);
            };
            if !value_equal(value, actual, 0, limits.max_parser_depth, context)? {
                return Ok(false);
            }
        }
        for key in after_keys.keys() {
            context
                .charge_work(key.len() as u64 + 1)
                .map_err(PdfError::from_context)?;
            if key != b"Contents" && !before_keys.contains_key(key) {
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
    use crate::{
        AnnotationContentsMutationRequest, OpenOptions, PdfEngine, PdfErrorCode, PdfRevision,
        PdfRevisionRequest, annotations::encode_text,
    };
    use binas_core::{
        BudgetLimits, CancellationToken, ResourceBudget,
        transaction::{PlanEnvelope, TransactionError},
        verification::CheckStatus,
    };

    fn context(work: u64) -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                // Verification borrows every value; no managed allocation is needed.
                max_live_bytes: 0,
                max_work_units: work,
            }),
            CancellationToken::new(),
        )
    }

    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }

    fn fixture(contents: &str) -> PdfDocument {
        fixture_with_popup(contents, true)
    }

    fn fixture_with_popup(contents: &str, popup: bool) -> PdfDocument {
        // Original fixture with a popup relationship and opaque annotation data.
        // The verifier is exercised independently of the planner's narrower
        // reverse-reference eligibility rules for popup-bearing annotations.
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".into(),
            "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".into(),
            format!(
                "<< /Type /Page /Parent 2 0 R /Annots [{}] >>",
                if popup { "5 0 R 6 0 R" } else { "5 0 R" }
            ),
            "<< /Type /Page /Parent 2 0 R >>".into(),
            format!(
                "<< /Type /Annot /Subtype /Text /P 3 0 R /Rect [10 10 20 20] /F 4 {} /Private << /Keep [(original) 42] >> {contents} >>",
                if popup { "/Popup 6 0 R" } else { "" }
            ),
            if popup {
                "<< /Type /Annot /Subtype /Popup /Parent 5 0 R /P 3 0 R /Rect [30 30 80 80] >>"
            } else {
                "<< /Private (unreferenced) >>"
            }
            .into(),
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        PdfEngine::default()
            .open_owned(bytes, OpenOptions::default())
            .unwrap()
    }

    fn expected() -> BTreeMap<ObjectRef, Vec<u8>> {
        BTreeMap::from([(reference(5), encode_text("updated Ω 😀"))])
    }

    fn candidate(original: &PdfDocument) -> PdfDocument {
        mutate(original, |dictionary| {
            dictionary.insert(
                b"Contents".to_vec(),
                Value::String(expected()[&reference(5)].clone()),
            );
        })
    }

    fn mutate(
        document: &PdfDocument,
        update: impl FnOnce(&mut BTreeMap<Vec<u8>, Value>),
    ) -> PdfDocument {
        let mut parsed = document.parsed().clone();
        let Value::Dict(dictionary) = &mut parsed.objects.get_mut(&reference(5)).unwrap().value
        else {
            panic!("annotation dictionary");
        };
        update(dictionary);
        document.with_parsed(parsed)
    }

    #[test]
    fn contents_only_accepts_existing_missing_and_null_source_contents() {
        for contents in ["/Contents (old)", "", "/Contents null"] {
            let original = fixture(contents);
            let candidate = candidate(&original);
            let context = context(1_000_000);
            assert!(verify(&original, &candidate, &expected(), &context).unwrap());
            let usage = context.budget().snapshot();
            assert_eq!(usage.live_bytes, 0);
            assert!(usage.work_units > 0);
            for replacement in [
                None,
                Some(Value::String(b"wrong".to_vec())),
                Some(Value::Null),
            ] {
                let tampered = mutate(&candidate, |dictionary| {
                    dictionary.remove(b"Contents".as_slice());
                    if let Some(value) = replacement {
                        dictionary.insert(b"Contents".to_vec(), value);
                    }
                });
                assert!(!verify(&original, &tampered, &expected(), &context).unwrap());
            }
        }
    }

    #[test]
    fn requested_contents_cannot_hide_rect_parent_flags_popup_or_opaque_loss() {
        let original = fixture("/Contents (old)");
        let candidate = candidate(&original);
        let expected = expected();
        let context = context(1_000_000);
        for (key, replacement) in [
            (
                b"Rect".as_slice(),
                Some(Value::Array(
                    [30, 30, 40, 40].into_iter().map(Value::Integer).collect(),
                )),
            ),
            (b"P", Some(Value::Ref(reference(4)))),
            (b"F", None),
            (b"Popup", None),
            (b"Private", None),
            (
                b"C",
                Some(Value::Array(
                    [1, 0, 0].into_iter().map(Value::Integer).collect(),
                )),
            ),
        ] {
            let tampered = mutate(&candidate, |dictionary| {
                dictionary.remove(key);
                if let Some(value) = replacement {
                    dictionary.insert(key.to_vec(), value);
                }
                assert_eq!(
                    dictionary.get(b"Contents".as_slice()),
                    Some(&Value::String(expected[&reference(5)].clone()))
                );
            });
            assert!(
                !verify(&original, &tampered, &expected, &context).unwrap(),
                "{key:?}"
            );
        }
        let tampered = mutate(&candidate, |dictionary| {
            let Value::Dict(private) = dictionary.get_mut(b"Private".as_slice()).unwrap() else {
                panic!("private dictionary");
            };
            private.insert(
                b"Keep".to_vec(),
                Value::Array(vec![Value::String(b"changed".to_vec()), Value::Integer(42)]),
            );
        });
        assert!(!verify(&original, &tampered, &expected, &context).unwrap());
    }

    #[test]
    fn publication_rejects_self_consistent_rect_change_without_advancing_revision() {
        let original = fixture_with_popup("/Contents (old)", false);
        let mut revision = PdfRevision::new(original.clone());
        let snapshot = revision.snapshot();
        let annotations = revision.annotations().unwrap();
        let context = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1_000_000,
                max_work_units: 1_000_000,
            }),
            CancellationToken::new(),
        );
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    annotations: vec![AnnotationContentsMutationRequest {
                        annotation_index: 0,
                        contents: "requested Ω".into(),
                    }],
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        let mut objects = plan.envelope.intent().objects.clone();
        let Value::Dict(dictionary) = &mut objects.get_mut(&reference(5)).unwrap().value else {
            panic!("annotation dictionary");
        };
        dictionary.insert(
            b"Rect".to_vec(),
            Value::Array([30, 30, 40, 40].into_iter().map(Value::Integer).collect()),
        );
        let dishonest_intent = super::super::Intent {
            objects,
            streams: BTreeMap::new(),
            annotations: plan.envelope.intent().annotations.clone(),
            reorder: None,
            labels: None,
            text_pages: BTreeMap::new(),
            font_effects: Vec::new(),
            geometry: None,
            checkboxes: None,
            flatten: None,
            text_fields: None,
            duplicate: None,
            delete: None,
        };
        let mut parsed = original.parsed().clone();
        for (reference, object) in &dishonest_intent.objects {
            parsed.objects.insert(*reference, object.clone());
        }
        let evidence = super::super::verify_candidate(
            &original,
            &original.with_parsed(parsed),
            &dishonest_intent,
            &context,
        )
        .unwrap();
        assert!(
            evidence
                .checks()
                .iter()
                .any(|check| check.code.code() == "expected_changes"
                    && check.status == CheckStatus::Passed)
        );
        assert!(
            evidence
                .checks()
                .iter()
                .any(|check| check.code.code() == "annotation_dictionary_effects"
                    && check.status == CheckStatus::Failed)
        );
        let required = super::super::required(&dishonest_intent);
        assert!(evidence.validate(&required).is_err());
        let tampered_plan = super::super::PdfRevisionPlan {
            envelope: PlanEnvelope::new(snapshot, dishonest_intent, required),
            store: original.parsed().objects.clone(),
            _decoded_reservations: Vec::new(),
        };
        assert!(matches!(
            revision.commit(&tampered_plan, &context),
            Err(TransactionError::Evidence(_))
        ));
        assert_eq!(revision.snapshot(), snapshot);
        assert_eq!(revision.annotations().unwrap(), annotations);
    }

    #[test]
    fn source_boundary_and_candidate_stream_shape_remain_required() {
        let original = fixture("/Contents (old)");
        let expected = expected();
        let context = context(1_000_000);
        for (key, value) in [
            (b"Subtype".as_slice(), Value::Name(b"Link".to_vec())),
            (b"AP", Value::Null),
            (b"Contents", Value::Integer(5)),
        ] {
            let unsupported = mutate(&original, |dictionary| {
                dictionary.insert(key.to_vec(), value);
            });
            assert!(!verify(&unsupported, &candidate(&unsupported), &expected, &context).unwrap());
        }
        let mut parsed = original.parsed().clone();
        parsed.objects.get_mut(&reference(5)).unwrap().stream = Some(Vec::new().into());
        let streamed_source = original.with_parsed(parsed);
        assert!(
            !verify(
                &streamed_source,
                &candidate(&streamed_source),
                &expected,
                &context
            )
            .unwrap()
        );
        let candidate = candidate(&original);
        let mut parsed = candidate.parsed().clone();
        parsed.objects.get_mut(&reference(5)).unwrap().stream = Some(Vec::new().into());
        assert!(
            !verify(
                &original,
                &candidate.with_parsed(parsed),
                &expected,
                &context
            )
            .unwrap()
        );
    }

    #[test]
    fn annotation_verification_preserves_cancellation_and_cumulative_work_errors() {
        let original = fixture("/Contents (old)");
        let candidate = candidate(&original);
        let limited = context(1);
        assert_eq!(
            verify(&original, &candidate, &expected(), &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(
            verify(&original, &candidate, &expected(), &limited)
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        let cancelled = context(1_000_000);
        cancelled.cancellation().cancel();
        assert_eq!(
            verify(&original, &candidate, &expected(), &cancelled)
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
    }
}
