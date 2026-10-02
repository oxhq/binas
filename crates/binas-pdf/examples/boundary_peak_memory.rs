// SPDX-License-Identifier: Apache-2.0
//! Public-API memory workload. The parent measures Windows process counters.
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    EngineConfig, OpenOptions, PdfEngine, PdfSession, PdfWriterMode, SurgicalTextEditRequest,
};
use serde_json::json;
use std::{error::Error, fs, io, path::Path};

fn sync() {
    let mut line = String::new();
    io::stdin().read_line(&mut line).unwrap();
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let case = &args[1];
    println!("{}", json!({"ready": true}));
    sync();
    let input = fs::read(&args[2])?;
    if case == "page-import" {
        let engine = PdfEngine::default();
        let document = engine.open_owned(input, OpenOptions::default())?;
        assert_eq!(document.inspect()?.page_count, 2);
        let candidate = document.extract_pages(&[1, 0, 1])?;
        assert!(candidate.verification.passed);
        assert_eq!(document.inspect()?.page_count, 2);
        assert_eq!(
            engine
                .open(&candidate.bytes, OpenOptions::default())?
                .inspect()?
                .page_count,
            3
        );
        fs::write(Path::new(&args[3]), &candidate.bytes)?;
        println!(
            "{}",
            json!({"done": true, "case": case, "error": null, "original_pages": 2, "candidate_pages": 3, "output_bytes": candidate.bytes.len()})
        );
        sync();
        return Ok(());
    }
    let mut checkpoints = Vec::new();
    let mut outputs = Vec::new();
    let mut open_count = 0;
    let mut input_move_bytes = 0;
    let mut output_copy_bytes = 0;
    let mut open = |bytes: Vec<u8>| {
        open_count += 1;
        input_move_bytes += bytes.len();
        let mut config = EngineConfig::default();
        config.limits.max_input_bytes = 8 * 1024 * 1024;
        config.limits.max_stream_bytes = 16 * 1024 * 1024;
        config.limits.max_total_decoded_bytes = 16 * 1024 * 1024;
        config.limits.max_decode_live_bytes = 16 * 1024 * 1024;
        config.limits.max_output_bytes = 16 * 1024 * 1024;
        PdfSession::open(
            bytes,
            config,
            OperationContext::new(
                ResourceBudget::new(BudgetLimits {
                    max_live_bytes: if case == "live-refusal" {
                        1
                    } else if case.starts_with("output") {
                        48 * 1024 * 1024
                    } else {
                        24 * 1024 * 1024
                    },
                    max_work_units: 256 * 1024 * 1024,
                }),
                CancellationToken::new(),
            ),
        )
    };
    let mut observed_error: Option<&str> = None;
    let mut error_message = None;
    match open(input.clone()) {
        Err(error) => {
            observed_error = Some(error.code());
        }
        Ok(mut session) => {
            let budget = session.context().budget().snapshot();
            checkpoints.push(json!({"phase": "open", "live_bytes": budget.live_bytes, "work_units": budget.work_units}));
            if case == "cancel-refusal" {
                session.context().cancellation().cancel();
            }
            if case.starts_with("output") {
                let refused = case == "output-refusal";
                let replacement = "A".repeat(if refused {
                    8 * 1024 * 1024 - 4
                } else {
                    1024 * 1024
                });
                let mode = if refused {
                    PdfWriterMode::Incremental
                } else {
                    PdfWriterMode::Compact
                };
                let before = session.snapshot()?;
                match session.prepare_text_edit(
                    SurgicalTextEditRequest {
                        old_text: "ALPHA".into(),
                        replacement,
                        match_index: 0,
                    },
                    mode,
                    1,
                ) {
                    Err(error) => {
                        observed_error = Some(error.code());
                        error_message = Some(error.to_string());
                        assert_eq!(session.snapshot()?, before);
                    }
                    Ok(plan) => {
                        assert!(!refused, "output limit should refuse preparation");
                        session.accept_text_edit(&plan)?;
                        let exported = session.export(mode)?;
                        outputs.push(exported.output.as_bytes().len());
                        fs::write(Path::new(&args[3]), exported.output.as_bytes())?;
                        assert_eq!(
                            session.extract_page_text_spans(0)?.spans[0].text.len(),
                            1024 * 1024
                        );
                    }
                }
            } else if case.ends_with("refusal") {
                let error = if case == "font-refusal" {
                    session
                        .prepare_text_edit(
                            SurgicalTextEditRequest {
                                old_text: "ALPHA".into(),
                                replacement: "OMEGA".into(),
                                match_index: 0,
                            },
                            PdfWriterMode::Compact,
                            1,
                        )
                        .unwrap_err()
                } else {
                    session.extract_page_text_spans(0).unwrap_err()
                };
                observed_error = Some(error.code());
                assert_eq!(session.snapshot()?.revision().value(), 0);
            } else if case.starts_with("edit") {
                for (index, (old, new)) in [
                    ("ALPHA", "OMEGA"),
                    ("OMEGA", "SIGMA"),
                    ("SIGMA", "DELTA"),
                    ("DELTA", "OMEGA"),
                ]
                .iter()
                .enumerate()
                {
                    let plan = session.prepare_text_edit(
                        SurgicalTextEditRequest {
                            old_text: (*old).into(),
                            replacement: (*new).into(),
                            match_index: 0,
                        },
                        PdfWriterMode::Compact,
                        1,
                    )?;
                    session.accept_text_edit(&plan)?;
                    if case == "edit-reopen" && index < 3 {
                        let exported = session.export(PdfWriterMode::Compact)?;
                        let bytes = exported.output.as_bytes().to_vec();
                        output_copy_bytes += bytes.len();
                        session.close();
                        session = open(bytes)?;
                    }
                }
                let exported = session.export(PdfWriterMode::Compact)?;
                let bytes = exported.output.as_bytes();
                fs::write(Path::new(&args[3]), bytes)?;
                outputs.push(bytes.len());
                assert_eq!(session.extract_page_text_spans(0)?.spans[0].text, "OMEGA");
            } else {
                for index in 0..if case == "read-once" { 1 } else { 32 } {
                    let extraction = session.extract_page_text_spans(0)?;
                    assert_eq!(extraction.spans[0].text, "ALPHA");
                    if case == "read-reopen" && index < 31 {
                        session.close();
                        session = open(input.clone())?;
                    }
                }
            }
            session.close();
            let budget = session.context().budget().snapshot();
            assert_eq!(budget.live_bytes, 0);
            checkpoints.push(json!({"phase": "close", "live_bytes": budget.live_bytes, "work_units": budget.work_units}));
        }
    }
    let expected = match case.as_str() {
        "input-refusal" | "decode-refusal" | "live-refusal" | "output-refusal" => {
            Some("resource_limit")
        }
        "invalid-refusal" => Some("invalid_syntax"),
        "font-refusal" => Some("unsupported_feature"),
        "cancel-refusal" => Some("cancelled"),
        _ => None,
    };
    assert_eq!(observed_error, expected);
    if case == "output-refusal" {
        assert!(
            error_message
                .as_deref()
                .unwrap()
                .contains("output exceeds max_output_bytes")
        );
    }
    println!(
        "{}",
        json!({"done": true, "case": case, "input_bytes": input.len(), "error": observed_error,
        "open_count": open_count, "caller_input_move_bytes": input_move_bytes, "caller_output_copy_bytes": output_copy_bytes,
        "output_sizes": outputs, "checkpoints": checkpoints, "error_message": error_message})
    );
    sync();
    Ok(())
}
