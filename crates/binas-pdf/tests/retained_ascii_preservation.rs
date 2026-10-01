//! P3a public operation harness. Independent evidence is produced separately by
//! qa/retained-ascii/oracle.py; no suite oracle claim is added to runtime receipts.
use binas_core::verification::{CheckCode, CheckStatus};
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    OpenOptions, PdfEngine, PdfErrorCode, PdfRevision, PdfTextEditError, PdfWriterMode,
    SurgicalTextEditRequest,
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

fn revision(bytes: &[u8]) -> PdfRevision {
    PdfRevision::new(
        PdfEngine::default()
            .open_owned(bytes.to_vec(), OpenOptions::default())
            .unwrap(),
    )
}

fn request() -> SurgicalTextEditRequest {
    SurgicalTextEditRequest {
        old_text: "ALPHA".into(),
        replacement: "OMEGA".into(),
        match_index: 0,
    }
}

#[test]
fn public_preparation_acceptance_exports_both_storage_shapes_and_modes() {
    for (shape, input) in [
        (
            "plain",
            include_bytes!("fixtures/retained-ascii/retained-ascii-plain.pdf").as_slice(),
        ),
        (
            "flate",
            include_bytes!("fixtures/retained-ascii/retained-ascii-flate.pdf").as_slice(),
        ),
    ] {
        for (name, mode) in [
            ("incremental", PdfWriterMode::Incremental),
            ("compact", PdfWriterMode::Compact),
        ] {
            let mut live = revision(input);
            let prior = live.clone();
            let before = live.snapshot();
            let original_text = live.text().unwrap();
            assert_eq!(
                original_text,
                ["ALPHA", "KEEP", "UNTOUCHED PAGE ONE", "UNTOUCHED PAGE TWO"]
            );
            let prepared = live
                .prepare_text_edit(request(), mode, 1, &context())
                .unwrap();
            let repeated = live
                .prepare_text_edit(request(), mode, 1, &context())
                .unwrap();
            assert_eq!(prepared.output().as_bytes(), repeated.output().as_bytes());
            assert_eq!(live.snapshot(), before);
            assert_eq!(live.text().unwrap(), original_text);
            let output = prepared.output().clone();
            let required = prepared.capability().required_checks().clone();
            let receipt = live
                .accept_text_edit(prepared, &request(), 1, &required, &context())
                .unwrap();
            assert_eq!(receipt.before(), before);
            assert_eq!(receipt.after(), live.snapshot());
            assert_eq!(receipt.output_id(), output.id());
            assert_eq!(receipt.output_len(), output.len());
            assert_eq!(receipt.changed_objects().len(), 1);
            assert_eq!(receipt.changed_objects()[0].number, 4);
            assert_eq!(receipt.changed_objects()[0].generation, 0);
            assert_eq!(receipt.evidence().checks().len(), 21);
            for scope in ["appearance", "accessibility", "opaque_semantics"] {
                let code = CheckCode::new("pdf.scope", scope).unwrap();
                assert_eq!(
                    receipt
                        .evidence()
                        .checks()
                        .iter()
                        .find(|check| check.code == code)
                        .unwrap()
                        .status,
                    CheckStatus::NotChecked
                );
            }
            let mut expected_text = original_text.clone();
            let selected = expected_text
                .iter_mut()
                .find(|text| *text == "ALPHA")
                .unwrap();
            *selected = "OMEGA".into();
            assert_eq!(live.text().unwrap(), expected_text);
            assert_eq!(prior.snapshot(), before);
            assert_eq!(prior.text().unwrap(), original_text);
            let reopened = revision(output.as_bytes());
            assert_eq!(reopened.text().unwrap(), expected_text);
            if mode == PdfWriterMode::Incremental {
                assert!(output.as_bytes().starts_with(input));
                // This is preservation of history, not secure removal/redaction.
                if shape == "plain" {
                    assert!(
                        output
                            .as_bytes()
                            .windows(7)
                            .any(|bytes| bytes == b"(ALPHA)")
                    );
                }
            }
            if let Some(directory) = std::env::var_os("BINAS_P3A_OUTPUT_DIR") {
                let directory = Path::new(&directory);
                std::fs::create_dir_all(directory).unwrap();
                let stem = format!("{shape}-{name}");
                std::fs::write(directory.join(format!("{stem}.pdf")), output.as_bytes()).unwrap();
                let facts = serde_json::json!({
                    "evidence_kind": "runtime_same_engine_receipt_observation",
                    "external_oracle_executed_by_receipt": false,
                    "operation": receipt.operation(), "contract": receipt.contract_version(),
                    "engine_version": receipt.engine_version(), "mode": name,
                    "input_id": format!("{:?}", receipt.input_id()), "input_len": receipt.input_len(),
                    "output_id": format!("{:?}", receipt.output_id()), "output_len": receipt.output_len(),
                    "before": format!("{:?}", receipt.before()), "after": format!("{:?}", receipt.after()),
                    "request": {"old_text": "ALPHA", "replacement": "OMEGA", "match_index": 0},
                    "changed_objects": receipt.changed_objects().iter().map(|id| serde_json::json!({"number": id.number, "generation": id.generation})).collect::<Vec<_>>(),
                    "checks": receipt.evidence().checks().iter().map(|check| serde_json::json!({"namespace": check.code.namespace(), "code": check.code.code(), "status": format!("{:?}", check.status)})).collect::<Vec<_>>()
                });
                std::fs::write(
                    directory.join(format!("{stem}-receipt.json")),
                    serde_json::to_vec_pretty(&facts).unwrap(),
                )
                .unwrap();
            }
        }
    }
}

