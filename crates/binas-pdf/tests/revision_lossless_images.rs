//! Public image-storage workflows preserve the current edited revision.
//! Exact PDF dictionaries, encoded masks and RGB rendering use the independent oracle.
use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget,
    transaction::TransactionError,
};
use binas_pdf::{
    DecodeParams, EngineConfig, ImageColorSpace, Limits, OpenOptions, PageTransform, PdfDocument,
    PdfEngine, PdfErrorCode, PdfFilter, PdfLosslessImagePlan, PdfLosslessImageSkipReason,
    PdfPageTextEditRequest, PdfPageTransformRequest, PdfReferenceIndex, PdfReferenceTarget,
    PdfRevision, PdfRevisionRequest, PdfSharedEditMode, PdfWriterMode, StreamObjectRef,
    SurgicalTextEditRequest, list_annotations, list_form_fields, list_image_xobjects, list_streams,
    read_decoded_stream, read_document_info, read_embedded_attachment_bytes,
    read_embedded_attachments, read_named_destinations, read_outlines, read_page_labels,
    read_xmp_metadata,
};
use std::io::Write;

const SOURCE: &[u8] = include_bytes!("../../../qa/preservation/fixtures/lossless-images.pdf");

fn context() -> OperationContext {
    limited_context(128 * 1024 * 1024, 512 * 1024 * 1024)
}
fn limited_context(memory: u64, work: u64) -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: memory,
            max_work_units: work,
        }),
        CancellationToken::new(),
    )
}
fn open(bytes: &[u8]) -> PdfDocument {
    PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap()
}
fn reference(number: u32) -> PdfReferenceTarget {
    PdfReferenceTarget {
        number,
        generation: 0,
    }
}
fn edit(old: &str, new: &str) -> PdfRevisionRequest {
    PdfRevisionRequest {
        page_text: vec![PdfPageTextEditRequest {
            page_index: 0,
            shared: PdfSharedEditMode::Reject,
            edit: SurgicalTextEditRequest {
                old_text: old.into(),
                replacement: new.into(),
                match_index: 0,
            },
        }],
        ..Default::default()
    }
}
fn edited(document: PdfDocument) -> PdfRevision {
    let mut revision = PdfRevision::new(document);
    let ctx = context();
    let plan = revision.plan(edit("before", "after"), &ctx).unwrap();
    revision.commit(&plan, &ctx).unwrap();
    assert_eq!(revision.text().unwrap(), ["after", "untouched"]);
    revision
}
fn exported_document(revision: &PdfRevision) -> PdfDocument {
    let output = revision.export(PdfWriterMode::Compact, &context()).unwrap();
    open(output.output.as_bytes())
}

fn assert_plan(plan: &PdfLosslessImagePlan) {
    assert!(plan.census_complete());
    assert!(plan.census_gaps().is_empty());
    assert_eq!(plan.effects().len(), 2);
    for (effect, number, color_space, components, encoded) in [
        (&plan.effects()[0], 7, ImageColorSpace::DeviceRgb, 3, 18_448),
        (&plan.effects()[1], 8, ImageColorSpace::DeviceGray, 1, 583),
    ] {
        assert_eq!(effect.image, reference(number));
        assert_eq!((effect.width, effect.height), (96, 64));
        assert_eq!(effect.color_space, color_space);
        assert_eq!(effect.source_predictor, 1);
        assert_eq!(effect.output_predictor, 12);
        assert_eq!(effect.decoded_bytes, 96 * 64 * components);
        assert_eq!(effect.encoded_before, encoded);
        assert!(effect.encoded_after < effect.encoded_before);
    }
    assert_eq!(
        plan.encoded_bytes_saved(),
        plan.effects()
            .iter()
            .map(|effect| (effect.encoded_before - effect.encoded_after) as u64)
            .sum::<u64>()
    );
    for (number, reason) in [
        (9, PdfLosslessImageSkipReason::UnsupportedFilter),
        (13, PdfLosslessImageSkipReason::NotSmaller),
    ] {
        assert!(
            plan.skips()
                .iter()
                .any(|skip| skip.image == reference(number) && skip.reason == reason)
        );
    }
}

