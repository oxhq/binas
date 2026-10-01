use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    AnnotationContentsMutationRequest, OpenOptions, PdfDocument, PdfEngine, PdfRevision,
    PdfRevisionRequest, PdfWriterMode, SurgicalTextEditRequest,
};
use std::io::Write;

fn context(work: u64) -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 10_000_000,
            max_work_units: work,
        }),
        CancellationToken::new(),
    )
}
fn document(
    filtered: bool,
    shared_stream: bool,
    shared_annotation: bool,
    text: &str,
) -> PdfDocument {
    let content = format!("BT ({text}) Tj (bravo) Tj ET");
    let encoded = if filtered {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(content.as_bytes()).unwrap();
        encoder.finish().unwrap()
    } else {
        content.into_bytes()
    };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Lang (en) >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 7 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [5 0 R] /MediaBox [0 0 200 200] >>"
            .to_vec(),
        [
            format!(
                "<< /Length {} {} >>\nstream\n",
                encoded.len(),
                if filtered { "/Filter /FlateDecode" } else { "" }
            )
            .into_bytes(),
            encoded,
            b"\nendstream".to_vec(),
        ]
        .concat(),
        b"<< /Type /Annot /Subtype /Text /Rect [10 10 20 20] /Contents (old note) /P 3 0 R >>"
            .to_vec(),
        b"<< /Title (Original) >>".to_vec(),
    ];
    objects.push(
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] {} {} >>",
            if shared_stream { "/Contents 4 0 R" } else { "" },
            if shared_annotation {
                "/Annots [5 0 R]"
            } else {
                ""
            }
        )
        .into_bytes(),
    );
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
            "trailer\n<< /Size {} /Root 1 0 R /Info 6 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    PdfEngine::default()
        .open_owned(bytes, OpenOptions::default())
        .unwrap()
}
fn text(old: &str, replacement: &str) -> SurgicalTextEditRequest {
    SurgicalTextEditRequest {
        old_text: old.into(),
        replacement: replacement.into(),
        match_index: 0,
    }
}
fn request() -> PdfRevisionRequest {
    PdfRevisionRequest {
        text: vec![text("alpha", "longer text"), text("bravo", "B")],
        annotations: vec![AnnotationContentsMutationRequest {
            annotation_index: 0,
            contents: "café 中文 😀".into(),
        }],
        ..Default::default()
    }
}

#[test]
fn mixed_same_stream_batch_commits_once_then_exports_explicitly() {
    for filtered in [false, true] {
        let document = document(filtered, false, false, "alpha");
        let original = document.input_source().clone();
        let mut revision = PdfRevision::new(document);
        let context = context(1_000_000);
        let plan = revision.plan(request(), &context).unwrap();
        assert_eq!(plan.changed_objects().count(), 2);
        assert_eq!(revision.text().unwrap(), ["alpha", "bravo"]);
        let receipt = revision.commit(&plan, &context).unwrap();
        assert_eq!(receipt.after.revision().value(), 1);
        assert_eq!(revision.snapshot().source(), original.id());
        assert_eq!(revision.text().unwrap(), ["longer text", "B"]);
        assert_eq!(
            revision.annotations().unwrap()[0].contents.as_deref(),
            Some("café 中文 😀")
        );
        let export = revision
            .export(PdfWriterMode::Incremental, &context)
            .unwrap();
        assert!(export.output.as_bytes().starts_with(original.as_bytes()));
        let reopened = PdfEngine::default()
            .open_source(export.output, OpenOptions::default())
            .unwrap();
        assert_eq!(reopened.inspect().unwrap().xref_revisions, 2);
        assert_eq!(
            reopened
                .extract_text_spans()
                .unwrap()
                .spans
                .into_iter()
                .map(|span| span.text)
                .collect::<Vec<_>>(),
            ["longer text", "B"]
        );
        assert_eq!(
            binas_pdf::read_document_info(&reopened)
                .unwrap()
                .title
                .as_deref(),
            Some("Original")
        );
        assert!(revision.commit(&plan, &context).is_err());
    }
}

