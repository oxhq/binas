// SPDX-License-Identifier: Apache-2.0
//! Bounded local timing harness; correctness gates execute before sampling.
use binas_core::verification::CheckStatus;
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{EngineConfig, PdfSession, PdfWriterMode, SurgicalTextEditRequest, TextExtraction};
use serde_json::{Value, json};
use std::{error::Error, fs, hint::black_box, path::Path, time::Instant};

const READS: usize = 32;
const WARMUPS: usize = 3;
const SAMPLES: usize = 21;
const STEPS: [(&str, &str); 4] = [
    ("ALPHA", "OMEGA"),
    ("OMEGA", "SIGMA"),
    ("SIGMA", "DELTA"),
    ("DELTA", "OMEGA"),
];

fn open(input: Vec<u8>) -> PdfSession {
    let mut config = EngineConfig::default();
    config.limits.max_output_bytes = 1024 * 1024;
    PdfSession::open(
        input,
        config,
        OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 16 * 1024 * 1024,
                max_work_units: 256 * 1024 * 1024,
            }),
            CancellationToken::new(),
        ),
    )
    .expect("fixture must open")
}

fn request(old: &str, replacement: &str) -> SurgicalTextEditRequest {
    SurgicalTextEditRequest {
        old_text: old.into(),
        replacement: replacement.into(),
        match_index: 0,
    }
}

fn text(value: &TextExtraction) -> Vec<&str> {
    value.spans.iter().map(|span| span.text.as_str()).collect()
}

fn edit(session: &mut PdfSession, old: &str, new: &str, mode: PdfWriterMode) {
    let plan = session
        .prepare_text_edit(request(old, new), mode, 1)
        .unwrap();
    black_box(session.accept_text_edit(&plan).unwrap());
}

fn sequence(input: &[u8], reopen: bool) -> Vec<u8> {
    let mut session = open(input.to_vec());
    for (index, (old, new)) in STEPS.iter().enumerate() {
        edit(&mut session, old, new, PdfWriterMode::Compact);
        if reopen && index + 1 < STEPS.len() {
            let output = session.export(PdfWriterMode::Compact).unwrap();
            session.close();
            session = open(output.output.as_bytes().to_vec());
        }
    }
    session
        .export(PdfWriterMode::Compact)
        .unwrap()
        .output
        .as_bytes()
        .to_vec()
}

fn correctness(input: &[u8], fonted: bool, name: &str, evidence: &Path) -> Value {
    let mut session = open(input.to_vec());
    let before = session.snapshot().unwrap();
    let extracted = session.extract_page_text_spans(0).unwrap();
    assert_eq!(text(&extracted), ["ALPHA", "KEEP"]);
    if fonted {
        assert!(extracted.warnings.is_empty());
        assert_eq!(
            session
                .prepare_text_edit(request("ALPHA", "OMEGA"), PdfWriterMode::Compact, 1)
                .unwrap_err()
                .code(),
            "unsupported_feature"
        );
        assert_eq!(session.snapshot().unwrap(), before);
        assert_eq!(
            text(&session.extract_page_text_spans(0).unwrap()),
            ["ALPHA", "KEEP"]
        );
    } else {
        for (label, mode) in [
            ("incremental", PdfWriterMode::Incremental),
            ("compact", PdfWriterMode::Compact),
        ] {
            let mut retained = open(input.to_vec());
            let plan = retained
                .prepare_text_edit(request("ALPHA", "OMEGA"), mode, 1)
                .unwrap();
            assert_eq!(
                text(&retained.extract_page_text_spans(0).unwrap()),
                ["ALPHA", "KEEP"]
            );
            let receipt = retained.accept_text_edit(&plan).unwrap();
            assert_eq!(receipt.changed_objects().len(), 1);
            assert_eq!(receipt.changed_objects()[0].number, 4);
            assert_eq!(
                retained.accept_text_edit(&plan).unwrap_err().code(),
                "stale_plan"
            );
            let output = retained.export(mode).unwrap();
            assert_eq!(output.evidence.checks().len(), 7);
            assert!(
                output
                    .evidence
                    .checks()
                    .iter()
                    .all(|check| check.status == CheckStatus::Passed)
            );
            let mut independent_session = open(input.to_vec());
            edit(&mut independent_session, "ALPHA", "OMEGA", mode);
            let repeated = independent_session.export(mode).unwrap();
            assert_eq!(output.output.as_bytes(), repeated.output.as_bytes());
            let reopened = open(output.output.as_bytes().to_vec());
            assert_eq!(
                text(&reopened.extract_page_text_spans(0).unwrap()),
                ["OMEGA", "KEEP"]
            );
            fs::write(
                evidence.join(format!("{name}-{label}.pdf")),
                output.output.as_bytes(),
            )
            .unwrap();
        }
        let retained = sequence(input, false);
        let reopened = sequence(input, true);
        assert_eq!(retained, reopened);
        let checked = open(retained.clone());
        assert_eq!(
            text(&checked.extract_page_text_spans(0).unwrap()),
            ["OMEGA", "KEEP"]
        );
        fs::write(
            evidence.join(format!("{name}-multi-retained-compact.pdf")),
            retained,
        )
        .unwrap();
        fs::write(
            evidence.join(format!("{name}-multi-reopen-compact.pdf")),
            reopened,
        )
        .unwrap();
    }
    session.close();
    assert_eq!(
        session.extract_page_text_spans(0).unwrap_err().code(),
        "session_closed"
    );
    json!({"fixture": name, "fonted": fonted, "selected_text": ["ALPHA", "KEEP"],
        "warning_count": extracted.warnings.len(), "fonted_raw_edit_refused": fonted,
        "raw_single_output_byte_parity": !fonted, "raw_multi_compact_byte_parity": !fonted,
        "closed_refusal": true})
}

