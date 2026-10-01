//! Catalog changes need a declared operation and its separate source/effect
//! checks. A staged catalog is never permission to drop unrelated entries.

use binas_core::OperationContext;

use super::{Intent, page_keys::value_equal};
use crate::{PdfDocument, PdfError, object::Value};

pub(super) fn verify(
    original: &PdfDocument,
    candidate: &PdfDocument,
    intent: &Intent,
    context: &OperationContext,
) -> Result<bool, PdfError> {
    context.checkpoint().map_err(PdfError::from_context)?;
    if original.source_id() != candidate.source_id() {
        return Ok(false);
    }
    let (Value::Dict(before_trailer), Value::Dict(after_trailer)) =
        (&original.parsed().trailer, &candidate.parsed().trailer)
    else {
        return Ok(false);
    };
    let Some(Value::Ref(root)) = before_trailer.get(b"Root".as_slice()) else {
        return Ok(false);
    };
    if after_trailer.get(b"Root".as_slice()) != Some(&Value::Ref(*root)) {
        return Ok(false);
    }
    let before = original.parsed().object(*root)?;
    let after = candidate.parsed().object(*root)?;
    if before.stream.is_some() || after.stream.is_some() {
        return Ok(false);
    }
    let (Value::Dict(before), Value::Dict(after)) = (&before.value, &after.value) else {
        return Ok(false);
    };
    let permitted = |key: &[u8]| {
        (key == b"PageLabels" && intent.labels.is_some())
            || intent
                .delete
                .as_ref()
                .is_some_and(|plan| plan.allows_catalog_key(key))
            || (key == b"AcroForm"
                && (intent
                    .flatten
                    .as_ref()
                    .is_some_and(|plan| plan.changes_catalog_acroform())
                    || intent
                        .duplicate
                        .as_ref()
                        .is_some_and(|plan| plan.changes_catalog_acroform())))
    };
    for (key, value) in before {
        context
            .charge_work(key.len() as u64 + 1)
            .map_err(PdfError::from_context)?;
        if permitted(key) {
            continue;
        }
        let Some(actual) = after.get(key) else {
            return Ok(false);
        };
        if !value_equal(
            value,
            actual,
            0,
            original.parsed().limits.max_parser_depth,
            context,
        )? {
            return Ok(false);
        }
    }
    for key in after.keys() {
        context
            .charge_work(key.len() as u64 + 1)
            .map_err(PdfError::from_context)?;
        if !before.contains_key(key) && !permitted(key) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        OpenOptions, PdfEngine, PdfFieldFlattenRequest, PdfReferenceIndex, PdfRevision,
        forms::flatten, object::ObjectRef,
    };
    use binas_core::{
        BudgetLimits, CancellationToken, ResourceBudget,
        transaction::{PlanEnvelope, TransactionError},
        verification::CheckStatus,
    };
    use std::collections::BTreeMap;

    fn reference(number: u32) -> ObjectRef {
        ObjectRef {
            number,
            generation: 0,
        }
    }
    fn context() -> OperationContext {
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 64 * 1024 * 1024,
                max_work_units: 256 * 1024 * 1024,
            }),
            CancellationToken::new(),
        )
    }

    #[test]
    fn deletion_composes_direct_registration_destinations_and_labels_without_other_catalog_loss() {
        use crate::{PdfExcludedPageTargetPolicy, PdfPageDeleteRequest, PdfRevisionRequest};
        let source = PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/text-field-fill.pdf"),
                OpenOptions::default(),
            )
            .unwrap();
        let mut parsed = source.parsed().clone();
        let form = parsed.object(reference(23)).unwrap().value.clone();
        let labels = parsed.object(reference(31)).unwrap().value.clone();
        let Value::Dict(catalog) = &mut parsed.objects.get_mut(&reference(1)).unwrap().value else {
            panic!()
        };
        catalog.insert(b"AcroForm".to_vec(), form);
        catalog.insert(b"PageLabels".to_vec(), labels);
        catalog.insert(
            b"OpenAction".to_vec(),
            Value::Array(vec![Value::Ref(reference(4)), Value::Name(b"Fit".to_vec())]),
        );
        catalog.insert(
            b"DeleteKeep".to_vec(),
            Value::String(b"source catalog value".to_vec()),
        );
        let source = source.with_parsed(parsed);
        let request = || PdfRevisionRequest {
            delete_page: Some(PdfPageDeleteRequest {
                page_index: 1,
                excluded_targets: PdfExcludedPageTargetPolicy::RemapToPage { page_index: 0 },
            }),
            ..Default::default()
        };
        let mut revision = PdfRevision::new(source.clone());
        let planned = revision.plan(request(), &context()).unwrap();
        let receipt = revision.commit(&planned, &context()).unwrap();
        assert_eq!(receipt.after.revision().value(), 1);
        let actual = revision.state.value();
        assert_eq!(actual.page_refs().unwrap(), [reference(3)]);
        for number in [23, 31] {
            assert_eq!(
                actual.parsed().object(reference(number)).unwrap().value,
                source.parsed().object(reference(number)).unwrap().value
            );
        }
        let Value::Dict(catalog) = &actual.parsed().object(reference(1)).unwrap().value else {
            panic!()
        };
        assert_eq!(
            catalog.get(b"DeleteKeep".as_slice()),
            Some(&Value::String(b"source catalog value".to_vec()))
        );
        assert_eq!(
            catalog.get(b"OpenAction".as_slice()),
            Some(&Value::Array(vec![
                Value::Ref(reference(3)),
                Value::Name(b"Fit".to_vec())
            ]))
        );
        assert_eq!(crate::read_page_labels(actual).unwrap()[0].label, "A-1");

        for corruption in 0..3 {
            let mut revision = PdfRevision::new(source.clone());
            let snapshot = revision.snapshot();
            let ctx = context();
            let index = PdfReferenceIndex::build(&source, &ctx).unwrap();
            let mut intent = Intent {
                objects: BTreeMap::new(),
                streams: BTreeMap::new(),
                annotations: BTreeMap::new(),
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
            let deletion = crate::preservation::delete::plan(
                &source,
                snapshot,
                &index,
                &request().delete_page.unwrap(),
                &intent.objects,
                &ctx,
            )
            .unwrap();
            crate::preservation::delete::apply(&source, &deletion, &mut intent.objects, &ctx)
                .unwrap();
            let mut labels = crate::preservation::labels::plan_deleted_labels(
                &source,
                deletion.source_indices(),
                &ctx,
            )
            .unwrap();
            let catalog_keys: Vec<_> = deletion.catalog_keys().collect();
            for (reference, object) in std::mem::take(&mut labels.changes) {
                super::super::merge_catalog_labels(
                    &source,
                    &mut intent.objects,
                    reference,
                    object,
                    &catalog_keys,
                    &ctx,
                )
                .unwrap();
            }
            intent.labels = Some(labels);
            intent.delete = Some(deletion);
            let Value::Dict(catalog) = &mut intent.objects.get_mut(&reference(1)).unwrap().value
            else {
                panic!()
            };
            match corruption {
                0 => {
                    catalog.remove(b"DeleteKeep".as_slice());
                }
                1 => {
                    catalog.insert(
                        b"OpenAction".to_vec(),
                        Value::Array(vec![Value::Ref(reference(4)), Value::Name(b"Fit".to_vec())]),
                    );
                }
                _ => {
                    let Value::Dict(labels) = catalog.get_mut(b"PageLabels".as_slice()).unwrap()
                    else {
                        panic!()
                    };
                    let Value::Array(nums) = labels.get_mut(b"Nums".as_slice()).unwrap() else {
                        panic!()
                    };
                    let Value::Dict(spec) = &mut nums[1] else {
                        panic!()
                    };
                    spec.insert(b"Unrequested".to_vec(), Value::Bool(true));
                }
            }
            let checks = super::super::required(&intent);
            let forged = super::super::PdfRevisionPlan {
                envelope: PlanEnvelope::new(snapshot, intent, checks),
                store: source.parsed().objects.clone(),
                _decoded_reservations: Vec::new(),
            };
            assert!(
                revision.commit(&forged, &context()).is_err(),
                "corruption {corruption}"
            );
            assert_eq!(revision.snapshot(), snapshot);
            assert_eq!(
                revision.state.value().page_refs().unwrap(),
                [reference(3), reference(4)]
            );
        }
    }

    #[test]
    fn direct_acroform_flatten_does_not_authorize_unrelated_catalog_loss() {
        let source = PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/targeted-flatten.pdf"),
                OpenOptions::default(),
            )
            .unwrap();
        let mut parsed = source.parsed().clone();
        let acroform = parsed.object(reference(21)).unwrap().value.clone();
        let Value::Dict(catalog) = &mut parsed.objects.get_mut(&reference(1)).unwrap().value else {
            panic!()
        };
        catalog.insert(b"AcroForm".to_vec(), acroform);
        let source = source.with_parsed(parsed);
        let mut revision = PdfRevision::new(source.clone());
        let ctx = context();
        let index = PdfReferenceIndex::build(&source, &ctx).unwrap();
        let requests = ["group.answer", "group.keep"].map(|name| PdfFieldFlattenRequest {
            field_name: name.into(),
            match_index: 0,
        });
        let mut intent = Intent {
            objects: BTreeMap::new(),
            streams: BTreeMap::new(),
            annotations: BTreeMap::new(),
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
        let flatten = flatten::plan(
            &source,
            revision.snapshot(),
            &index,
            &requests,
            &intent.objects,
            &ctx,
        )
        .unwrap();
        assert!(flatten.changes_catalog_acroform());
        flatten::apply(&source, &flatten, &mut intent.objects, &ctx).unwrap();
        intent.flatten = Some(flatten);
        let Value::Dict(catalog) = &mut intent.objects.get_mut(&reference(1)).unwrap().value else {
            panic!()
        };
        assert!(catalog.remove(b"Private".as_slice()).is_some());
        let mut parsed = source.parsed().clone();
        for (reference, object) in &intent.objects {
            parsed.objects.insert(*reference, object.clone());
        }
        let candidate = source.with_parsed(parsed);
        let report = super::super::verify_candidate(&source, &candidate, &intent, &ctx).unwrap();
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code.code() == "expected_changes"
                    && check.status == CheckStatus::Passed)
        );
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code.code() == "catalog_dictionary_effects"
                    && check.status == CheckStatus::Failed)
        );
        let before = revision.snapshot();
        let required = super::super::required(&intent);
        let plan = super::super::PdfRevisionPlan {
            envelope: PlanEnvelope::new(before, intent, required),
            store: source.parsed().objects.clone(),
            _decoded_reservations: Vec::new(),
        };
        assert!(matches!(
            revision.commit(&plan, &ctx),
            Err(TransactionError::Evidence(_))
        ));
        assert_eq!(revision.snapshot(), before);
    }

    #[test]
    fn duplicate_direct_form_and_labels_merge_without_authorizing_catalog_loss() {
        use crate::{
            PdfDuplicateFieldName, PdfPageDuplicateRequest, PdfRevisionRequest,
            preservation::{duplicate, labels},
        };
        let source = PdfEngine::default()
            .open(
                include_bytes!("../../../../qa/preservation/fixtures/text-field-fill.pdf"),
                OpenOptions::default(),
            )
            .unwrap();
        let mut parsed = source.parsed().clone();
        let mut acro = parsed.object(reference(23)).unwrap().value.clone();
        let Value::Dict(acro_values) = &mut acro else {
            panic!()
        };
        // Use independent root fields so duplication updates direct /Fields in
        // the catalog. Old unreachable registration dictionaries remain exact.
        acro_values.insert(
            b"Fields".to_vec(),
            Value::Array(
                [8, 11, 12, 13]
                    .map(|number| Value::Ref(reference(number)))
                    .into(),
            ),
        );
        for number in [8, 11] {
            let Value::Dict(field) = &mut parsed.objects.get_mut(&reference(number)).unwrap().value
            else {
                panic!()
            };
            field.remove(b"Parent".as_slice());
        }
        let page_labels = parsed.object(reference(31)).unwrap().value.clone();
        let Value::Dict(catalog) = &mut parsed.objects.get_mut(&reference(1)).unwrap().value else {
            panic!()
        };
        catalog.insert(b"AcroForm".to_vec(), acro);
        catalog.insert(b"PageLabels".to_vec(), page_labels);
        let source = source.with_parsed(parsed);
        let request = PdfPageDuplicateRequest {
            page_index: 0,
            fields: ["name", "keep"]
                .map(|name| PdfDuplicateFieldName {
                    field_name: name.into(),
                    new_field_name: format!("{name}_copy"),
                })
                .into(),
        };
        let mut revision = PdfRevision::new(source.clone());
        let ctx = context();
        let index = PdfReferenceIndex::build(&source, &ctx).unwrap();
        let mut intent = Intent {
            objects: BTreeMap::new(),
            streams: BTreeMap::new(),
            annotations: BTreeMap::new(),
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
        let duplicate = duplicate::plan(
            &source,
            revision.snapshot(),
            &index,
            &request,
            &intent.objects,
            &ctx,
        )
        .unwrap();
        assert!(duplicate.changes_catalog_acroform());
        duplicate::apply(&source, &duplicate, &mut intent.objects, &ctx).unwrap();
        let mut labels =
            labels::plan_occurrence_labels(&source, duplicate.source_indices(), &ctx).unwrap();
        for (reference, object) in std::mem::take(&mut labels.changes) {
            super::super::merge_catalog_labels(
                &source,
                &mut intent.objects,
                reference,
                object,
                &[b"AcroForm"],
                &ctx,
            )
            .unwrap();
        }
        intent.labels = Some(labels);
        intent.duplicate = Some(duplicate);
        let candidate = |intent: &Intent| {
            let mut parsed = source.parsed().clone();
            for (reference, object) in &intent.objects {
                parsed.objects.insert(*reference, object.clone());
            }
            source.with_parsed(parsed)
        };
        let valid =
            super::super::verify_candidate(&source, &candidate(&intent), &intent, &ctx).unwrap();
        valid.validate(&super::super::required(&intent)).unwrap();
        let Value::Dict(catalog) = &mut intent.objects.get_mut(&reference(1)).unwrap().value else {
            panic!()
        };
        assert!(catalog.remove(b"Private".as_slice()).is_some());
        let report =
            super::super::verify_candidate(&source, &candidate(&intent), &intent, &ctx).unwrap();
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code.code() == "expected_changes"
                    && check.status == CheckStatus::Passed)
        );
        assert!(
            report
                .checks()
                .iter()
                .any(|check| check.code.code() == "catalog_dictionary_effects"
                    && check.status == CheckStatus::Failed)
        );
        let before = revision.snapshot();
        let required = super::super::required(&intent);
        let plan = super::super::PdfRevisionPlan {
            envelope: PlanEnvelope::new(before, intent, required),
            store: source.parsed().objects.clone(),
            _decoded_reservations: Vec::new(),
        };
        assert!(matches!(
            revision.commit(&plan, &ctx),
            Err(TransactionError::Evidence(_))
        ));
        assert_eq!(revision.snapshot(), before);
        let valid = revision
            .plan(
                PdfRevisionRequest {
                    duplicate_page: Some(request),
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        revision.commit(&valid, &ctx).unwrap();
        assert_eq!(revision.page_ids().unwrap().len(), 3);
        assert_eq!(revision.snapshot().revision().value(), 1);
    }
}