/// Compare to a separately exported, unoptimized committed baseline, including
/// all decoded streams and public document inventories. Predictors change only
/// for the two declared image identities; final encoded bytes use the oracle.
fn assert_outputs(revision: &PdfRevision, baseline: &PdfDocument) {
    let original_streams = list_streams(baseline).unwrap();
    let original_images = list_image_xobjects(baseline).unwrap();
    let original_refs = PdfReferenceIndex::build(baseline, &context()).unwrap();
    let original_pages = PdfRevision::new(baseline.clone()).page_ids().unwrap();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &context()).unwrap();
        assert_eq!(output.snapshot, revision.snapshot());
        assert_eq!(output.mode, mode);
        if mode == PdfWriterMode::Incremental {
            assert!(output.output.as_bytes().starts_with(SOURCE));
            // Positive payload savings do not promise a smaller incremental PDF.
            assert!(output.output.len() > SOURCE.len());
        }
        let reopened = open(output.output.as_bytes());
        assert_eq!(
            PdfRevision::new(reopened.clone()).page_ids().unwrap(),
            original_pages
        );
        for index in 0..original_pages.len() {
            assert_eq!(
                reopened.page_geometry(index).unwrap(),
                baseline.page_geometry(index).unwrap()
            );
            assert_eq!(
                reopened
                    .extract_page_text_spans(index)
                    .unwrap()
                    .spans
                    .iter()
                    .map(|span| &span.text)
                    .collect::<Vec<_>>(),
                baseline
                    .extract_page_text_spans(index)
                    .unwrap()
                    .spans
                    .iter()
                    .map(|span| &span.text)
                    .collect::<Vec<_>>()
            );
        }
        assert_eq!(
            list_form_fields(&reopened).unwrap(),
            list_form_fields(baseline).unwrap()
        );
        assert_eq!(
            list_annotations(&reopened).unwrap(),
            list_annotations(baseline).unwrap()
        );
        assert_eq!(
            read_document_info(&reopened).unwrap(),
            read_document_info(baseline).unwrap()
        );
        assert_eq!(
            read_xmp_metadata(&reopened).unwrap(),
            read_xmp_metadata(baseline).unwrap()
        );
        assert_eq!(
            read_outlines(&reopened).unwrap(),
            read_outlines(baseline).unwrap()
        );
        assert_eq!(
            read_named_destinations(&reopened).unwrap(),
            read_named_destinations(baseline).unwrap()
        );
        assert_eq!(
            read_page_labels(&reopened).unwrap(),
            read_page_labels(baseline).unwrap()
        );
        let attachments = read_embedded_attachments(&reopened).unwrap();
        let before_attachments = read_embedded_attachments(baseline).unwrap();
        assert_eq!(attachments, before_attachments);
        for (before, after) in before_attachments.iter().zip(&attachments) {
            assert_eq!(
                read_embedded_attachment_bytes(baseline, before).unwrap(),
                read_embedded_attachment_bytes(&reopened, after).unwrap()
            );
        }
        let streams = list_streams(&reopened).unwrap();
        assert_eq!(streams.len(), original_streams.len());
        for (before, after) in original_streams.iter().zip(&streams) {
            assert_eq!(after.object, before.object);
            assert_eq!(
                read_decoded_stream(&reopened, after.object).unwrap(),
                read_decoded_stream(baseline, before.object).unwrap()
            );
            if [7, 8].contains(&after.object.object_number) {
                assert!(after.encoded_length < before.encoded_length);
                assert_eq!(after.filter_chain.len(), 1);
                assert_eq!(after.filter_chain[0].filter, PdfFilter::FlateDecode);
                assert_eq!(
                    after.filter_chain[0].decode_params,
                    Some(DecodeParams {
                        predictor: 12,
                        colors: if after.object.object_number == 7 {
                            3
                        } else {
                            1
                        },
                        columns: 96,
                        bits_per_component: 8,
                        ..Default::default()
                    })
                );
            } else {
                assert_eq!(before, after);
            }
        }
        let images = list_image_xobjects(&reopened).unwrap();
        assert_eq!(images.len(), original_images.len());
        for (before, after) in original_images.iter().zip(&images) {
            assert_eq!(before.object, after.object);
            assert_eq!(
                (before.width, before.height, before.color_space),
                (after.width, after.height, after.color_space)
            );
        }
        let refs = PdfReferenceIndex::build(&reopened, &context()).unwrap();
        assert!(refs.records().iter().all(|record| !record.dangling));
        assert_eq!(refs.records(), original_refs.records());
    }
}

