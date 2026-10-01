use binas_pdf::{
    DocumentEntryLocation, OmittedDocumentEntry, OpenOptions, PageAssemblyPolicy, PageTransform,
    PdfEngine, PdfErrorCode, list_annotations,
};

// Generated Binas fixtures: each structure is reachable only through the catalog.
// These isolate assembly loss; they are not third-party compatibility evidence.
fn catalog_structure_pdf(key: &str) -> Vec<u8> {
    let structure = match key {
        "Metadata" => {
            stream(b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">catalog-only-marker</x:xmpmeta>")
        }
        "Outlines" => b"<< /Type /Outlines /First 6 0 R /Last 6 0 R /Count 1 >>".to_vec(),
        "Names" => b"<< /Dests << /Names [(catalog-only-marker) [3 0 R /Fit]] >> >>".to_vec(),
        "PageLabels" => b"<< /Nums [0 << /P (catalog-only-marker) /S /D >>] >>".to_vec(),
        "StructTreeRoot" => b"<< /Type /StructTreeRoot /K [] /ID (catalog-only-marker) >>".to_vec(),
        "AcroForm" => b"<< /Fields [6 0 R] >>".to_vec(),
        _ => b"<< /PrivateData (catalog-only-marker) >>".to_vec(),
    };
    let child = if key == "AcroForm" {
        b"<< /FT /Tx /T (catalog-only-marker) /V (retained-value) >>".to_vec()
    } else {
        b"<< /Title (catalog-only-marker) /Parent 5 0 R /Dest [3 0 R /Fit] >>".to_vec()
    };
    classic(
        &[
            format!("<< /Type /Catalog /Pages 2 0 R /{key} 5 0 R >>").into_bytes(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>".to_vec(),
            stream(b"q Q"),
            structure,
            child,
        ],
        "",
    )
}

#[test]
fn assembly_refuses_each_unpreserved_catalog_family_on_every_entrypoint() {
    let plain = open(&pdf("A", "B"));
    for key in [
        "Metadata",
        "Outlines",
        "Names",
        "PageLabels",
        "StructTreeRoot",
        "AcroForm",
        "VendorExtension",
    ] {
        let document = open(&catalog_structure_pdf(key));
        for result in [
            document.copy_pages(&[0]),
            document.extract_pages(&[0]),
            document.merge_pages(&[&plain]),
            document.insert_pages(0, &plain, &[0]),
            plain.merge_pages(&[&document]),
            plain.insert_pages(0, &document, &[0]),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.code, PdfErrorCode::UnsupportedFeature, "{key}");
            assert!(error.message.contains(key), "{error:?}");
        }
    }
}

#[test]
fn explicit_catalog_omissions_report_exact_input_location_and_key() {
    let plain = open(&pdf("A", "B"));
    for key in [
        "Metadata",
        "Outlines",
        "Names",
        "PageLabels",
        "StructTreeRoot",
        "AcroForm",
        "VendorExtension",
    ] {
        let input = catalog_structure_pdf(key);
        let original = input.clone();
        let document = open(&input);
        document.inspect().unwrap();
        let policy = PageAssemblyPolicy {
            remove_catalog_entries: vec![key.as_bytes().to_vec()],
            ..Default::default()
        };
        for (input_index, outcome) in [
            (0, document.copy_pages_with_policy(&[0], &policy).unwrap()),
            (
                0,
                document.extract_pages_with_policy(&[0], &policy).unwrap(),
            ),
            (
                1,
                plain
                    .merge_pages_with_policy(&[&document], &policy)
                    .unwrap(),
            ),
            (
                1,
                plain
                    .insert_pages_with_policy(0, &document, &[0], &policy)
                    .unwrap(),
            ),
        ] {
            assert_eq!(
                outcome.report.omitted_document_entries,
                vec![OmittedDocumentEntry {
                    input_index,
                    location: DocumentEntryLocation::Catalog,
                    key: key.as_bytes().to_vec(),
                }]
            );
            assert!(
                !outcome
                    .bytes
                    .windows(b"catalog-only-marker".len())
                    .any(|window| window == b"catalog-only-marker"),
                "{key}"
            );
            open(&outcome.bytes).validate().unwrap();
        }
        assert_eq!(input, original);
        assert_eq!(document.source_len(), original.len());
        // Neither observation nor a successful candidate changes the source document.
        assert_eq!(
            document.copy_pages(&[0]).unwrap_err().code,
            PdfErrorCode::UnsupportedFeature
        );
    }
}

