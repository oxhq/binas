//! Original retained-geometry regressions; the included fixtures are mandatory
//! Apache-2.0 Binas QA inputs. Assertions use requested values and stable identities.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    EngineConfig, OpenOptions, PageGeometry, PageTransform, PdfDocument, PdfEngine, PdfErrorCode,
    PdfPageTextEditRequest, PdfPageTransformRequest, PdfReferenceIndex, PdfReferenceOrigin,
    PdfReferencePathStep, PdfReferenceTarget, PdfRevision, PdfRevisionRequest, PdfSharedEditMode,
    PdfWriterMode, SurgicalTextEditRequest, list_annotations, list_form_fields, read_document_info,
    read_embedded_attachment_bytes, read_embedded_attachments, read_named_destinations,
    read_outlines, read_page_labels,
};

const UNICODE: &[u8] = include_bytes!("../../../qa/preservation/fixtures/content-sequence.pdf");
const STRUCTURES: &[u8] =
    include_bytes!("../../../qa/preservation/fixtures/reorder-structures.pdf");

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
fn geometry(media_box: [f64; 4], crop_box: [f64; 4], rotation: i32) -> PageGeometry {
    PageGeometry {
        media_box,
        crop_box,
        rotation_degrees: rotation,
        bleed_box: None,
        trim_box: None,
        art_box: None,
    }
}
fn page_transform(page_index: usize, transform: PageTransform) -> PdfPageTransformRequest {
    PdfPageTransformRequest {
        page_index,
        transform,
    }
}
fn geometry_request(page_index: usize, transform: PageTransform) -> PdfRevisionRequest {
    PdfRevisionRequest {
        page_transforms: vec![page_transform(page_index, transform)],
        ..Default::default()
    }
}
fn mixed_request() -> PdfRevisionRequest {
    PdfRevisionRequest {
        page_text: vec![PdfPageTextEditRequest {
            page_index: 0,
            shared: PdfSharedEditMode::CloneForPage,
            edit: SurgicalTextEditRequest {
                old_text: "Ω".into(),
                replacement: "Ψ".into(),
                match_index: 0,
            },
        }],
        page_transforms: vec![page_transform(
            0,
            PageTransform {
                rotation_degrees: Some(450),
                media_box: Some([0., 0., 240., 260.]),
                crop_box: Some([5., 6., 230., 250.]),
                ..Default::default()
            },
        )],
        reorder_pages: Some(vec![1, 0]),
        ..Default::default()
    }
}

#[test]
fn unicode_geometry_and_reorder_bind_original_page_identity_and_commit_once() {
    let mut revision = PdfRevision::new(open(UNICODE));
    let context = context();
    let source_snapshot = revision.snapshot();
    let original = geometry([0., 0., 200., 200.], [0., 0., 200., 200.], 0);
    let expected = geometry([0., 0., 240., 260.], [5., 6., 230., 250.], 90);
    let plan = revision.plan(mixed_request(), &context).unwrap();
    let effects: Vec<_> = plan.geometry_effects().collect();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].page, reference(3));
    assert_eq!(effects[0].before, original);
    assert_eq!(effects[0].after, expected);
    assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    assert_eq!(revision.page_geometry(0).unwrap(), original);
    assert_eq!(
        revision.text().unwrap(),
        ["Ω", "Ω", "Ж", "Ψ", "Ж", "Ж", "Ж", "Ю"]
    );
    let receipt = revision.commit(&plan, &context).unwrap();
    assert_eq!(receipt.before, source_snapshot);
    assert_eq!(receipt.after.revision().value(), 1);
    assert_eq!(receipt.after.source(), source_snapshot.source());
    assert_eq!(revision.page_ids().unwrap(), [reference(4), reference(3)]);
    assert_eq!(revision.page_geometry(0).unwrap(), original);
    assert_eq!(revision.page_geometry(1).unwrap(), expected);
    assert_eq!(
        revision.text().unwrap(),
        ["Ж", "Ж", "Ж", "Ю", "Ψ", "Ω", "Ж", "Ψ"]
    );
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = revision.export(mode, &context).unwrap();
        assert_eq!(exported.snapshot, receipt.after);
        if mode == PdfWriterMode::Incremental {
            assert!(exported.output.as_bytes().starts_with(UNICODE));
        }
        let reopened = PdfRevision::new(open(exported.output.as_bytes()));
        assert_eq!(reopened.page_ids().unwrap(), [reference(4), reference(3)]);
        assert_eq!(reopened.page_geometry(0).unwrap(), original);
        assert_eq!(reopened.page_geometry(1).unwrap(), expected);
        assert_eq!(
            reopened.text().unwrap(),
            ["Ж", "Ж", "Ж", "Ю", "Ψ", "Ω", "Ж", "Ψ"]
        );
    }
}

