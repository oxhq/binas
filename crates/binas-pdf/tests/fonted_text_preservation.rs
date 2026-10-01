//! P3b evidence for the existing page_text API, not a new negotiated capability.
//! External reader/render results live in qa/fonted-text and never in receipts.
use binas_core::verification::CheckStatus;
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    OpenOptions, PdfEngine, PdfErrorCode, PdfPageTextEditRequest, PdfRevision, PdfRevisionRequest,
    PdfSharedEditMode, PdfTextEditError, PdfWriterMode, SurgicalTextEditRequest,
};
use std::path::Path;

fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 10_000_000,
            max_work_units: 10_000_000,
        }),
        CancellationToken::new(),
    )
}
fn revision(input: &[u8]) -> PdfRevision {
    PdfRevision::new(
        PdfEngine::default()
            .open_owned(input.to_vec(), OpenOptions::default())
            .unwrap(),
    )
}
fn edit() -> SurgicalTextEditRequest {
    SurgicalTextEditRequest {
        old_text: "ALPHA".into(),
        replacement: "OMEGA-LONG".into(),
        match_index: 0,
    }
}
fn request() -> PdfRevisionRequest {
    PdfRevisionRequest {
        page_text: vec![PdfPageTextEditRequest {
            page_index: 0,
            edit: edit(),
            shared: PdfSharedEditMode::Reject,
        }],
        ..Default::default()
    }
}

