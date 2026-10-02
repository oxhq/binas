//! Provisioned original producer corpus. Independent oracle runs separately.
use std::{env, fs, path::PathBuf};

use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    EngineConfig, OpenOptions, PdfEngine, PdfPageTextEditRequest, PdfRevision, PdfRevisionRequest,
    PdfSession, PdfSharedEditMode, PdfWriterMode, SurgicalTextEditRequest,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Manifest {
    corpus_root: String,
    fixtures: Vec<Fixture>,
}
#[derive(Deserialize)]
struct Fixture {
    id: String,
    path: String,
    sha256: String,
    kind: String,
    target: Option<[u32; 2]>,
    expected_error_code_edit: Option<String>,
}
fn context() -> OperationContext {
    OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 128 * 1024 * 1024,
            max_work_units: 128 * 1024 * 1024,
        }),
        CancellationToken::new(),
    )
}
fn config() -> EngineConfig {
    let mut config = EngineConfig::default();
    config.limits.max_input_bytes = 8 * 1024 * 1024;
    config.limits.max_output_bytes = 16 * 1024 * 1024;
    config
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
fn held_out_producers_have_declared_effects_and_atomic_refusals() {
    let Some(manifest_path) = env::var_os("BINAS_P5_CORPUS_MANIFEST") else {
        assert!(env::var_os("BINAS_REQUIRE_P5_CORPUS").is_none());
        return;
    };
    let manifest_path = PathBuf::from(manifest_path);
    let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    let root = manifest_path.parent().unwrap().join(manifest.corpus_root);
    let outputs = env::var_os("BINAS_P5_OUTPUT_DIR").map(PathBuf::from);
    if let Some(outputs) = &outputs {
        fs::create_dir_all(outputs).unwrap();
    }
    for fixture in manifest.fixtures {
        let input = fs::read(root.join(&fixture.path)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&input)), fixture.sha256);
        if fixture.kind == "parse_reject" {
            continue; // Required external_compatibility_corpus owns parse refusal.
        }
        let document = PdfEngine::default()
            .open_owned(input.clone(), OpenOptions::default())
            .unwrap();
        let mut revision = PdfRevision::new(document);
        let before = revision.snapshot();
        let original = revision.text().unwrap();
        assert!(
            original.iter().any(|text| text == "ALPHA"),
            "{}",
            fixture.id
        );
        let target = fixture.target.unwrap();
        if fixture.kind.starts_with("refusal_") {
            let error = revision.plan(request(), &context()).expect_err(&fixture.id);
            assert_eq!(
                error.code.as_str(),
                fixture.expected_error_code_edit.unwrap()
            );
            assert_eq!(revision.snapshot(), before);
            assert_eq!(revision.text().unwrap(), original);
        } else if fixture.kind == "fonted" {
            let plan = revision.plan(request(), &context()).unwrap();
            assert_eq!(revision.snapshot(), before);
            let committed = revision.commit(&plan, &context()).unwrap();
            assert_eq!(committed.changes.len(), 1);
            assert_eq!(committed.changes[0].number, target[0]);
            assert_eq!(u32::from(committed.changes[0].generation), target[1]);
            committed.evidence.validate(&committed.required).unwrap();
            let mut expected = original.clone();
            *expected.iter_mut().find(|text| *text == "ALPHA").unwrap() = "OMEGA-LONG".into();
            assert_eq!(revision.text().unwrap(), expected);
            for (name, mode) in [
                ("incremental", PdfWriterMode::Incremental),
                ("compact", PdfWriterMode::Compact),
            ] {
                let exported = revision.export(mode, &context()).unwrap();
                if let Some(outputs) = &outputs {
                    fs::write(
                        outputs.join(format!("{}-{name}.pdf", fixture.id)),
                        exported.output.as_bytes(),
                    )
                    .unwrap();
                }
            }
            // Retained public sessions deliberately stay raw; no fonted fallback.
            let mut session = PdfSession::open(input, config(), context()).unwrap();
            let snapshot = session.snapshot().unwrap();
            let error = session
                .prepare_text_edit(edit(), PdfWriterMode::Compact, 1)
                .unwrap_err();
            assert_eq!(error.code(), "unsupported_feature");
            assert_eq!(session.snapshot().unwrap(), snapshot);
        } else {
            assert_eq!(fixture.kind, "raw");
            for (name, mode) in [
                ("incremental", PdfWriterMode::Incremental),
                ("compact", PdfWriterMode::Compact),
            ] {
                let mut session = PdfSession::open(input.clone(), config(), context()).unwrap();
                let snapshot = session.snapshot().unwrap();
                let plan = session.prepare_text_edit(edit(), mode, 1).unwrap();
                assert_eq!(session.snapshot().unwrap(), snapshot);
                let receipt = session.accept_text_edit(&plan).unwrap();
                assert_eq!(receipt.changed_objects().len(), 1);
                assert_eq!(receipt.changed_objects()[0].number, target[0]);
                let text = session.extract_page_text_spans(0).unwrap();
                assert_eq!(
                    text.spans
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<Vec<_>>(),
                    ["OMEGA-LONG", "KEEP"]
                );
                let exported = session.export(mode).unwrap();
                if let Some(outputs) = &outputs {
                    fs::write(
                        outputs.join(format!("{}-{name}.pdf", fixture.id)),
                        exported.output.as_bytes(),
                    )
                    .unwrap();
                }
            }
        }
    }
}