fn key(name: &[u8]) -> PdfReferencePathStep {
    PdfReferencePathStep::Key(name.to_vec())
}
fn index(value: usize) -> PdfReferencePathStep {
    PdfReferencePathStep::Index(value)
}
fn assert_edge(
    index: &PdfReferenceIndex,
    source: u32,
    target: u32,
    path: Vec<PdfReferencePathStep>,
) {
    assert!(
        index.records().iter().any(|edge| edge.origin
            == PdfReferenceOrigin::Object(reference(source))
            && edge.target == reference(target)
            && edge.path == path
            && !edge.dangling),
        "missing edge {source} -> {target} at {path:?}"
    );
}

#[test]
fn geometry_and_reorder_preserve_structured_fixture_in_both_writers() {
    let mut revision = PdfRevision::new(open(STRUCTURES));
    let context = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                page_transforms: vec![page_transform(
                    1,
                    PageTransform {
                        crop_box: Some([20., 30., 280., 370.]),
                        rotation_degrees: Some(180),
                        ..Default::default()
                    },
                )],
                reorder_pages: Some(vec![2, 0, 1]),
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    let effects: Vec<_> = plan.geometry_effects().collect();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].page, reference(5));
    assert_eq!(
        effects[0].before,
        geometry([0., 0., 300., 400.], [10., 20., 290., 380.], 90)
    );
    assert_eq!(
        effects[0].after,
        geometry([0., 0., 300., 400.], [20., 30., 280., 370.], 180)
    );
    revision.commit(&plan, &context).unwrap();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let exported = revision.export(mode, &context).unwrap();
        let reopened = open(exported.output.as_bytes());
        assert_eq!(
            reopened.page_geometry(0).unwrap(),
            geometry([0., 0., 500., 600.], [30., 40., 470., 560.], 270)
        );
        assert_eq!(
            reopened.page_geometry(1).unwrap(),
            geometry([0., 0., 300., 400.], [10., 20., 290., 380.], 90)
        );
        assert_eq!(
            reopened.page_geometry(2).unwrap(),
            geometry([0., 0., 300., 400.], [20., 30., 280., 370.], 180)
        );
        assert_eq!(
            read_document_info(&reopened).unwrap().title.as_deref(),
            Some("Résumé 東京")
        );
        let outlines = read_outlines(&reopened).unwrap();
        assert_eq!(outlines.len(), 1);
        assert_eq!(outlines[0].title, "Résumé 東京");
        let destinations = read_named_destinations(&reopened).unwrap();
        assert_eq!(destinations.len(), 1);
        assert_eq!(
            (destinations[0].name.as_str(), destinations[0].page_index),
            ("landing", 2)
        );
        assert_eq!(
            read_page_labels(&reopened)
                .unwrap()
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["C-1", "A-1", "B-1"]
        );
        let fields = list_form_fields(&reopened).unwrap();
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "customer");
        assert_eq!(fields[0].value.as_deref(), Some("Zoë"));
        assert_eq!(fields[0].object_number, Some(25));
        let annotations = list_annotations(&reopened).unwrap();
        assert_eq!(annotations.len(), 2);
        assert!(
            annotations
                .iter()
                .all(|item| item.page_index == 1 && item.page_object_number == 4)
        );
        assert!(annotations.iter().any(|item| item.subtype == "Link"
            && item.object_number == Some(35)
            && item.rect == [30., 80., 150., 100.]));
        let attachments = read_embedded_attachments(&reopened).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "note.txt");
        assert_eq!(
            read_embedded_attachment_bytes(&reopened, &attachments[0]).unwrap(),
            b"Original attachment: preserve these exact bytes.\n"
        );
        let references = PdfReferenceIndex::build(&reopened, &context).unwrap();
        assert!(references.records().iter().all(|edge| !edge.dangling));
        assert_edge(&references, 1, 26, vec![key(b"StructTreeRoot")]);
        assert_edge(&references, 26, 36, vec![key(b"ParentTree")]);
        for (slot, element) in [(0, 27), (1, 28), (2, 29), (3, 37)] {
            assert_edge(&references, 26, element, vec![key(b"K"), index(slot)]);
        }
        for (slot, element) in [(1, 27), (3, 28), (5, 29)] {
            assert_edge(
                &references,
                36,
                element,
                vec![key(b"Nums"), index(slot), index(0)],
            );
        }
        assert_edge(&references, 36, 37, vec![key(b"Nums"), index(7)]);
        for (element, page) in [(27, 4), (28, 5), (29, 6), (37, 4)] {
            assert_edge(&references, element, page, vec![key(b"Pg")]);
        }
        assert_edge(&references, 37, 25, vec![key(b"K"), key(b"Obj")]);
        assert_edge(&references, 24, 25, vec![key(b"Fields"), index(0)]);
        assert_edge(&references, 24, 25, vec![key(b"CO"), index(0)]);
        assert_edge(&references, 21, 6, vec![key(b"Dest"), index(0)]);
        assert_edge(&references, 35, 6, vec![key(b"Dest"), index(0)]);
    }
}

