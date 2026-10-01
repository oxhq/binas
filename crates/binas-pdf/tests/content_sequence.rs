//! Original generated Type3 fixtures; complete lexical tokens may cross Contents parts.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfDocument, PdfEngine, PdfErrorCode,
    PdfPageTextEditRequest, PdfRevision, PdfRevisionRequest, PdfSharedEditMode, PdfWriterMode,
    SurgicalTextEditRequest,
};
use std::collections::BTreeMap;

fn stream(data: &[u8], extra: &str) -> Vec<u8> {
    let mut out = format!("<< /Length {} {extra} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

fn fixture(parts: &[&[u8]], second_font: bool, form: Option<&[u8]>) -> Vec<u8> {
    fixture_with_slots(parts, second_font, form, None)
}

fn fixture_with_slots(
    parts: &[&[u8]],
    second_font: bool,
    form: Option<&[u8]>,
    slots: Option<&[usize]>,
) -> Vec<u8> {
    let mut objects = BTreeMap::<usize, Vec<u8>>::new();
    objects.insert(1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.insert(
        2,
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 200 200] >>".to_vec(),
    );
    let refs = slots
        .map_or_else(
            || (0..parts.len()).collect::<Vec<_>>(),
            |slots| slots.to_vec(),
        )
        .into_iter()
        .map(|i| format!("{} 0 R", 20 + i))
        .collect::<Vec<_>>()
        .join(" ");
    for (page, font) in [(3, 10), (4, if second_font { 11 } else { 10 })] {
        objects.insert(page,format!("<< /Type /Page /Parent 2 0 R /Contents [{refs}] /Resources << /Font << /F1 {font} 0 R /F2 11 0 R >> /XObject << /Fm 12 0 R >> >> >>").into_bytes());
    }
    for (font, cmap) in [(10, 15), (11, 16)] {
        objects.insert(font,format!("<< /Type /Font /Subtype /Type3 /Name /Fixture /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /A 13 0 R /B 14 0 R >> /Encoding << /Type /Encoding /Differences [65 /A /B] >> /FirstChar 65 /LastChar 66 /Widths [600 600] /Resources << >> /ToUnicode {cmap} 0 R >>").into_bytes());
    }
    objects.insert(12,stream(form.unwrap_or(b""),"/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Resources << /Font << /F1 11 0 R >> >>"));
    objects.insert(13, stream(b"600 0 0 0 600 700 d1 50 0 500 700 re f", ""));
    objects.insert(
        14,
        stream(b"600 0 0 0 600 700 d1 0 0 m 300 700 l 600 0 l h f", ""),
    );
    for (number, a, b) in [(15, "03A9", "03A8"), (16, "0416", "042E")] {
        objects.insert(number,stream(format!("/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Fixture{number} def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 2 beginbfchar <41> <{a}> <42> <{b}> endbfchar endcmap CMapName currentdict /CMap defineresource pop end end").as_bytes(),""));
    }
    for (i, part) in parts.iter().enumerate() {
        objects.insert(20 + i, stream(part, ""));
    }
    let size = objects.keys().last().unwrap() + 1;
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = BTreeMap::new();
    for (number, value) in objects {
        offsets.insert(number, bytes.len());
        bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        bytes.extend(value);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for n in 1..size {
        bytes.extend_from_slice(
            match offsets.get(&n) {
                Some(offset) => format!("{offset:010} 00000 n \n"),
                None => "0000000000 00000 f \n".into(),
            }
            .as_bytes(),
        );
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn unicode_request(shared: PdfSharedEditMode) -> PdfRevisionRequest {
    PdfRevisionRequest {
        page_text: vec![PdfPageTextEditRequest {
            page_index: 0,
            edit: SurgicalTextEditRequest {
                old_text: "Ω".into(),
                replacement: "Ψ".into(),
                match_index: 0,
            },
            shared,
        }],
        ..Default::default()
    }
}

#[test]
fn repeated_physical_operand_occurrences_are_not_silently_edited_together() {
    let bytes = fixture_with_slots(
        &[b"BT /F1 12 Tf", b"(A) Tj", b"ET"],
        false,
        None,
        Some(&[0, 1, 1, 2]),
    );
    let document = open(&bytes);
    let extracted = document.extract_page_text_spans(0).unwrap();
    assert_eq!(
        extracted
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["Ω", "Ω"]
    );
    assert_eq!(
        extracted.spans[0].source_span,
        extracted.spans[1].source_span
    );
    for shared in [
        PdfSharedEditMode::Reject,
        PdfSharedEditMode::CloneForPage,
        PdfSharedEditMode::AllPageOwners,
    ] {
        let revision = PdfRevision::new(document.clone());
        let before = revision.snapshot();
        assert!(revision.plan(unicode_request(shared), &context()).is_err());
        assert_eq!(revision.snapshot(), before);
    }
}

#[test]
fn scoped_ascii_in_an_active_font_without_explicit_cmap_is_not_a_raw_edit() {
    let bytes = fixture(&[b"BT /F1 12 Tf", b"(A) Tj ET"], false, None);
    // Same-length dictionary-key replacement keeps the generated xref exact.
    let bytes = String::from_utf8(bytes)
        .unwrap()
        .replace("/ToUnicode", "/NoUnicode")
        .into_bytes();
    let document = open(&bytes);
    let extracted = document.extract_page_text_spans(0).unwrap();
    assert_eq!(extracted.spans[0].font_name.as_deref(), Some("F1"));
    assert!(!extracted.spans[0].to_unicode);
    let revision = PdfRevision::new(document);
    let mut request = unicode_request(PdfSharedEditMode::CloneForPage);
    request.page_text[0].edit.old_text = "A".into();
    request.page_text[0].edit.replacement = "B".into();
    assert!(revision.plan(request, &context()).is_err());
}

#[test]
fn unicode_clone_and_reorder_preserve_complete_page_inventory_and_other_slots() {
    let bytes = fixture(
        &[b"BT /F1 12 Tf (B) Tj", b"(A)", b"Tj (B) Tj ET"],
        true,
        None,
    );
    let mut revision = PdfRevision::new(open(&bytes));
    let ctx = context();
    let mut request = unicode_request(PdfSharedEditMode::CloneForPage);
    request.reorder_pages = Some(vec![1, 0]);
    let plan = revision.plan(request, &ctx).unwrap();
    let effects = plan.stream_effects().collect::<Vec<_>>();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].source_stream.number, 21);
    assert_ne!(effects[0].output_stream.number, 21);
    let cloned = effects[0].output_stream.number;
    revision.commit(&plan, &ctx).unwrap();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let output = revision.export(mode, &ctx).unwrap();
        let reopened = open(output.output.as_bytes());
        for (page, expected, objects) in [
            (0, ["Ю", "Ж", "Ю"], [20, 21, 22]),
            (1, ["Ψ", "Ψ", "Ψ"], [20, cloned, 22]),
        ] {
            let extraction = reopened.extract_page_text_spans(page).unwrap();
            assert!(extraction.warnings.is_empty());
            assert_eq!(
                extraction
                    .spans
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                extraction
                    .spans
                    .iter()
                    .map(|s| s.object_number)
                    .collect::<Vec<_>>(),
                objects
            );
        }
    }
}
fn open(bytes: &[u8]) -> PdfDocument {
    PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap()
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
fn split_operands_keep_physical_provenance_and_page_font_identity() {
    let bytes = fixture(
        &[b"BT /F1 12 Tf 1 0 0 1 10 20 Tm", b"(A)", b"Tj ET"],
        true,
        None,
    );
    let doc = open(&bytes);
    for (page, expected) in [(0, "Ω"), (1, "Ж")] {
        let extraction = doc.extract_page_text_spans(page).unwrap();
        assert!(extraction.warnings.is_empty(), "{:?}", extraction.warnings);
        let span = &extraction.spans[0];
        assert_eq!(extraction.spans.len(), 1);
        assert_eq!(span.text, expected);
        assert!(span.to_unicode);
        assert_eq!(span.object_number, 21);
        assert_eq!(
            span.source_span.unwrap().slice(&bytes),
            Some(b"(A)".as_slice())
        );
        assert_eq!(span.geometry.origin, Some([10., 20.]));
        assert_eq!(span.geometry.font_size, Some(12.));
    }
}

#[test]
fn graphics_restore_font_and_leading_but_not_text_matrix_across_parts() {
    let bytes = fixture(
        &[
            b"BT /F1 12 Tf 14 TL 1 0 0 1 10 90 Tm q",
            b"/F2 20 Tf 30 TL 1 0 0 1 50 80 Tm (A) Tj",
            b"Q T* (A) Tj ET",
        ],
        false,
        None,
    );
    let extraction = open(&bytes).extract_page_text_spans(0).unwrap();
    assert_eq!(
        extraction
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["Ж", "Ω"]
    );
    assert_eq!(extraction.spans[1].geometry.font_size, Some(12.));
    assert_eq!(extraction.spans[1].geometry.origin, Some([50., 66.]));
}

#[test]
fn form_inherits_actual_font_object_and_explicit_tf_uses_form_resources() {
    let bytes = fixture(
        &[b"BT /F1 12 Tf ET /Fm Do BT (A) Tj ET"],
        false,
        Some(b"BT (A) Tj /F1 18 Tf (A) Tj ET"),
    );
    let extraction = open(&bytes).extract_page_text_spans(0).unwrap();
    assert_eq!(
        extraction
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["Ω", "Ж", "Ω"]
    );
    assert_eq!(
        extraction
            .spans
            .iter()
            .map(|s| s.geometry.font_size)
            .collect::<Vec<_>>(),
        [Some(12.), Some(18.), Some(12.)]
    );
}

#[test]
fn complete_sequence_rejects_unbalanced_state_and_honors_shared_limits_and_cancel() {
    for parts in [
        vec![b"BT /F1 12 Tf".as_slice(), b"(A) Tj"],
        vec![b"q".as_slice(), b"BT /F1 12 Tf (A) Tj ET"],
    ] {
        assert!(
            open(&fixture(&parts, false, None))
                .extract_page_text_spans(0)
                .is_err()
        );
    }
    let bytes = fixture(&[b"BT /F1 12 Tf", b"(A) Tj ET"], false, None);
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        open(&bytes)
            .with_cancellation(token)
            .extract_page_text_spans(0)
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
    let doc = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 1,
            ..Limits::default()
        },
    })
    .open_owned(bytes, OpenOptions::default())
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            doc.clone().extract_page_text_spans(0).unwrap_err().code,
            PdfErrorCode::ResourceLimit
        );
    }
}