#[test]
fn exact_sample_candidates_preserve_the_required_edit_and_unrelated_streams() {
    let source = open(SOURCE);
    let mut revision = edited(source.clone());
    let baseline = exported_document(&revision);
    let before = revision.snapshot();
    let ctx = context();
    let plan = revision.plan_lossless_images(&ctx).unwrap();
    assert_plan(&plan);
    assert_eq!(plan.snapshot(), before);
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.text().unwrap(), ["after", "untouched"]);
    let receipt = revision
        .commit_lossless_images(&plan, &ctx)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.before, before);
    assert_eq!(receipt.after, revision.snapshot());
    assert_eq!(receipt.after.source(), before.source());
    assert_eq!(receipt.after.revision().value(), 2);
    assert_eq!(receipt.changes, plan.effects());
    receipt.evidence.validate(&receipt.required).unwrap();
    assert_outputs(&revision, &baseline);
    assert_eq!(
        PdfRevision::new(source).text().unwrap(),
        ["before", "untouched"]
    );
}

#[test]
fn original_encoding_wins_when_a_candidate_is_larger_and_repeat_is_a_noop() {
    let mut revision = edited(open(SOURCE));
    let baseline = exported_document(&revision);
    let ctx = context();
    let plan = revision.plan_lossless_images(&ctx).unwrap();
    assert_plan(&plan);
    revision
        .commit_lossless_images(&plan, &ctx)
        .unwrap()
        .unwrap();
    let before = revision.snapshot();
    let outputs = [PdfWriterMode::Incremental, PdfWriterMode::Compact]
        .map(|mode| revision.export(mode, &ctx).unwrap());
    let repeat = revision.plan_lossless_images(&ctx).unwrap();
    assert!(repeat.effects().is_empty());
    assert_eq!(repeat.encoded_bytes_saved(), 0);
    for number in [7, 8, 13] {
        assert!(
            repeat
                .skips()
                .iter()
                .any(|skip| skip.image == reference(number)
                    && skip.reason == PdfLosslessImageSkipReason::NotSmaller)
        );
    }
    assert!(
        revision
            .commit_lossless_images(&repeat, &ctx)
            .unwrap()
            .is_none()
    );
    assert!(
        revision
            .commit_lossless_images(&repeat, &ctx)
            .unwrap()
            .is_none()
    );
    assert_eq!(revision.snapshot(), before);
    for output in outputs {
        assert_eq!(
            revision
                .export(output.mode, &ctx)
                .unwrap()
                .output
                .as_bytes(),
            output.output.as_bytes()
        );
    }
    assert_outputs(&revision, &baseline);
}

#[test]
fn later_edits_reorder_and_geometry_keep_optimized_samples_and_current_intent() {
    let mut revision = edited(open(SOURCE));
    let mut unoptimized = revision.clone();
    let ctx = context();
    let plan = revision.plan_lossless_images(&ctx).unwrap();
    revision
        .commit_lossless_images(&plan, &ctx)
        .unwrap()
        .unwrap();
    for current in [&mut revision, &mut unoptimized] {
        let plan = current
            .plan(
                PdfRevisionRequest {
                    reorder_pages: Some(vec![1, 0]),
                    page_transforms: vec![PdfPageTransformRequest {
                        page_index: 0,
                        transform: PageTransform {
                            rotation_degrees: Some(90),
                            ..Default::default()
                        },
                    }],
                    ..edit("after", "final")
                },
                &ctx,
            )
            .unwrap();
        current.commit(&plan, &ctx).unwrap();
    }
    assert_eq!(revision.text().unwrap(), ["untouched", "final"]);
    assert_eq!(revision.page_geometry(1).unwrap().rotation_degrees, 90);
    let baseline = exported_document(&unoptimized);
    assert_outputs(&revision, &baseline);
    let repeated = revision.plan_lossless_images(&ctx).unwrap();
    assert!(repeated.effects().is_empty());
    let before = revision.snapshot();
    assert!(
        revision
            .commit_lossless_images(&repeated, &ctx)
            .unwrap()
            .is_none()
    );
    assert_eq!(revision.snapshot(), before);
}