fn nested_bytes(first: &str, second: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
        format!("<< /Type /Pages /Parent 2 0 R /Kids [5 0 R] /Count 1 {first} >>"),
        format!("<< /Type /Pages /Parent 2 0 R /Kids [6 0 R] /Count 1 {second} >>"),
        "<< /Type /Page /Parent 3 0 R /BleedBox [1 1 199 199] /TrimBox [2 2 198 198] /ArtBox [3 3 197 197] >>".into(),
        "<< /Type /Page /Parent 4 0 R >>".into(),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
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
const FIRST: &str = "/MediaBox [0 0 200 200] /Rotate 90";
const SECOND: &str = "/MediaBox [0 0 300 400] /CropBox [10 30 290 380] /Rotate 270";

#[test]
fn media_only_after_cross_parent_reorder_keeps_the_source_absent_crop_semantics() {
    let mut revision = PdfRevision::new(open(&nested_bytes(FIRST, SECOND)));
    let context = context();
    let plan = revision
        .plan(
            PdfRevisionRequest {
                page_transforms: vec![page_transform(
                    0,
                    PageTransform {
                        media_box: Some([0., 0., 500., 600.]),
                        ..Default::default()
                    },
                )],
                reorder_pages: Some(vec![1, 0]),
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    let mut original = geometry([0., 0., 200., 200.], [0., 0., 200., 200.], 90);
    original.bleed_box = Some([1., 1., 199., 199.]);
    original.trim_box = Some([2., 2., 198., 198.]);
    original.art_box = Some([3., 3., 197., 197.]);
    let expected = PageGeometry {
        media_box: [0., 0., 500., 600.],
        crop_box: [0., 0., 500., 600.],
        ..original
    };
    let effects: Vec<_> = plan.geometry_effects().collect();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].page, reference(5));
    assert_eq!(effects[0].before, original);
    assert_eq!(effects[0].after, expected);
    revision.commit(&plan, &context).unwrap();
    assert_eq!(revision.page_ids().unwrap(), [reference(6), reference(5)]);
    assert_eq!(revision.page_geometry(1).unwrap(), expected);
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let result = revision.export(mode, &context).unwrap();
        let reopened = open(result.output.as_bytes());
        assert_eq!(reopened.page_geometry(1).unwrap(), expected);
        assert_eq!(
            reopened.page_geometry(0).unwrap(),
            geometry([0., 0., 300., 400.], [10., 30., 290., 380.], 270)
        );
    }
}