#[test]
fn scoped_unicode_edits_change_the_operand_stream_and_preserve_other_context() {
    for (page, old, new, expected) in [(0, "Ω", "Ψ", ["Ψ", "Ж"]), (1, "Ж", "Ю", ["Ω", "Ю"])]
    {
        let mut revision = PdfRevision::new(open(&fixture(
            &[b"BT /F1 12 Tf", b"(A)", b"Tj ET"],
            true,
            None,
        )));
        let ctx = context();
        let plan = revision
            .plan(
                PdfRevisionRequest {
                    page_text: vec![PdfPageTextEditRequest {
                        page_index: page,
                        edit: SurgicalTextEditRequest {
                            old_text: old.into(),
                            replacement: new.into(),
                            match_index: 0,
                        },
                        shared: PdfSharedEditMode::CloneForPage,
                    }],
                    ..Default::default()
                },
                &ctx,
            )
            .unwrap();
        assert_eq!(
            plan.stream_effects().next().unwrap().source_stream.number,
            21
        );
        revision.commit(&plan, &ctx).unwrap();
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            let output = revision.export(mode, &ctx).unwrap();
            let reopened = open(output.output.as_bytes());
            for (page, text) in expected.iter().enumerate() {
                assert_eq!(
                    reopened.extract_page_text_spans(page).unwrap().spans[0].text,
                    *text
                );
            }
        }
    }
}