#[test]
fn public_font_interpretation_shared_ownership_and_signature_marker_refuse_atomically() {
    for (input, code) in [
        (
            include_bytes!("fixtures/retained-ascii/fonted-refusal.pdf").as_slice(),
            PdfErrorCode::UnsupportedFeature,
        ),
        (
            include_bytes!("fixtures/retained-ascii/shared-owner.pdf").as_slice(),
            PdfErrorCode::UnsafeRewrite,
        ),
        (
            include_bytes!("fixtures/retained-ascii/signature-marker.pdf").as_slice(),
            PdfErrorCode::UnsafeRewrite,
        ),
        (
            include_bytes!("fixtures/retained-ascii/repeated-owner.pdf").as_slice(),
            PdfErrorCode::UnsafeRewrite,
        ),
        (
            include_bytes!("fixtures/retained-ascii/foreign-owner.pdf").as_slice(),
            PdfErrorCode::UnsafeRewrite,
        ),
        (
            include_bytes!("fixtures/retained-ascii/indirect-length.pdf").as_slice(),
            PdfErrorCode::UnsupportedFeature,
        ),
        (
            include_bytes!("fixtures/retained-ascii/unsupported-gs.pdf").as_slice(),
            PdfErrorCode::UnsupportedFeature,
        ),
    ] {
        for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
            let live = revision(input);
            let before = live.snapshot();
            let original = live.text().unwrap();
            let error = live
                .prepare_text_edit(request(), mode, 1, &context())
                .unwrap_err();
            assert!(
                matches!(error, PdfTextEditError::Pdf(ref detail) if detail.code == code),
                "unexpected refusal: {error:?}"
            );
            assert_eq!(live.snapshot(), before);
            assert_eq!(live.text().unwrap(), original);
        }
    }
}

#[test]
fn genuinely_encrypted_fixture_opens_structurally_but_refuses_preparation() {
    // Independent pypdf generation/decryption is recorded in fixture provenance.
    // Opening inspects encrypted containers; it does not imply decryption.
    let live = revision(include_bytes!(
        "fixtures/retained-ascii/encrypted-fonted.pdf"
    ));
    let before = live.snapshot();
    for mode in [PdfWriterMode::Incremental, PdfWriterMode::Compact] {
        let error = live
            .prepare_text_edit(request(), mode, 1, &context())
            .unwrap_err();
        assert!(
            matches!(error, PdfTextEditError::Pdf(ref detail) if detail.code == PdfErrorCode::UnsafeRewrite),
            "unexpected encryption refusal: {error:?}"
        );
        assert_eq!(live.snapshot(), before);
    }
}