#[test]
fn inherited_crop_survives_media_only_and_disjoint_requests_validate_the_final_state() {
    let mut revision = PdfRevision::new(open(&nested_bytes(FIRST, SECOND)));
    let context = context();
    let first = revision
        .plan(
            geometry_request(
                1,
                PageTransform {
                    media_box: Some([0., 0., 400., 500.]),
                    ..Default::default()
                },
            ),
            &context,
        )
        .unwrap();
    revision.commit(&first, &context).unwrap();
    assert_eq!(
        revision.page_geometry(1).unwrap(),
        geometry([0., 0., 400., 500.], [10., 30., 290., 380.], 270)
    );
    let combined = revision
        .plan(
            PdfRevisionRequest {
                page_transforms: vec![
                    page_transform(
                        1,
                        PageTransform {
                            crop_box: Some([20., 30., 480., 580.]),
                            ..Default::default()
                        },
                    ),
                    page_transform(
                        1,
                        PageTransform {
                            media_box: Some([0., 0., 500., 600.]),
                            ..Default::default()
                        },
                    ),
                    page_transform(
                        1,
                        PageTransform {
                            rotation_degrees: Some(-270),
                            ..Default::default()
                        },
                    ),
                ],
                ..Default::default()
            },
            &context,
        )
        .unwrap();
    assert_eq!(combined.geometry_effects().count(), 1);
    revision.commit(&combined, &context).unwrap();
    assert_eq!(
        revision.page_geometry(1).unwrap(),
        geometry([0., 0., 500., 600.], [20., 30., 480., 580.], 90)
    );
    assert_eq!(
        revision.page_geometry(0).unwrap().media_box,
        [0., 0., 200., 200.]
    );
    assert_eq!(revision.page_geometry(0).unwrap().rotation_degrees, 90);
}

