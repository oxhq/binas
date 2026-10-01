use std::io::Write;

use binas_core::{
    BudgetLimits, CancellationToken, ContextError, OperationContext, ResourceBudget,
    transaction::TransactionError,
};
use binas_pdf::{
    OpenOptions, PdfDocument, PdfEngine, PdfErrorCode, PdfPageTextEditRequest, PdfRevision,
    PdfRevisionRequest, PdfSharedEditMode, PdfWriterMode, SurgicalTextEditRequest,
};

fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 64 * 1024 * 1024,
            max_work_units: 256 * 1024 * 1024,
        }),
        CancellationToken::new(),
    )
}

fn fixture(filtered: bool, slot: &str, text: &str) -> Vec<u8> {
    fixture_custom(filtered, slot, text, "", false)
}

fn fixture_custom(filtered: bool, slot: &str, text: &str, entries: &str, setup: bool) -> Vec<u8> {
    let content = format!("BT ({text}) Tj (beta) Tj ET").into_bytes();
    let encoded = if filtered {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&content).unwrap();
        encoder.finish().unwrap()
    } else {
        content
    };
    let mut stream = format!(
        "<< /Length {} {} {entries} >>\nstream\n",
        encoded.len(),
        if filtered { "/Filter /FlateDecode" } else { "" }
    )
    .into_bytes();
    stream.extend_from_slice(&encoded);
    stream.extend_from_slice(b"\nendstream");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 8 0 R /Names << /Dests << /Names [(landing) [6 0 R /Fit]] >> >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [5 0 R] /Count 1 /MediaBox [0 0 300 400] /CropBox [10 20 290 380] /Rotate 90 /Resources << >> >>".to_vec(),
        b"<< /Type /Pages /Parent 2 0 R /Kids [6 0 R] /Count 1 /MediaBox [0 0 500 600] /CropBox [30 40 470 560] /Rotate 270 /Resources << >> >>".to_vec(),
        format!("<< /Type /Page /Parent 3 0 R /Contents {slot} >>").into_bytes(),
        b"<< /Type /Page /Parent 4 0 R /Contents 7 0 R >>".to_vec(),
        stream,
        b"<< /Nums [0 << /P (A-) /S /D >> 1 << /P (B-) /S /D >>] >>".to_vec(),
    ];
    if setup {
        let data = b"BT /F1 12 Tf ET";
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                data.len(),
                std::str::from_utf8(data).unwrap()
            )
            .into_bytes(),
        );
        objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    }
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

fn document(filtered: bool, slot: &str) -> PdfDocument {
    PdfEngine::default()
        .open_owned(fixture(filtered, slot, "alpha"), OpenOptions::default())
        .unwrap()
}

fn edit(
    page: usize,
    old: &str,
    replacement: &str,
    shared: PdfSharedEditMode,
) -> PdfPageTextEditRequest {
    PdfPageTextEditRequest {
        page_index: page,
        edit: SurgicalTextEditRequest {
            old_text: old.into(),
            replacement: replacement.into(),
            match_index: 0,
        },
        shared,
    }
}

fn request(edits: Vec<PdfPageTextEditRequest>) -> PdfRevisionRequest {
    PdfRevisionRequest {
        page_text: edits,
        ..Default::default()
    }
}

fn page_text(document: &PdfDocument, page: usize) -> String {
    document
        .extract_page_text_spans(page)
        .unwrap()
        .spans
        .into_iter()
        .map(|span| span.text)
        .collect::<Vec<_>>()
        .join("|")
}

fn exports(revision: &PdfRevision, expected: [&str; 2]) -> Vec<PdfDocument> {
    [PdfWriterMode::Incremental, PdfWriterMode::Compact]
        .into_iter()
        .map(|mode| {
            let result = revision.export(mode, &context()).unwrap();
            assert_eq!(result.snapshot, revision.snapshot());
            let reopened = PdfEngine::default()
                .open_source(result.output, OpenOptions::default())
                .unwrap();
            assert_eq!([page_text(&reopened, 0), page_text(&reopened, 1)], expected);
            reopened
        })
        .collect()
}