#[test]
fn source_and_branch_bound_plans_cannot_revert_other_committed_work() {
    let mut left = edited(open(SOURCE));
    let ctx = context();
    let plan = left.plan_lossless_images(&ctx).unwrap();
    let mut changed = SOURCE.to_vec();
    let position = changed
        .windows(b"(before)".len())
        .position(|value| value == b"(before)")
        .unwrap();
    changed[position + 1] = b'B';
    let mut other_source = PdfRevision::new(open(&changed));
    let before = other_source.snapshot();
    assert!(matches!(
        other_source.commit_lossless_images(&plan, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(other_source.snapshot(), before);
    let mut right = left.clone();
    for (current, value) in [(&mut left, "left"), (&mut right, "right")] {
        let edit = current.plan(edit("after", value), &ctx).unwrap();
        current.commit(&edit, &ctx).unwrap();
    }
    assert_eq!(left.snapshot(), right.snapshot());
    let foreign = left.plan_lossless_images(&ctx).unwrap();
    let before = right.snapshot();
    assert!(
        matches!(right.commit_lossless_images(&foreign, &ctx), Err(TransactionError::Stage(error)) if error.code == PdfErrorCode::UnsafeRewrite)
    );
    assert_eq!(right.snapshot(), before);
    assert_eq!(right.text().unwrap(), ["right", "untouched"]);
    assert!(matches!(
        right.commit_lossless_images(&plan, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    let baseline = exported_document(&right);
    let own = right.plan_lossless_images(&ctx).unwrap();
    right.commit_lossless_images(&own, &ctx).unwrap().unwrap();
    let committed = right.snapshot();
    assert!(matches!(
        right.commit_lossless_images(&own, &ctx),
        Err(TransactionError::StalePlan { .. })
    ));
    assert_eq!(right.snapshot(), committed);
    assert_outputs(&right, &baseline);
}

#[test]
fn cancelled_or_limited_plans_and_commits_leave_the_edited_baseline_retryable() {
    let mut revision = edited(open(SOURCE));
    let before = revision.snapshot();
    let baseline = exported_document(&revision);
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision.plan_lossless_images(&cancelled).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    for limited in [
        limited_context(0, 512 * 1024 * 1024),
        limited_context(128 * 1024 * 1024, 0),
    ] {
        assert_eq!(
            revision.plan_lossless_images(&limited).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), before);
    }
    let exhausted = limited_context(128 * 1024 * 1024, 1);
    for _ in 0..2 {
        assert_eq!(
            revision.plan_lossless_images(&exhausted).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(exhausted.budget().snapshot().work_units, 1);
    }
    let ctx = context();
    let plan = revision.plan_lossless_images(&ctx).unwrap();
    assert!(matches!(
        revision.commit_lossless_images(&plan, &cancelled),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert!(
        revision
            .commit_lossless_images(&plan, &limited_context(128 * 1024 * 1024, 2))
            .is_err()
    );
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.text().unwrap(), ["after", "untouched"]);
    assert_eq!(
        list_streams(&exported_document(&revision)).unwrap(),
        list_streams(&baseline).unwrap()
    );
    revision
        .commit_lossless_images(&plan, &ctx)
        .unwrap()
        .unwrap();
    assert_outputs(&revision, &baseline);
}

#[test]
fn failed_exports_can_retry_without_losing_the_image_commit() {
    let mut revision = edited(open(SOURCE));
    let baseline = exported_document(&revision);
    let ctx = context();
    let plan = revision.plan_lossless_images(&ctx).unwrap();
    revision
        .commit_lossless_images(&plan, &ctx)
        .unwrap()
        .unwrap();
    let committed = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let cancelled = context();
        cancelled.cancellation().cancel();
        assert_eq!(
            revision.export(mode, &cancelled).unwrap_err().code,
            PdfErrorCode::Cancelled
        );
        assert_eq!(
            revision
                .export(mode, &limited_context(128 * 1024 * 1024, 0))
                .unwrap_err()
                .code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), committed);
        assert_eq!(revision.export(mode, &ctx).unwrap().snapshot, committed);
    }
    assert_outputs(&revision, &baseline);
}

#[test]
fn configured_output_failure_does_not_discard_edits_or_optimized_samples() {
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_output_bytes: 1,
            ..Default::default()
        },
    })
    .open(SOURCE, OpenOptions::default())
    .unwrap();
    let mut revision = edited(document);
    let ctx = context();
    let plan = revision.plan_lossless_images(&ctx).unwrap();
    revision
        .commit_lossless_images(&plan, &ctx)
        .unwrap()
        .unwrap();
    let before = revision.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        assert_eq!(
            revision.export(mode, &ctx).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
        assert_eq!(revision.snapshot(), before);
    }
    let edit = revision.plan(edit("after", "later"), &ctx).unwrap();
    revision.commit(&edit, &ctx).unwrap();
    assert_eq!(revision.text().unwrap(), ["later", "untouched"]);
    let again = revision.plan_lossless_images(&ctx).unwrap();
    assert!(again.effects().is_empty());
    assert!(
        revision
            .commit_lossless_images(&again, &ctx)
            .unwrap()
            .is_none()
    );
    assert_eq!(revision.snapshot().revision().value(), 3);
}

#[test]
fn decoded_image_limit_refuses_before_publishing_any_storage_change() {
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_total_decoded_bytes: 16 * 1024,
            ..Default::default()
        },
    })
    .open(SOURCE, OpenOptions::default())
    .unwrap();
    let revision = edited(document);
    let before = revision.snapshot();
    assert_eq!(
        revision.plan_lossless_images(&context()).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(revision.snapshot(), before);
    assert_eq!(revision.text().unwrap(), ["after", "untouched"]);
    // The failed image decode does not prevent bounded reads of unaffected data.
    let output = revision.export(PdfWriterMode::Compact, &context()).unwrap();
    let reopened = open(output.output.as_bytes());
    let stream = StreamObjectRef {
        object_number: 13,
        object_generation: 0,
    };
    assert_eq!(
        read_decoded_stream(&reopened, stream).unwrap(),
        read_decoded_stream(&open(SOURCE), stream).unwrap()
    );
}