#[test]
fn invalid_transforms_and_overlapping_keys_refuse_without_publishing_any_edit() {
    let revision = PdfRevision::new(open(UNICODE));
    let context = context();
    let snapshot = revision.snapshot();
    for transform in [
        PageTransform {
            rotation_degrees: Some(45),
            ..Default::default()
        },
        PageTransform {
            media_box: Some([0., 0., 0., 200.]),
            ..Default::default()
        },
        PageTransform {
            crop_box: Some([20., 20., 10., 10.]),
            ..Default::default()
        },
        PageTransform {
            crop_box: Some([-1., 0., 200., 200.]),
            ..Default::default()
        },
        PageTransform {
            media_box: Some([0., 0., f64::NAN, 200.]),
            ..Default::default()
        },
        PageTransform {
            crop_box: Some([0., 0., 200., f64::INFINITY]),
            ..Default::default()
        },
        PageTransform {
            translate: Some([0., 0.]),
            ..Default::default()
        },
        PageTransform {
            scale: Some([1., 1.]),
            ..Default::default()
        },
    ] {
        let mut request = mixed_request();
        request.page_transforms = vec![page_transform(0, transform)];
        assert!(revision.plan(request, &context).is_err());
        assert_eq!(revision.snapshot(), snapshot);
        assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
        assert_eq!(
            revision.text().unwrap(),
            ["Ω", "Ω", "Ж", "Ψ", "Ж", "Ж", "Ж", "Ю"]
        );
    }
    let rotation = page_transform(
        0,
        PageTransform {
            rotation_degrees: Some(90),
            ..Default::default()
        },
    );
    assert!(
        revision
            .plan(
                PdfRevisionRequest {
                    page_transforms: vec![rotation.clone(), rotation],
                    ..Default::default()
                },
                &context
            )
            .is_err()
    );
    assert_eq!(
        revision
            .plan(
                geometry_request(
                    usize::MAX,
                    PageTransform {
                        rotation_degrees: Some(90),
                        ..Default::default()
                    }
                ),
                &context
            )
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
    let inherited = PdfRevision::new(open(&nested_bytes(FIRST, SECOND)));
    assert!(
        inherited
            .plan(
                geometry_request(
                    1,
                    PageTransform {
                        media_box: Some([0., 0., 50., 50.]),
                        ..Default::default()
                    }
                ),
                &context
            )
            .is_err()
    );
    assert_eq!(inherited.snapshot().revision().value(), 0);
}

#[test]
fn malformed_source_geometry_is_not_silently_repaired_by_an_unrelated_rotation() {
    for attributes in [
        "/MediaBox [0 0 200] /Rotate 90",
        "/MediaBox [0 0 200 200] /CropBox [0 0 300 300]",
        "/MediaBox [0 0 200 200] /Rotate 45",
        "/MediaBox [0 0 200 200] /Rotate /Unknown",
    ] {
        let revision = PdfRevision::new(open(&nested_bytes(attributes, SECOND)));
        assert!(
            revision
                .plan(
                    geometry_request(
                        0,
                        PageTransform {
                            rotation_degrees: Some(180),
                            ..Default::default()
                        }
                    ),
                    &context()
                )
                .is_err()
        );
        assert_eq!(revision.snapshot().revision().value(), 0);
    }
}

#[test]
fn oversized_geometry_batches_fail_the_configured_count_limit() {
    let mut config = EngineConfig::default();
    // Leave enough parsed-value capacity for the source fixture, then exceed
    // that same configured limit with operation requests.
    config.limits.max_container_items = 256;
    let document = PdfEngine::new(config)
        .open_owned(nested_bytes(FIRST, SECOND), OpenOptions::default())
        .unwrap();
    let revision = PdfRevision::new(document);
    let request = PdfRevisionRequest {
        page_transforms: vec![
            page_transform(
                0,
                PageTransform {
                    rotation_degrees: Some(90),
                    ..Default::default()
                }
            );
            257
        ],
        ..Default::default()
    };
    assert_eq!(
        revision.plan(request, &context()).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(revision.snapshot().revision().value(), 0);
}

#[test]
fn cancellation_failed_commit_and_stale_plans_leave_the_committed_geometry_usable() {
    let mut revision = PdfRevision::new(open(UNICODE));
    let generous = context();
    let snapshot = revision.snapshot();
    let original = geometry([0., 0., 200., 200.], [0., 0., 200., 200.], 0);
    let cancelled = context();
    cancelled.cancellation().cancel();
    assert_eq!(
        revision.plan(mixed_request(), &cancelled).unwrap_err().code,
        PdfErrorCode::Cancelled
    );
    let plan = revision.plan(mixed_request(), &generous).unwrap();
    assert!(revision.commit(&plan, &cancelled).is_err());
    // Permit staging the sparse objects, then exhaust verification work.
    let exhausted = limited_context(128 * 1024 * 1024, plan.changed_objects().count() as u64);
    assert!(revision.commit(&plan, &exhausted).is_err());
    assert_eq!(revision.snapshot(), snapshot);
    assert_eq!(revision.page_geometry(0).unwrap(), original);
    assert_eq!(revision.page_geometry(1).unwrap(), original);
    assert_eq!(revision.page_ids().unwrap(), [reference(3), reference(4)]);
    assert_eq!(
        revision.text().unwrap(),
        ["Ω", "Ω", "Ж", "Ψ", "Ж", "Ж", "Ж", "Ю"]
    );
    revision.commit(&plan, &generous).unwrap();
    let committed = revision.snapshot();
    assert!(revision.commit(&plan, &generous).is_err());
    assert_eq!(revision.snapshot(), committed);
    assert_eq!(
        revision.page_geometry(1).unwrap(),
        geometry([0., 0., 240., 260.], [5., 6., 230., 250.], 90)
    );
}