#[test]
fn raw_and_filtered_scalar_and_direct_array_clones_preserve_other_page() {
    for filtered in [false, true] {
        for slot in ["7 0 R", "[7 0 R]"] {
            let original = document(filtered, slot);
            let original_source = original.source_id();
            let mut revision = PdfRevision::new(original.clone());
            let context = context();
            assert!(
                revision
                    .plan(
                        request(vec![edit(0, "alpha", "local", PdfSharedEditMode::Reject)]),
                        &context
                    )
                    .is_err()
            );
            let plan = revision
                .plan(
                    request(vec![edit(
                        0,
                        "alpha",
                        "local",
                        PdfSharedEditMode::CloneForPage,
                    )]),
                    &context,
                )
                .unwrap();
            let effects = plan.stream_effects().cloned().collect::<Vec<_>>();
            assert_eq!(effects.len(), 1);
            assert_eq!(effects[0].source_stream.number, 7);
            assert!(effects[0].output_stream.number >= 9);
            assert_eq!(effects[0].output_stream.generation, 0);
            assert_eq!(
                effects[0]
                    .pages
                    .iter()
                    .map(|p| p.number)
                    .collect::<Vec<_>>(),
                [5]
            );
            assert_eq!(effects[0].shared, PdfSharedEditMode::CloneForPage);
            assert_eq!(revision.snapshot().revision().value(), 0);
            revision.commit(&plan, &context).unwrap();
            assert_eq!(revision.snapshot().source(), original_source);
            exports(&revision, ["local|beta", "alpha|beta"]);
            assert_eq!(page_text(&original, 0), "alpha|beta");
            assert_eq!(page_text(&original, 1), "alpha|beta");
        }
    }
}

#[test]
fn all_page_owners_changes_both_without_allocating_a_clone() {
    for filtered in [false, true] {
        let mut revision = PdfRevision::new(document(filtered, "[7 0 R]"));
        let context = context();
        let plan = revision
            .plan(
                request(vec![edit(
                    1,
                    "alpha",
                    "global",
                    PdfSharedEditMode::AllPageOwners,
                )]),
                &context,
            )
            .unwrap();
        let effect = plan.stream_effects().next().unwrap();
        assert_eq!(effect.source_stream, effect.output_stream);
        assert_eq!(
            effect.pages.iter().map(|p| p.number).collect::<Vec<_>>(),
            [5, 6]
        );
        revision.commit(&plan, &context).unwrap();
        exports(&revision, ["global|beta", "global|beta"]);
    }
}

#[test]
fn repeated_slots_and_conflicting_or_overlapping_requests_do_not_publish() {
    for shared in [
        PdfSharedEditMode::Reject,
        PdfSharedEditMode::CloneForPage,
        PdfSharedEditMode::AllPageOwners,
    ] {
        let revision = PdfRevision::new(document(false, "[7 0 R 7 0 R]"));
        assert!(
            revision
                .plan(request(vec![edit(0, "alpha", "local", shared)]), &context())
                .is_err()
        );
        assert_eq!(revision.snapshot().revision().value(), 0);
    }
    let revision = PdfRevision::new(document(false, "7 0 R"));
    for edits in [
        vec![
            edit(0, "alpha", "global", PdfSharedEditMode::AllPageOwners),
            edit(1, "beta", "local", PdfSharedEditMode::CloneForPage),
        ],
        vec![
            edit(1, "beta", "local", PdfSharedEditMode::CloneForPage),
            edit(0, "alpha", "global", PdfSharedEditMode::AllPageOwners),
        ],
        vec![
            edit(0, "alpha", "first", PdfSharedEditMode::CloneForPage),
            edit(0, "alpha", "second", PdfSharedEditMode::CloneForPage),
        ],
    ] {
        assert!(revision.plan(request(edits), &context()).is_err());
        assert_eq!(revision.snapshot().revision().value(), 0);
    }
    exports(&revision, ["alpha|beta", "alpha|beta"]);
}

#[test]
fn same_page_clones_coalesce_and_different_pages_receive_distinct_objects() {
    let mut revision = PdfRevision::new(document(true, "7 0 R"));
    let context = context();
    let plan = revision
        .plan(
            request(vec![
                edit(0, "alpha", "first", PdfSharedEditMode::CloneForPage),
                edit(0, "beta", "second", PdfSharedEditMode::CloneForPage),
            ]),
            &context,
        )
        .unwrap();
    assert_eq!(plan.stream_effects().count(), 1);
    revision.commit(&plan, &context).unwrap();
    exports(&revision, ["first|second", "alpha|beta"]);

    let mut revision = PdfRevision::new(document(true, "[7 0 R]"));
    let plan = revision
        .plan(
            request(vec![
                edit(0, "alpha", "left", PdfSharedEditMode::CloneForPage),
                edit(1, "alpha", "right", PdfSharedEditMode::CloneForPage),
            ]),
            &context,
        )
        .unwrap();
    let effects = plan.stream_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 2);
    assert_ne!(effects[0].output_stream, effects[1].output_stream);
    revision.commit(&plan, &context).unwrap();
    exports(&revision, ["left|beta", "right|beta"]);
}