#[test]
fn all_owner_unicode_edit_requires_compatible_meaning_in_every_page_context() {
    for different in [false, true] {
        let mut revision = PdfRevision::new(open(&fixture(
            &[b"BT /F1 12 Tf", b"(A)", b"Tj ET"],
            different,
            None,
        )));
        let ctx = context();
        let planned = revision.plan(
            PdfRevisionRequest {
                page_text: vec![PdfPageTextEditRequest {
                    page_index: 0,
                    edit: SurgicalTextEditRequest {
                        old_text: "Ω".into(),
                        replacement: "Ψ".into(),
                        match_index: 0,
                    },
                    shared: PdfSharedEditMode::AllPageOwners,
                }],
                ..Default::default()
            },
            &ctx,
        );
        if different {
            assert!(planned.is_err());
            continue;
        }
        let plan = planned.unwrap();
        revision.commit(&plan, &ctx).unwrap();
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            let output = revision.export(mode, &ctx).unwrap();
            let document = open(output.output.as_bytes());
            for page in 0..2 {
                assert_eq!(
                    document.extract_page_text_spans(page).unwrap().spans[0].text,
                    "Ψ"
                );
            }
        }
    }
}

#[test]
fn legacy_raw_text_does_not_silently_become_a_unicode_edit() {
    let revision = PdfRevision::new(open(&fixture(&[b"BT /F1 12 Tf (A) Tj ET"], false, None)));
    assert!(
        revision
            .plan(
                PdfRevisionRequest {
                    text: vec![SurgicalTextEditRequest {
                        old_text: "Ω".into(),
                        replacement: "Ψ".into(),
                        match_index: 0
                    }],
                    ..Default::default()
                },
                &context()
            )
            .is_err()
    );
}

