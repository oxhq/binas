// SPDX-License-Identifier: Apache-2.0
use std::{collections::BTreeSet, fs, path::Path};

use binas_pdf::{
    DocumentEntryLocation, OpenOptions, PageAssemblyPolicy, PdfEngine, PdfErrorCode, list_streams,
    read_decoded_stream, read_document_info, read_named_destinations, read_outlines,
    read_page_labels, read_xmp_metadata,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    fixtures: Vec<Fixture>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    path: String,
    sha256: String,
    origin: String,
    license: String,
    mandatory: bool,
    expected_effect: String,
    catalog_entries: Vec<String>,
    trailer_entries: Vec<String>,
}

#[test]
fn mandatory_preservation_corpus_integrity_and_expected_effects() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/preservation");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(root.join("manifest.json")).expect("mandatory preservation manifest missing"),
    )
    .unwrap();
    assert_eq!(manifest.schema_version, 1);
    // Freeze the mandatory set: removing a manifest entry cannot improve this gate.
    let expected = BTreeSet::from([
        "metadata.pdf",
        "outlines.pdf",
        "names.pdf",
        "labels.pdf",
        "unicode-alt-tags.pdf",
        "type3-shared.pdf",
        "type3-separate.pdf",
    ]);
    let mut exercised = BTreeSet::new();
    for fixture in &manifest.fixtures {
        assert!(expected.contains(fixture.path.as_str()));
        assert!(exercised.insert(fixture.path.as_str()), "duplicate fixture");
        assert!(fixture.mandatory);
        assert_eq!(fixture.origin, "original_generated");
        assert_eq!(fixture.license, "Apache-2.0");
        let input = fs::read(root.join(&fixture.path)).expect("mandatory PDF missing");
        assert_eq!(format!("{:x}", Sha256::digest(&input)), fixture.sha256);
        let engine = PdfEngine::default();
        let document = engine.open(&input, OpenOptions::default()).unwrap();
        let initial = document.inspect().unwrap();
        match fixture.expected_effect.as_str() {
            "refuse_unrequested_loss" => {
                assert_eq!(initial.page_count, 1);
                let error = document.extract_pages(&[0]).unwrap_err();
                assert_eq!(
                    error.code,
                    PdfErrorCode::UnsupportedFeature,
                    "{}: {error}",
                    fixture.path
                );
                assert!(error.message.contains("cannot preserve"));
                let policy = PageAssemblyPolicy {
                    remove_catalog_entries: fixture
                        .catalog_entries
                        .iter()
                        .map(|key| key.as_bytes().to_vec())
                        .collect(),
                    remove_trailer_entries: fixture
                        .trailer_entries
                        .iter()
                        .map(|key| key.as_bytes().to_vec())
                        .collect(),
                };
                let output = document.extract_pages_with_policy(&[0], &policy).unwrap();
                let actual: BTreeSet<_> = output
                    .report
                    .omitted_document_entries
                    .iter()
                    .map(|entry| {
                        assert_eq!(entry.input_index, 0);
                        (
                            match entry.location {
                                DocumentEntryLocation::Catalog => "catalog",
                                DocumentEntryLocation::Trailer => "trailer",
                            },
                            entry.key.clone(),
                        )
                    })
                    .collect();
                let expected: BTreeSet<_> = policy
                    .remove_catalog_entries
                    .iter()
                    .map(|key| ("catalog", key.clone()))
                    .chain(
                        policy
                            .remove_trailer_entries
                            .iter()
                            .map(|key| ("trailer", key.clone())),
                    )
                    .collect();
                assert_eq!(actual, expected);
                assert_eq!(actual.len(), output.report.omitted_document_entries.len());
                let reopened = engine.open(&output.bytes, OpenOptions::default()).unwrap();
                assert_eq!(reopened.inspect().unwrap().page_count, 1);
                match fixture.path.as_str() {
                    "metadata.pdf" => {
                        assert_eq!(
                            read_document_info(&document).unwrap().title.as_deref(),
                            Some("Original title")
                        );
                        assert!(read_xmp_metadata(&document).unwrap().is_some());
                        assert!(read_document_info(&reopened).unwrap().title.is_none());
                        assert!(read_xmp_metadata(&reopened).unwrap().is_none());
                    }
                    "outlines.pdf" => {
                        assert_eq!(
                            read_outlines(&document).unwrap()[0].title,
                            "Original bookmark"
                        );
                        assert!(read_outlines(&reopened).unwrap().is_empty());
                    }
                    "names.pdf" => {
                        assert_eq!(read_named_destinations(&document).unwrap().len(), 1);
                        assert!(read_named_destinations(&reopened).unwrap().is_empty());
                    }
                    "labels.pdf" => {
                        assert_eq!(read_page_labels(&document).unwrap()[0].label, "A-1");
                        assert_eq!(read_page_labels(&reopened).unwrap()[0].label, "1");
                    }
                    "unicode-alt-tags.pdf" => {}
                    other => panic!("unexpected loss-boundary fixture: {other}"),
                }
                // This proves a refusal boundary for Unicode-bearing tags, not a repaired tag serializer.
                if fixture.path == "unicode-alt-tags.pdf" {
                    let alt = "Imagen: café 中文 😀"
                        .encode_utf16()
                        .map(|unit| format!("{unit:04X}"))
                        .collect::<String>();
                    assert!(String::from_utf8_lossy(&input).contains(&format!("/Alt <FEFF{alt}>")));
                }
            }
            "extract_reversed_pages_preserves_text_and_glyph_stream" => {
                assert!(fixture.catalog_entries.is_empty() && fixture.trailer_entries.is_empty());
                assert_eq!(initial.page_count, 2);
                let before = document.extract_text_spans().unwrap();
                assert_eq!(
                    before
                        .spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<Vec<_>>(),
                    ["A", "AA"]
                );
                let output = document.extract_pages(&[1, 0]).unwrap();
                assert!(output.report.omitted_document_entries.is_empty());
                let reopened = engine.open(&output.bytes, OpenOptions::default()).unwrap();
                assert_eq!(reopened.inspect().unwrap().page_count, 2);
                assert_eq!(
                    reopened
                        .extract_text_spans()
                        .unwrap()
                        .spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<Vec<_>>(),
                    ["AA", "A"]
                );
                let glyph = b"600 0 0 0 600 700 d1 0 0 600 700 re f";
                for doc in [&document, &reopened] {
                    assert!(
                        list_streams(doc)
                            .unwrap()
                            .iter()
                            .any(|entry| read_decoded_stream(doc, entry.object).unwrap() == glyph)
                    );
                }
            }
            effect => panic!("unimplemented mandatory fixture expectation: {effect}"),
        }
        // Failed or successful read-based operations leave the source usable.
        assert_eq!(document.inspect().unwrap().page_count, initial.page_count);
    }
    assert_eq!(exercised, expected);
    let actual: BTreeSet<String> = fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".pdf"))
        .collect();
    assert_eq!(actual, expected.into_iter().map(str::to_owned).collect());
}