#[test]
fn assembly_checks_unselected_inputs_and_empty_requests_before_omission() {
    let protected = open(&catalog_structure_pdf("Metadata"));
    let plain = open(&pdf("A", "B"));
    let empty = open(&classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R /Metadata 3 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [] /Count 0 >>".to_vec(),
            stream(b"catalog-only-marker"),
        ],
        "",
    ));
    for result in [
        protected.copy_pages(&[]),
        protected.extract_pages(&[]),
        protected.merge_pages(&[]),
        plain.insert_pages(0, &protected, &[]),
        plain.merge_pages(&[&empty]),
    ] {
        assert_eq!(result.unwrap_err().code, PdfErrorCode::UnsupportedFeature);
    }
    let policy = PageAssemblyPolicy {
        remove_catalog_entries: vec![b"Metadata".to_vec()],
        ..Default::default()
    };
    let outcome = plain
        .merge_pages_with_policy(&[&empty, &protected], &policy)
        .unwrap();
    assert_eq!(
        outcome
            .report
            .omitted_document_entries
            .iter()
            .map(|entry| entry.input_index)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn catalog_permission_does_not_authorize_trailer_or_other_catalog_entries() {
    let input = classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R /Metadata 4 0 R /VendorExtension 5 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_vec(),
            stream(b"catalog-only-marker"),
            b"<< /PrivateData (private) >>".to_vec(),
            b"<< /Title (document-title) >>".to_vec(),
        ],
        " /Info 6 0 R",
    );
    let document = open(&input);
    let mut policy = PageAssemblyPolicy {
        remove_catalog_entries: vec![b"Metadata".to_vec(), b"Info".to_vec()],
        ..Default::default()
    };
    assert_eq!(
        document
            .copy_pages_with_policy(&[0], &policy)
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
    policy
        .remove_catalog_entries
        .push(b"VendorExtension".to_vec());
    let error = document.copy_pages_with_policy(&[0], &policy).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    assert!(error.message.contains("Info"));
    policy.remove_trailer_entries.push(b"Info".to_vec());
    let outcome = document.copy_pages_with_policy(&[0], &policy).unwrap();
    assert!(
        outcome
            .report
            .omitted_document_entries
            .contains(&OmittedDocumentEntry {
                input_index: 0,
                location: DocumentEntryLocation::Trailer,
                key: b"Info".to_vec()
            })
    );
    assert!(!String::from_utf8_lossy(&outcome.bytes).contains("document-title"));
}

#[test]
fn classic_trailer_entries_need_exact_explicit_removal() {
    for (key, value) in [
        ("Info", "<< /Title (trailer-only-marker) >>"),
        ("ID", "[<01234567> <89ABCDEF>]"),
        ("VendorExtension", "(trailer-only-marker)"),
        ("W", "[1 2 1]"),
        ("Filter", "/FlateDecode"),
        ("Type", "/VendorTrailer"),
    ] {
        let document = open(&classic(
            &[
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_vec(),
            ],
            &format!(" /{key} {value}"),
        ));
        assert_eq!(
            document.copy_pages(&[0]).unwrap_err().code,
            PdfErrorCode::UnsupportedFeature,
            "{key}"
        );
        let policy = PageAssemblyPolicy {
            remove_trailer_entries: vec![key.as_bytes().to_vec()],
            ..Default::default()
        };
        let outcome = document.copy_pages_with_policy(&[0], &policy).unwrap();
        assert_eq!(
            outcome.report.omitted_document_entries,
            vec![OmittedDocumentEntry {
                input_index: 0,
                location: DocumentEntryLocation::Trailer,
                key: key.as_bytes().to_vec(),
            }]
        );
        assert!(!String::from_utf8_lossy(&outcome.bytes).contains("trailer-only-marker"));
    }
}