#[test]
fn unknown_extgstate_never_supplies_a_complete_editing_proof() {
    let doc = open(&fixture(
        &[b"BT /F1 12 Tf (A) Tj", b"/GS1 gs (A) Tj ET"],
        false,
        None,
    ));
    match doc.extract_page_text_spans(0) {
        Ok(extraction) => assert!(!extraction.warnings.is_empty()),
        Err(error) => assert_eq!(error.code, PdfErrorCode::UnsupportedFeature),
    }
    let revision = PdfRevision::new(doc);
    assert!(
        revision
            .plan(
                PdfRevisionRequest {
                    page_text: vec![PdfPageTextEditRequest {
                        page_index: 0,
                        edit: SurgicalTextEditRequest {
                            old_text: "Ω".into(),
                            replacement: "Ψ".into(),
                            match_index: 0
                        },
                        shared: PdfSharedEditMode::CloneForPage
                    }],
                    ..Default::default()
                },
                &context()
            )
            .is_err()
    );
}

#[test]
fn mandatory_original_fixture_preserves_sequence_and_form_font_contexts() {
    let bytes = include_bytes!("../../../qa/preservation/fixtures/content-sequence.pdf");
    let document = open(bytes);
    for (page, expected) in [(0, ["Ω", "Ω", "Ж", "Ψ"]), (1, ["Ж", "Ж", "Ж", "Ю"])] {
        let extraction = document.extract_page_text_spans(page).unwrap();
        assert!(extraction.warnings.is_empty());
        assert_eq!(
            extraction
                .spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(extraction.spans[0].object_number, 21);
        assert_eq!(
            extraction.spans[0].source_span.unwrap().slice(bytes),
            Some(b"(A)".as_slice())
        );
    }
}

#[test]
fn font_matrix_and_array_operands_can_cross_complete_lexical_token_boundaries() {
    let bytes = fixture(
        &[
            b"BT /F1 12",
            b"Tf 1 0 0",
            b"1 30 40 Tm [(A) -20 (B)]",
            b"TJ ET",
        ],
        false,
        None,
    );
    let extraction = open(&bytes).extract_page_text_spans(0).unwrap();
    assert!(extraction.warnings.is_empty());
    assert_eq!(
        extraction
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["Ω", "Ψ"]
    );
    for (span, token) in extraction.spans.iter().zip([b"(A)", b"(B)"]) {
        assert_eq!(span.object_number, 22);
        assert_eq!(
            span.source_span.unwrap().slice(&bytes),
            Some(token.as_slice())
        );
        assert_eq!(span.geometry.font_size, Some(12.));
    }
    assert_eq!(extraction.spans[0].geometry.origin, Some([30., 40.]));
}