#[test]
fn conflicting_and_late_invalid_requests_do_not_publish_changes() {
    let revision = PdfRevision::new(document(false, false, false, "alpha"));
    let context = context(1_000_000);
    let duplicate = PdfRevisionRequest {
        text: vec![text("alpha", "one"), text("alpha", "two")],
        annotations: vec![],
        ..Default::default()
    };
    assert!(revision.plan(duplicate, &context).is_err());
    let annotation = AnnotationContentsMutationRequest {
        annotation_index: 0,
        contents: "one".into(),
    };
    assert!(
        revision
            .plan(
                PdfRevisionRequest {
                    text: vec![],
                    annotations: vec![annotation.clone(), annotation],
                    ..Default::default()
                },
                &context
            )
            .is_err()
    );
    let mut invalid = request();
    invalid.annotations[0].annotation_index = 999;
    assert!(revision.plan(invalid, &context).is_err());
    assert_eq!(revision.text().unwrap(), ["alpha", "bravo"]);
    assert_eq!(revision.snapshot().revision().value(), 0);
}

#[test]
fn plans_reject_other_sources_independent_opens_and_divergent_branches() {
    let mut first = PdfRevision::new(document(false, false, false, "alpha"));
    let mut branch = first.clone();
    let context = context(1_000_000);
    let first_plan = first
        .plan(
            PdfRevisionRequest {
                text: vec![text("alpha", "first")],
                annotations: vec![],
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    let branch_plan = branch
        .plan(
            PdfRevisionRequest {
                text: vec![text("alpha", "other")],
                annotations: vec![],
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    first.commit(&first_plan, &context).unwrap();
    branch.commit(&branch_plan, &context).unwrap();
    assert_eq!(first.snapshot(), branch.snapshot());
    let next = first
        .plan(
            PdfRevisionRequest {
                text: vec![text("first", "third")],
                annotations: vec![],
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    assert!(branch.commit(&next, &context).is_err());
    let mut independent = PdfRevision::new(document(false, false, false, "alpha"));
    assert!(independent.commit(&first_plan, &context).is_err());
    let mut other_source = PdfRevision::new(document(false, false, false, "delta"));
    assert!(other_source.commit(&first_plan, &context).is_err());
    assert_eq!(branch.text().unwrap(), ["other", "bravo"]);
}

#[test]
fn shared_stream_and_annotation_ownership_are_refused() {
    let context = context(1_000_000);
    for (stream, annotation) in [(true, false), (false, true)] {
        let revision = PdfRevision::new(document(false, stream, annotation, "alpha"));
        let request = if stream {
            PdfRevisionRequest {
                text: vec![text("alpha", "next")],
                annotations: vec![],
                ..Default::default()
            }
        } else {
            PdfRevisionRequest {
                text: vec![],
                annotations: vec![AnnotationContentsMutationRequest {
                    annotation_index: 0,
                    contents: "next".into(),
                }],
                ..Default::default()
            }
        };
        assert!(revision.plan(request, &context).is_err());
        assert_eq!(revision.snapshot().revision().value(), 0);
    }
}

#[test]
fn cancellation_and_mid_stage_work_failure_do_not_advance_revision() {
    let mut revision = PdfRevision::new(document(false, false, false, "alpha"));
    let generous = context(1_000_000);
    let plan = revision.plan(request(), &generous).unwrap();
    let cancelled = context(1_000_000);
    cancelled.cancellation().cancel();
    assert!(revision.commit(&plan, &cancelled).is_err());
    let exhausted = context(1); // One object staged, second object charge fails.
    assert!(revision.commit(&plan, &exhausted).is_err());
    assert_eq!(exhausted.budget().snapshot().work_units, 1);
    assert_eq!(revision.snapshot().revision().value(), 0);
    assert_eq!(revision.text().unwrap(), ["alpha", "bravo"]);
    assert!(revision.commit(&plan, &exhausted).is_err());
    revision.commit(&plan, &generous).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 1);
}

#[test]
fn annotation_parent_conflict_is_not_unique_ownership() {
    let document = document(false, false, false, "alpha");
    let mut bytes = document.input_source().as_bytes().to_vec();
    let offset = bytes
        .windows(8)
        .position(|value| value == b"/P 3 0 R")
        .unwrap();
    bytes[offset + 3] = b'2';
    let conflicting = PdfEngine::default()
        .open_owned(bytes, OpenOptions::default())
        .unwrap();
    let revision = PdfRevision::new(conflicting);
    let request = PdfRevisionRequest {
        text: vec![],
        annotations: vec![AnnotationContentsMutationRequest {
            annotation_index: 0,
            contents: "new".into(),
        }],
        ..Default::default()
    };
    let error = revision.plan(request, &context(1_000_000)).unwrap_err();
    assert!(error.message.contains("contradicts"));
    assert_eq!(revision.snapshot().revision().value(), 0);
    assert_eq!(
        revision.annotations().unwrap()[0].contents.as_deref(),
        Some("old note")
    );
}

#[test]
fn text_annotation_and_reorder_commit_atomically_and_survive_both_writers() {
    let mut revision = PdfRevision::new(document(true, false, false, "alpha"));
    let source = revision.clone();
    let original_ids = revision.page_ids().unwrap();
    let context = context(10_000_000);
    let mut request = request();
    request.reorder_pages = Some(vec![1, 0]);
    let plan = revision.plan(request, &context).unwrap();
    assert_eq!(plan.changed_objects().count(), 3);
    let commit = revision.commit(&plan, &context).unwrap();
    assert_eq!(commit.after.revision().value(), 1);
    assert_eq!(
        revision.page_ids().unwrap(),
        [original_ids[1], original_ids[0]]
    );
    assert_eq!(source.page_ids().unwrap(), original_ids);
    assert_eq!(source.text().unwrap(), ["alpha", "bravo"]);
    assert_eq!(revision.text().unwrap(), ["longer text", "B"]);
    assert_eq!(revision.annotations().unwrap()[0].page_index, 1);
    assert_eq!(
        revision
            .page_labels()
            .unwrap()
            .iter()
            .map(|label| label.label.as_str())
            .collect::<Vec<_>>(),
        ["1", "2"]
    );
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &context).unwrap();
        assert_eq!(output.snapshot, commit.after);
        let reopened = PdfEngine::default()
            .open_source(output.output, OpenOptions::default())
            .unwrap();
        assert_eq!(
            binas_pdf::read_document_info(&reopened)
                .unwrap()
                .title
                .as_deref(),
            Some("Original")
        );
        let reopened = PdfRevision::new(reopened);
        assert_eq!(reopened.page_ids().unwrap(), revision.page_ids().unwrap());
        assert_eq!(reopened.text().unwrap(), revision.text().unwrap());
        assert_eq!(
            reopened.annotations().unwrap()[0].contents.as_deref(),
            Some("café 中文 😀")
        );
        assert_eq!(reopened.annotations().unwrap()[0].page_index, 1);
    }
}

#[test]
fn invalid_reorder_and_failed_commit_leave_all_other_edits_unpublished() {
    let mut revision = PdfRevision::new(document(false, false, false, "alpha"));
    let generous = context(10_000_000);
    let original_ids = revision.page_ids().unwrap();
    for order in [vec![1, 1], vec![0], vec![0, 9]] {
        let mut request = request();
        request.reorder_pages = Some(order);
        assert!(revision.plan(request, &generous).is_err());
    }
    let mut request = request();
    request.reorder_pages = Some(vec![1, 0]);
    let plan = revision.plan(request, &generous).unwrap();
    // All three sparse replacements stage, then verification exhausts the budget.
    let exhausted = context(plan.changed_objects().count() as u64);
    assert!(revision.commit(&plan, &exhausted).is_err());
    assert_eq!(revision.snapshot().revision().value(), 0);
    assert_eq!(revision.page_ids().unwrap(), original_ids);
    assert_eq!(revision.text().unwrap(), ["alpha", "bravo"]);
    assert_eq!(
        revision.annotations().unwrap()[0].contents.as_deref(),
        Some("old note")
    );
    revision.commit(&plan, &generous).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 1);
}