// Small generated controls exercise the public planner's refusal/skip boundary.
// Object 5 is the selected case; object 6 is always a separate eligible image,
// so an unsupported image cannot disappear behind an entirely empty plan.
fn image_case(entries: &str, use_selected: bool) -> Vec<u8> {
    fn stream(entries: &str, bytes: &[u8], length: &str) -> Vec<u8> {
        [
            format!("<< {entries} /Length {length} >>\nstream\n").into_bytes(),
            bytes.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat()
    }
    fn encoded(value: u8) -> Vec<u8> {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        encoder.write_all(&[value; 32 * 16]).unwrap();
        encoder.finish().unwrap()
    }
    let selected = encoded(41);
    let eligible = encoded(73);
    let content = if use_selected {
        b"q 32 0 0 16 0 0 cm /Chosen Do Q q 32 0 0 16 40 0 cm /Eligible Do Q".as_slice()
    } else {
        b"q 32 0 0 16 40 0 cm /Eligible Do Q".as_slice()
    };
    let direct_length = selected.len().to_string();
    let (entries, length) = if let Some(entries) = entries.strip_suffix(" indirect-length") {
        (entries, "7 0 R")
    } else {
        (entries, direct_length.as_str())
    };
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Private [7 0 R (retain)] >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R /Resources << /XObject << /Chosen 5 0 R /Eligible 6 0 R >> >> >>".to_vec(),
        stream("", content, &content.len().to_string()),
        stream(entries, &selected, length),
        stream(
            "/Type /XObject /Subtype /Image /Width 32 /Height 16 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode",
            &eligible,
            &eligible.len().to_string(),
        ),
        selected.len().to_string().into_bytes(),
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
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

// This extractor is deliberately limited to the tiny generated controls above:
// uncompressed object layout, known writer delimiters and stream bytes that do
// not contain those delimiters. It does not serve as a general PDF parser.
fn case_stream(bytes: &[u8], number: u32) -> &[u8] {
    let header = format!("\n{number} 0 obj\n");
    let object = bytes
        .windows(header.len())
        .rposition(|part| part == header.as_bytes())
        .unwrap()
        + header.len();
    let start = bytes[object..]
        .windows(b"\nstream\n".len())
        .position(|part| part == b"\nstream\n")
        .unwrap()
        + object
        + b"\nstream\n".len();
    let end = bytes[start..]
        .windows(b"\nendstream".len())
        .position(|part| part == b"\nendstream")
        .unwrap()
        + start;
    &bytes[start..end]
}

#[test]
fn unsupported_image_cases_are_reported_and_retained_beside_an_eligible_image() {
    let base = "/Type /XObject /Subtype /Image /Width 32 /Height 16 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode";
    for (entries, reason, complete) in [
        (
            base.replace("/BitsPerComponent 8", "/BitsPerComponent 16"),
            PdfLosslessImageSkipReason::UnsupportedImageLayout,
            true,
        ),
        (
            base.replace("/Width 32", "/Width 32.0"),
            PdfLosslessImageSkipReason::UnsupportedImageLayout,
            true,
        ),
        (
            base.replace("/DeviceGray", "/DeviceCMYK"),
            PdfLosslessImageSkipReason::UnsupportedColorSpace,
            true,
        ),
        (
            base.replace("/Filter /FlateDecode", "/Filter [/FlateDecode]"),
            PdfLosslessImageSkipReason::UnsupportedFilter,
            true,
        ),
        (
            format!("{base} /DecodeParms << /Predictor 2 /Columns 32 >>"),
            PdfLosslessImageSkipReason::DecodeParameters,
            true,
        ),
        (
            format!("{base} indirect-length"),
            PdfLosslessImageSkipReason::DirectLengthRequired,
            true,
        ),
        (
            format!("{base} /ImageMask true"),
            PdfLosslessImageSkipReason::ImageMask,
            true,
        ),
        (
            format!("{base} /SMaskInData 0"),
            PdfLosslessImageSkipReason::EmbeddedAlpha,
            true,
        ),
        (
            format!("{base} /F (external-image.bin)"),
            PdfLosslessImageSkipReason::ExternalStream,
            false,
        ),
    ] {
        let bytes = image_case(&entries, true);
        let mut revision = PdfRevision::new(open(&bytes));
        let ctx = context();
        let before = revision.snapshot();
        let plan = revision.plan_lossless_images(&ctx).unwrap();
        assert_eq!(plan.census_complete(), complete, "{reason:?}");
        assert_eq!(plan.census_gaps().is_empty(), complete, "{reason:?}");
        assert_eq!(plan.effects().len(), 1, "{reason:?}");
        assert_eq!(plan.effects()[0].image, reference(6), "{reason:?}");
        assert_eq!(plan.skips().len(), 1, "{reason:?}");
        assert_eq!(plan.skips()[0].image, reference(5), "{reason:?}");
        assert_eq!(plan.skips()[0].reason, reason);
        let receipt = revision
            .commit_lossless_images(&plan, &ctx)
            .unwrap()
            .unwrap();
        assert_eq!(receipt.before, before);
        receipt.evidence.validate(&receipt.required).unwrap();
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            let output = revision
                .export(mode, &ctx)
                .unwrap_or_else(|error| panic!("{entries} / {reason:?} / {mode:?}: {error}"));
            assert_eq!(
                case_stream(output.output.as_bytes(), 5),
                case_stream(&bytes, 5),
                "{reason:?} {mode:?}"
            );
            let reopened = open(output.output.as_bytes());
            assert_eq!(
                read_decoded_stream(
                    &reopened,
                    StreamObjectRef {
                        object_number: 6,
                        object_generation: 0,
                    }
                )
                .unwrap(),
                vec![73; 32 * 16]
            );
        }
    }
}

#[test]
fn unused_image_remains_present_with_its_original_encoding() {
    let entries = "/Type /XObject /Subtype /Image /Width 32 /Height 16 /BitsPerComponent 8 /ColorSpace /DeviceGray /Filter /FlateDecode";
    let bytes = image_case(entries, false);
    let mut revision = PdfRevision::new(open(&bytes));
    let ctx = context();
    let plan = revision.plan_lossless_images(&ctx).unwrap();
    assert!(plan.census_complete());
    assert_eq!(plan.effects().len(), 1);
    assert_eq!(plan.effects()[0].image, reference(6));
    assert_eq!(plan.skips().len(), 1);
    assert_eq!(plan.skips()[0].image, reference(5));
    assert_eq!(plan.skips()[0].reason, PdfLosslessImageSkipReason::NotUsed);
    revision
        .commit_lossless_images(&plan, &ctx)
        .unwrap()
        .unwrap();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        assert_eq!(
            case_stream(output.output.as_bytes(), 5),
            case_stream(&bytes, 5)
        );
        assert_eq!(
            list_image_xobjects(&open(output.output.as_bytes()))
                .unwrap()
                .len(),
            2
        );
    }
}