fn ns(start: Instant) -> u64 {
    start.elapsed().as_nanos().try_into().unwrap()
}

fn reads(input: &[u8], reopen: bool, count: usize) {
    if reopen {
        for _ in 0..count {
            let session = open(input.to_vec());
            black_box(session.extract_page_text_spans(0).unwrap());
        }
    } else {
        let session = open(input.to_vec());
        // First decode is part of this batch; isolated warm reads are below.
        for _ in 0..count {
            black_box(session.extract_page_text_spans(0).unwrap());
        }
    }
}

fn sample(input: &[u8], fonted: bool) -> Value {
    let owned = input.to_vec();
    let start = Instant::now();
    let session = open(owned);
    let open_ns = ns(start);
    let start = Instant::now();
    black_box(session.extract_page_text_spans(0).unwrap());
    let first_read_ns = ns(start);
    let start = Instant::now();
    for _ in 0..READS {
        black_box(session.extract_page_text_spans(0).unwrap());
    }
    let warm_read_batch_ns = ns(start);
    let start = Instant::now();
    reads(input, true, READS);
    let reopen_read_batch_ns = ns(start);
    let mut phases = json!({"open_ns": open_ns, "first_read_ns": first_read_ns,
        "warm_read_batch_ns": warm_read_batch_ns, "reopen_read_batch_ns": reopen_read_batch_ns});
    if !fonted {
        for (label, mode) in [
            ("incremental", PdfWriterMode::Incremental),
            ("compact", PdfWriterMode::Compact),
        ] {
            let mut session = open(input.to_vec());
            let start = Instant::now();
            let plan = session
                .prepare_text_edit(request("ALPHA", "OMEGA"), mode, 1)
                .unwrap();
            let prepare = ns(start);
            let start = Instant::now();
            black_box(session.accept_text_edit(&plan).unwrap());
            let accept = ns(start);
            let start = Instant::now();
            black_box(session.export(mode).unwrap());
            let export = ns(start);
            phases[format!("{label}_prepare_checked_ns")] = json!(prepare);
            phases[format!("{label}_accept_validation_ns")] = json!(accept);
            phases[format!("{label}_export_checked_ns")] = json!(export);
        }
    }
    phases
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(args.get(1).ok_or("usage: session_measure REPO EVIDENCE")?);
    let evidence = Path::new(args.get(2).ok_or("missing EVIDENCE")?);
    fs::create_dir_all(evidence)?;
    let mut fixtures = Vec::new();
    let mut checks = Vec::new();
    for (directory, stem, fonted) in [
        ("retained-ascii", "retained-ascii", false),
        ("fonted-text", "fonted-text", true),
    ] {
        for shape in ["plain", "flate"] {
            let name = format!("{stem}-{shape}");
            let relative = format!("crates/binas-pdf/tests/fixtures/{directory}/{name}.pdf");
            let input = fs::read(root.join(&relative))?;
            checks.push(correctness(&input, fonted, &name, evidence));
            fixtures.push((name, relative, input, fonted));
        }
    }
    fs::write(
        evidence.join("correctness.json"),
        serde_json::to_vec_pretty(&checks)?,
    )?;
    let mut results = Vec::new();
    for (name, path, input, fonted) in &fixtures {
        for _ in 0..WARMUPS {
            black_box(sample(input, *fonted));
        }
        let mut samples = Vec::new();
        for index in 0..SAMPLES {
            samples.push(json!({"index": index, "phases": sample(input, *fonted)}));
        }
        results.push(
            json!({"fixture": name, "source_path": path, "input_bytes": input.len(),
            "fonted": fonted, "samples": samples}),
        );
    }
    let input = &fixtures[1].2;
    let mut batches = Vec::new();
    for index in 0..WARMUPS + SAMPLES {
        let mut row = json!({"index": index.saturating_sub(WARMUPS)});
        // Reverse order on alternating samples. Two distinct contexts and owners.
        for reopen in if index % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let label = if reopen { "reopen" } else { "retained" };
            let start = Instant::now();
            for _ in 0..2 {
                reads(input, reopen, READS);
            }
            row[format!("serial_two_{label}_ns")] = json!(ns(start));
            let start = Instant::now();
            std::thread::scope(|scope| {
                let workers: Vec<_> = (0..2)
                    .map(|_| scope.spawn(|| reads(input, reopen, READS)))
                    .collect();
                for worker in workers {
                    worker.join().unwrap();
                }
            });
            row[format!("parallel_two_{label}_ns")] = json!(ns(start));
            let start = Instant::now();
            black_box(sequence(input, reopen));
            row[format!("multi_four_{label}_ns")] = json!(ns(start));
        }
        if index >= WARMUPS {
            batches.push(row);
        }
    }
    let report = json!({"schema": 1, "engine_version": env!("CARGO_PKG_VERSION"),
        "warmups": WARMUPS, "samples_per_case": SAMPLES, "reads_per_batch": READS,
        "clock": "std::time::Instant elapsed nanoseconds; wall clock",
        "memory": "not measured; logical budget counters are not heap/process memory",
        "correctness": checks, "cases": results, "raw_flate_batch_samples": batches});
    fs::write(
        evidence.join("samples.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "correctness passed; {} fixtures; {SAMPLES} samples; evidence {}",
        fixtures.len(),
        evidence.display()
    );
    Ok(())
}