#[test]
fn explicit_cmap_page_edit_preserves_shared_fonts_and_exports_both_modes() {
    for (shape, input) in [
        (
            "plain",
            include_bytes!("fixtures/fonted-text/fonted-text-plain.pdf").as_slice(),
        ),
        (
            "flate",
            include_bytes!("fixtures/fonted-text/fonted-text-flate.pdf").as_slice(),
        ),
    ] {
        let document = PdfEngine::default()
            .open_owned(input.to_vec(), OpenOptions::default())
            .unwrap();
        let selected = document.extract_page_text_spans(0).unwrap();
        assert!(selected.warnings.is_empty());
        assert_eq!(
            selected
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            ["ALPHA", "KEEP"]
        );
        assert!(
            selected
                .spans
                .iter()
                .all(|span| span.font_name.as_deref() == Some("F1") && span.to_unicode)
        );
        let mut live = PdfRevision::new(document);
        let prior = live.clone();
        let before = live.snapshot();
        assert_eq!(
            live.text().unwrap(),
            ["ALPHA", "KEEP", "UNTOUCHED PAGE ONE", "UNTOUCHED PAGE TWO"]
        );
        // P2b's raw-ASCII negotiation must remain narrow and refuse this fonted input.
        assert!(
            matches!(live.prepare_text_edit(edit(), PdfWriterMode::Compact, 1, &context()), Err(PdfTextEditError::Pdf(error)) if error.code == PdfErrorCode::UnsupportedFeature)
        );
        let plan = live.plan(request(), &context()).unwrap();
        assert_eq!(live.snapshot(), before);
        let font = plan.font_effects().next().unwrap();
        assert_eq!(plan.font_effects().count(), 1);
        assert_eq!(font.page.number, 3);
        assert_eq!(font.source_stream.number, 4);
        assert_eq!(font.font.unwrap().number, 8);
        assert_eq!(font.cmap.number, 27);
        assert_eq!(font.encoded_bytes, "OMEGA-LONG".len());
        let stream = plan.stream_effects().next().unwrap();
        assert_eq!(plan.stream_effects().count(), 1);
        assert_eq!(stream.source_stream.number, 4);
        assert_eq!(stream.output_stream.number, 4);
        assert_eq!(
            stream
                .pages
                .iter()
                .map(|page| page.number)
                .collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(stream.shared, PdfSharedEditMode::Reject);
        let committed = live.commit(&plan, &context()).unwrap();
        committed.evidence.validate(&committed.required).unwrap();
        assert_eq!(committed.before, before);
        assert_eq!(committed.after, live.snapshot());
        assert_eq!(
            committed
                .changes
                .iter()
                .map(|id| (id.number, id.generation))
                .collect::<Vec<_>>(),
            [(4, 0)]
        );
        assert_eq!(committed.evidence.checks().len(), 11);
        assert!(
            committed
                .evidence
                .checks()
                .iter()
                .all(|check| check.status == CheckStatus::Passed)
        );
        assert_eq!(
            live.text().unwrap(),
            [
                "OMEGA-LONG",
                "KEEP",
                "UNTOUCHED PAGE ONE",
                "UNTOUCHED PAGE TWO"
            ]
        );
        assert_eq!(prior.snapshot(), before);
        assert_eq!(
            prior.text().unwrap(),
            ["ALPHA", "KEEP", "UNTOUCHED PAGE ONE", "UNTOUCHED PAGE TWO"]
        );
        for (name, mode) in [
            ("incremental", PdfWriterMode::Incremental),
            ("compact", PdfWriterMode::Compact),
        ] {
            let exported = live.export(mode, &context()).unwrap();
            let repeated = live.export(mode, &context()).unwrap();
            assert_eq!(exported.output.as_bytes(), repeated.output.as_bytes());
            assert_eq!(exported.snapshot, committed.after);
            assert_eq!(exported.evidence.checks().len(), 7);
            assert!(
                exported
                    .evidence
                    .checks()
                    .iter()
                    .all(|check| check.status == CheckStatus::Passed)
            );
            assert_eq!(
                revision(exported.output.as_bytes()).text().unwrap(),
                [
                    "OMEGA-LONG",
                    "KEEP",
                    "UNTOUCHED PAGE ONE",
                    "UNTOUCHED PAGE TWO"
                ]
            );
            if mode == PdfWriterMode::Incremental {
                assert!(exported.output.as_bytes().starts_with(input));
            }
            if let Some(directory) = std::env::var_os("BINAS_P3B_OUTPUT_DIR") {
                let directory = Path::new(&directory);
                std::fs::create_dir_all(directory).unwrap();
                let stem = format!("{shape}-{name}");
                std::fs::write(
                    directory.join(format!("{stem}.pdf")),
                    exported.output.as_bytes(),
                )
                .unwrap();
                let observation = serde_json::json!({
                    "api": "PdfRevision::plan(PdfRevisionRequest.page_text)/commit/export",
                    "evidence_kind": "runtime_same_engine_commit_export_observation",
                    "external_suite_claims_in_runtime_receipt": false,
                    "negotiated_fonted_capability_added": false,
                    "input_id": format!("{:?}", before.source()), "input_len": input.len(),
                    "output_id": format!("{:?}", exported.output.id()), "output_len": exported.output.len(),
                    "mode": name, "engine_version": exported.engine_version,
                    "before": format!("{:?}", committed.before), "after": format!("{:?}", committed.after),
                    "request": {"page_index": 0, "old_text": "ALPHA", "replacement": "OMEGA-LONG", "match_index": 0, "shared": "Reject"},
                    "changed_objects": [{"number": 4, "generation": 0}],
                    "commit_checks": committed.evidence.checks().iter().map(|check| serde_json::json!({"namespace": check.code.namespace(), "code": check.code.code(), "status": format!("{:?}", check.status)})).collect::<Vec<_>>(),
                    "export_checks": exported.evidence.checks().iter().map(|check| serde_json::json!({"namespace": check.code.namespace(), "code": check.code.code(), "status": format!("{:?}", check.status)})).collect::<Vec<_>>()
                });
                std::fs::write(
                    directory.join(format!("{stem}-observation.json")),
                    serde_json::to_vec_pretty(&observation).unwrap(),
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn unsupported_or_unsafe_fonted_edits_leave_revision_unchanged() {
    for (name, input, expected) in [
        (
            "missing CMap",
            include_bytes!("fixtures/fonted-text/fonted-text-missing-cmap.pdf").as_slice(),
            PdfErrorCode::UnsupportedFeature,
        ),
        (
            "unmapped replacement",
            include_bytes!("fixtures/fonted-text/fonted-text-missing-replacement.pdf").as_slice(),
            PdfErrorCode::UnsupportedFeature,
        ),
        (
            "ambiguous reverse CMap",
            include_bytes!("fixtures/fonted-text/fonted-text-ambiguous-cmap.pdf").as_slice(),
            PdfErrorCode::UnsafeRewrite,
        ),
        (
            "shared target content",
            include_bytes!("fixtures/fonted-text/fonted-text-shared-contents.pdf").as_slice(),
            PdfErrorCode::UnsafeRewrite,
        ),
    ] {
        let live = revision(input);
        let before = live.snapshot();
        let text = live.text().unwrap();
        match live.plan(request(), &context()) {
            Err(error) => assert_eq!(error.code, expected, "{name}: {error}"),
            Ok(_) => panic!("{name}: unsafe or unsupported plan was accepted"),
        }
        assert_eq!(live.snapshot(), before, "{name}");
        assert_eq!(live.text().unwrap(), text, "{name}");
    }
}