fn pdf(first: &str, second: &str) -> Vec<u8> {
    let first_content = format!("BT /F1 12 Tf ({first}) Tj ET");
    let second_content = format!("BT /F1 12 Tf ({second}) Tj ET");
    classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /Resources 7 0 R /MediaBox [0 0 612 792] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".to_vec(),
            stream(first_content.as_bytes()),
            stream(second_content.as_bytes()),
            b"<< /Font << /F1 8 0 R >> >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ],
        "",
    )
}

fn classic(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
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
            "trailer\n<< /Size {} /Root 1 0 R{trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn stream(bytes: &[u8]) -> Vec<u8> {
    [
        format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes(),
        bytes.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

fn annotation_pdf(annotations: &[&str]) -> Vec<u8> {
    let references = (0..annotations.len())
        .map(|index| format!("{} 0 R", index + 5))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Annots [{references}] /MediaBox [0 0 200 200] >>"
        )
        .into_bytes(),
        stream(b"BT /F1 12 Tf (A) Tj ET"),
    ];
    objects.extend(
        annotations
            .iter()
            .map(|annotation| annotation.as_bytes().to_vec()),
    );
    classic(&objects, "")
}

fn shared_annotation_pdf() -> Vec<u8> {
    classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Annots [5 0 R] /MediaBox [0 0 200 200] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Annots [5 0 R] /MediaBox [0 0 200 200] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] >>".to_vec(),
        ],
        "",
    )
}

fn unrelated_direct_annotation_pdf() -> Vec<u8> {
    classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Annots [5 0 R] /MediaBox [0 0 200 200] >>"
                .to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Annots [<< /Type /Annot /Subtype /Text /Rect [1 2 3 4] >>] /MediaBox [0 0 200 200] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] >>".to_vec(),
        ],
        "",
    )
}

fn unrelated_malformed_annotation_pdf() -> Vec<u8> {
    classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Annots [5 0 R] /MediaBox [0 0 200 200] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Annots 6 0 R /MediaBox [0 0 200 200] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Text /Rect [1 2 3 4] >>".to_vec(),
        ],
        "",
    )
}

fn indirect_link_action_pdf(action: &str) -> Vec<u8> {
    classic(
        &[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Annots [4 0 R] /MediaBox [0 0 200 200] >>".to_vec(),
            b"<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] /A 5 0 R >>".to_vec(),
            action.as_bytes().to_vec(),
        ],
        "",
    )
}

#[test]
fn extracts_duplicates_with_transitive_resources_and_deterministic_order() {
    let document = open(&pdf("A", "B"));
    let output = document.extract_pages(&[1, 0, 1]).unwrap();
    assert!(output.verification.passed);
    assert_eq!(output.report.output_pages, 3);
    let reopened = open(&output.bytes);
    assert_eq!(reopened.inspect().unwrap().page_count, 3);
    let a = reopened.query_text_all("A").unwrap();
    let b = reopened.query_text_all("B").unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 2);
    assert_ne!(b[0].object_number, a[0].object_number);
    assert_eq!(b[0].object_number, b[1].object_number);
    assert_eq!(b[0].generation, b[1].generation);
    assert_eq!(
        reopened
            .extract_text_spans()
            .unwrap()
            .spans
            .into_iter()
            .map(|span| (span.page_index, span.text))
            .collect::<Vec<_>>(),
        [(0, "B".into()), (1, "A".into()), (2, "B".into())]
    );
    assert_eq!(
        output.bytes,
        document.extract_pages(&[1, 0, 1]).unwrap().bytes
    );
}