#[test]
fn later_commit_edits_existing_clone_and_preserves_all_prior_changes() {
    let mut revision = PdfRevision::new(document(true, "7 0 R"));
    let context = context();
    let first = revision
        .plan(
            request(vec![edit(
                0,
                "alpha",
                "local",
                PdfSharedEditMode::CloneForPage,
            )]),
            &context,
        )
        .unwrap();
    let clone = first.stream_effects().next().unwrap().output_stream;
    revision.commit(&first, &context).unwrap();
    let second = revision
        .plan(
            request(vec![
                edit(0, "local", "final", PdfSharedEditMode::Reject),
                edit(1, "beta", "other", PdfSharedEditMode::Reject),
            ]),
            &context,
        )
        .unwrap();
    assert!(
        second
            .stream_effects()
            .any(|effect| effect.source_stream == clone && effect.output_stream == clone)
    );
    revision.commit(&second, &context).unwrap();
    assert_eq!(revision.snapshot().revision().value(), 2);
    assert!(matches!(
        revision.commit(&first, &context),
        Err(TransactionError::StalePlan { .. })
    ));
    exports(&revision, ["final|beta", "alpha|other"]);
}

#[test]
fn quota_cancel_and_different_source_fail_without_commit() {
    let mut revision = PdfRevision::new(document(false, "7 0 R"));
    let original = revision.snapshot();
    let none = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 0,
            max_work_units: 0,
        }),
        CancellationToken::new(),
    );
    assert_eq!(
        revision
            .plan(
                request(vec![edit(
                    0,
                    "alpha",
                    "local",
                    PdfSharedEditMode::CloneForPage
                )]),
                &none
            )
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
    let cancel_context = context();
    let plan = revision
        .plan(
            request(vec![edit(
                0,
                "alpha",
                "local",
                PdfSharedEditMode::CloneForPage,
            )]),
            &cancel_context,
        )
        .unwrap();
    let no_commit_work = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1024 * 1024,
            max_work_units: 0,
        }),
        CancellationToken::new(),
    );
    assert!(revision.commit(&plan, &no_commit_work).is_err());
    assert_eq!(revision.snapshot(), original);
    cancel_context.cancellation().cancel();
    assert!(matches!(
        revision.commit(&plan, &cancel_context),
        Err(TransactionError::Context(ContextError::Cancelled))
    ));
    assert_eq!(revision.snapshot(), original);
    let changed_bytes = fixture(false, "7 0 R", "omega");
    assert_eq!(changed_bytes.len(), fixture(false, "7 0 R", "alpha").len());
    let mut changed = PdfRevision::new(
        PdfEngine::default()
            .open_owned(changed_bytes, OpenOptions::default())
            .unwrap(),
    );
    assert!(matches!(
        changed.commit(&plan, &context()),
        Err(TransactionError::StalePlan { .. })
    ));
    exports(&revision, ["alpha|beta", "alpha|beta"]);
}

#[test]
fn mixed_cross_parent_reorder_keeps_clone_attached_to_selected_page() {
    for filtered in [false, true] {
        let original = document(filtered, "[7 0 R]");
        let geometry = [
            original.page_geometry(0).unwrap(),
            original.page_geometry(1).unwrap(),
        ];
        let mut revision = PdfRevision::new(original);
        let context = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    page_text: vec![edit(0, "alpha", "local", PdfSharedEditMode::CloneForPage)],
                    reorder_pages: Some(vec![1, 0]),
                    ..Default::default()
                },
                &context,
            )
            .unwrap();
        assert_eq!(plan.stream_effects().next().unwrap().pages[0].number, 5);
        revision.commit(&plan, &context).unwrap();
        assert_eq!(
            revision
                .page_ids()
                .unwrap()
                .iter()
                .map(|page| page.number)
                .collect::<Vec<_>>(),
            [6, 5]
        );
        for reopened in exports(&revision, ["alpha|beta", "local|beta"]) {
            assert_eq!(reopened.page_geometry(0).unwrap(), geometry[1]);
            assert_eq!(reopened.page_geometry(1).unwrap(), geometry[0]);
            assert_eq!(
                binas_pdf::read_named_destinations(&reopened).unwrap()[0].page_index,
                0
            );
            assert_eq!(
                binas_pdf::read_page_labels(&reopened)
                    .unwrap()
                    .iter()
                    .map(|page| page.label.as_str())
                    .collect::<Vec<_>>(),
                ["B-1", "A-1"]
            );
        }
    }
}

// Narrow inspection of these generated, flat stream dictionaries. Reparse and
// semantic extraction assertions remain separate; this is not a PDF parser.
fn fixture_stream_dl(bytes: &[u8], number: u32) -> Option<String> {
    let marker = format!("\n{number} 0 obj");
    let start = bytes
        .windows(marker.len())
        .rposition(|part| part == marker.as_bytes())
        .unwrap()
        + marker.len();
    let end = start
        + bytes[start..]
            .windows(b"\nstream".len())
            .position(|part| part == b"\nstream")
            .unwrap();
    let dictionary = std::str::from_utf8(&bytes[start..end]).unwrap();
    let mut tokens = dictionary.split_ascii_whitespace();
    while let Some(token) = tokens.next() {
        if token == "/DL" {
            return Some(tokens.next().unwrap().to_owned());
        }
    }
    None
}