#[test]
fn inserts_and_merges_colliding_source_object_numbers() {
    let left = open(&pdf("A", "B"));
    let right = open(&pdf("C", "D"));
    let inserted = left.insert_pages(1, &right, &[1]).unwrap();
    let inserted = open(&inserted.bytes);
    assert_eq!(inserted.inspect().unwrap().page_count, 3);
    let a = inserted.query_text("A", 0).unwrap().object_number;
    let d = inserted.query_text("D", 0).unwrap().object_number;
    let b = inserted.query_text("B", 0).unwrap().object_number;
    assert!(a < d && d < b);

    let merged = left.merge_pages(&[&right]).unwrap();
    assert!(merged.verification.no_dangling_references);
    let merged = open(&merged.bytes);
    assert_eq!(merged.inspect().unwrap().page_count, 4);
    for text in ["A", "B", "C", "D"] {
        assert_eq!(merged.query_text_all(text).unwrap().len(), 1);
    }
}

#[test]
fn applies_representable_page_transforms_and_preserves_text() {
    let transformed = open(&pdf("A", "B"))
        .transform_pages(
            &[0],
            PageTransform {
                rotation_degrees: Some(450),
                media_box: Some([0.0, 0.0, 300.0, 400.0]),
                crop_box: Some([10.0, 20.0, 290.0, 380.0]),
                translate: Some([12.0, 24.0]),
                scale: Some([2.0, 0.5]),
            },
        )
        .unwrap();
    assert!(transformed.verification.passed);
    assert!(
        transformed
            .bytes
            .windows(10)
            .any(|value| value == b"/Rotate 90")
    );
    assert!(
        transformed
            .bytes
            .windows(20)
            .any(|value| value == b"q 2 0 0 0.5 12 24 cm")
    );
    let reopened = open(&transformed.bytes);
    assert_eq!(reopened.query_text_all("A").unwrap().len(), 1);
    assert_eq!(reopened.query_text_all("B").unwrap().len(), 1);
}

#[test]
fn transforms_independent_link_and_markup_annotation_geometry() {
    let document = open(&annotation_pdf(&[
        "<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] /A << /S /URI /URI (https://example.test) >> >>",
        "<< /Type /Annot /Subtype /Highlight /Rect [50 60 90 80] /QuadPoints [50 80 90 80 50 60 90 60] >>",
    ]));
    let outcome = document
        .transform_pages(
            &[0],
            PageTransform {
                translate: Some([5.0, 7.0]),
                scale: Some([2.0, 3.0]),
                ..PageTransform::default()
            },
        )
        .unwrap();
    assert!(outcome.verification.passed);
    let annotations = list_annotations(&open(&outcome.bytes)).unwrap();
    assert_eq!(annotations[0].rect, [25.0, 67.0, 65.0, 127.0]);
    assert_eq!(annotations[1].rect, [105.0, 187.0, 185.0, 247.0]);
    let bytes = String::from_utf8_lossy(&outcome.bytes);
    assert!(bytes.contains("/QuadPoints [105.0 247.0 185.0 247.0 105.0 187.0 185.0 187.0]"));
    assert!(bytes.contains("q 2 0 0 3 5 7 cm"));
}

#[test]
fn refuses_link_destinations_and_resolves_indirect_actions() {
    for annotation in [
        "<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] /Dest [3 0 R /XYZ 10 20 null] >>",
        "<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] /A << /S /GoTo /D [3 0 R /XYZ 10 20 null] >> >>",
    ] {
        assert_eq!(
            open(&annotation_pdf(&[annotation]))
                .transform_pages(
                    &[0],
                    PageTransform {
                        translate: Some([1.0, 1.0]),
                        ..PageTransform::default()
                    },
                )
                .unwrap_err()
                .code,
            PdfErrorCode::UnsupportedFeature
        );
    }
    for action in [
        "<< /S /GoTo /D [3 0 R /XYZ 10 20 null] >>",
        "<< /S /GoToR /F (other.pdf) /D [0 /Fit] >>",
    ] {
        assert_eq!(
            open(&indirect_link_action_pdf(action))
                .transform_pages(
                    &[0],
                    PageTransform {
                        translate: Some([1.0, 1.0]),
                        ..PageTransform::default()
                    },
                )
                .unwrap_err()
                .code,
            PdfErrorCode::UnsupportedFeature
        );
    }
    assert!(
        open(&indirect_link_action_pdf(
            "<< /S /URI /URI (https://example.test) >>"
        ))
        .transform_pages(
            &[0],
            PageTransform {
                translate: Some([1.0, 1.0]),
                ..PageTransform::default()
            },
        )
        .unwrap()
        .verification
        .passed
    );
}

#[test]
fn ignores_unselected_direct_annotations_while_proving_selected_ownership() {
    let outcome = open(&unrelated_direct_annotation_pdf())
        .transform_pages(
            &[0],
            PageTransform {
                translate: Some([1.0, 1.0]),
                ..PageTransform::default()
            },
        )
        .unwrap();
    assert!(outcome.verification.passed);
    assert_eq!(
        list_annotations(&open(&outcome.bytes)).unwrap()[0].rect,
        [11.0, 21.0, 31.0, 41.0]
    );
    assert!(
        open(&unrelated_malformed_annotation_pdf())
            .transform_pages(
                &[0],
                PageTransform {
                    translate: Some([1.0, 1.0]),
                    ..PageTransform::default()
                },
            )
            .unwrap()
            .verification
            .passed
    );
}

#[test]
fn refuses_annotations_with_appearances_or_unsafe_geometry() {
    for annotation in [
        "<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] /AP << >> >>",
        "<< /Type /Annot /Subtype /Highlight /Rect [10 20 30 40] /QuadPoints [10 20] >>",
    ] {
        let error = open(&annotation_pdf(&[annotation]))
            .transform_pages(
                &[0],
                PageTransform {
                    translate: Some([1.0, 1.0]),
                    ..PageTransform::default()
                },
            )
            .unwrap_err();
        assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    }
    assert_eq!(
        open(&annotation_pdf(&[
            "<< /Type /Annot /Subtype /Link /Rect [10 20 30 40] >>"
        ]))
        .transform_pages(
            &[0],
            PageTransform {
                scale: Some([-1.0, 1.0]),
                ..PageTransform::default()
            },
        )
        .unwrap_err()
        .code,
        PdfErrorCode::UnsupportedFeature
    );
    assert_eq!(
        open(&shared_annotation_pdf())
            .transform_pages(
                &[0],
                PageTransform {
                    translate: Some([1.0, 1.0]),
                    ..PageTransform::default()
                },
            )
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn rejects_unsafe_transform_and_security_boundaries() {
    let document = open(&pdf("A", "B"));
    let error = document
        .transform_pages(
            &[0],
            PageTransform {
                rotation_degrees: Some(45),
                ..PageTransform::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);

    let mut signed_objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
    ];
    signed_objects.push(b"<< /Type /Sig /ByteRange [0 1 2 3] /Contents (x) >>".to_vec());
    let signed = open(&classic(&signed_objects, ""));
    assert_eq!(
        signed.extract_pages(&[0]).unwrap_err().code,
        PdfErrorCode::UnsafeRewrite
    );

    let encrypted_objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Filter /Standard >>".to_vec(),
    ];
    let encrypted = open(&classic(&encrypted_objects, " /Encrypt 4 0 R"));
    assert_eq!(
        encrypted.extract_pages(&[0]).unwrap_err().code,
        PdfErrorCode::UnsafeRewrite
    );
    let policy = PageAssemblyPolicy {
        remove_catalog_entries: vec![b"AcroForm".to_vec(), b"Perms".to_vec()],
        remove_trailer_entries: vec![b"Encrypt".to_vec()],
    };
    for document in [&signed, &encrypted] {
        assert_eq!(
            document
                .extract_pages_with_policy(&[0], &policy)
                .unwrap_err()
                .code,
            PdfErrorCode::UnsafeRewrite
        );
    }
}