#[test]
fn clone_updates_its_decoded_length_hint_and_preserves_original_hint() {
    let before_len = b"BT (alpha) Tj (beta) Tj ET".len();
    let after_len = b"BT (much-longer-local) Tj (beta) Tj ET".len();
    for filtered in [false, true] {
        for hint in [
            format!("/DL {before_len}"),
            "/DL null".into(),
            String::new(),
        ] {
            let input = fixture_custom(filtered, "7 0 R", "alpha", &hint, false);
            let original_dl = fixture_stream_dl(&input, 7);
            let mut revision = PdfRevision::new(
                PdfEngine::default()
                    .open_owned(input, OpenOptions::default())
                    .unwrap(),
            );
            let context = context();
            let plan = revision
                .plan(
                    request(vec![edit(
                        0,
                        "alpha",
                        "much-longer-local",
                        PdfSharedEditMode::CloneForPage,
                    )]),
                    &context,
                )
                .unwrap();
            let cloned = plan.stream_effects().next().unwrap().output_stream.number;
            revision.commit(&plan, &context).unwrap();
            for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
                let output = revision.export(mode, &context).unwrap();
                assert_eq!(fixture_stream_dl(output.output.as_bytes(), 7), original_dl);
                let actual = fixture_stream_dl(output.output.as_bytes(), cloned);
                if original_dl.as_deref().is_some_and(|value| value != "null") {
                    assert_eq!(actual, Some(after_len.to_string()));
                } else {
                    assert!(actual.is_none() || actual.as_deref() == Some("null"));
                }
                let reopened = PdfEngine::default()
                    .open_source(output.output, OpenOptions::default())
                    .unwrap();
                assert_eq!(
                    [page_text(&reopened, 0), page_text(&reopened, 1)],
                    ["much-longer-local|beta", "alpha|beta"]
                );
            }
        }
    }
}

#[test]
fn malformed_decoded_hints_and_opaque_clone_dictionary_keys_refuse() {
    for entries in [
        "/DL -1",
        "/DL (wrong)",
        "/DL [20]",
        "/DL 20.5",
        "/PrivateResource (opaque)",
    ] {
        let revision = PdfRevision::new(
            PdfEngine::default()
                .open_owned(
                    fixture_custom(true, "7 0 R", "alpha", entries, false),
                    OpenOptions::default(),
                )
                .unwrap(),
        );
        let snapshot = revision.snapshot();
        let error = revision
            .plan(
                request(vec![edit(
                    0,
                    "alpha",
                    "local",
                    PdfSharedEditMode::CloneForPage,
                )]),
                &context(),
            )
            .unwrap_err();
        assert!(matches!(
            error.code,
            PdfErrorCode::UnsafeRewrite | PdfErrorCode::UnsupportedFeature
        ));
        assert_eq!(revision.snapshot(), snapshot);
        // Planning failure did not create a revision or poison subsequent reads.
        assert_eq!(revision.text().unwrap(), ["alpha", "beta", "alpha", "beta"]);
        let cancel = context();
        cancel.cancellation().cancel();
        assert_eq!(
            revision
                .plan(
                    request(vec![edit(
                        0,
                        "alpha",
                        "local",
                        PdfSharedEditMode::CloneForPage
                    )]),
                    &cancel
                )
                .unwrap_err()
                .code,
            PdfErrorCode::Cancelled
        );
        assert_eq!(revision.snapshot(), snapshot);
    }
}

#[test]
fn cross_stream_font_setup_cannot_masquerade_as_an_eligible_raw_token() {
    let input = fixture_custom(
        false,
        "[9 0 R 7 0 R] /Resources << /Font << /F1 10 0 R >> >>",
        "alpha",
        "",
        true,
    );
    let source = PdfEngine::default()
        .open_owned(input, OpenOptions::default())
        .unwrap();
    // Font state crosses Contents streams; this font has no ToUnicode mapping,
    // so the retained Unicode edit cannot pretend the bytes are raw ASCII.
    assert_eq!(
        source.extract_page_text_spans(0).unwrap().spans[0]
            .font_name
            .as_deref(),
        Some("F1")
    );
    let revision = PdfRevision::new(source);
    for (page, mode) in [
        (0, PdfSharedEditMode::CloneForPage),
        (1, PdfSharedEditMode::AllPageOwners),
    ] {
        assert!(
            revision
                .plan(
                    request(vec![edit(page, "alpha", "local", mode)]),
                    &context()
                )
                .is_err()
        );
        assert_eq!(revision.snapshot().revision().value(), 0);
    }
}
