// SPDX-License-Identifier: Apache-2.0
use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfEngine, PdfSession, PdfWriterMode,
    SurgicalTextEditRequest,
};

mod fixture;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_dir = std::path::PathBuf::from(std::env::args().nth(1).expect("output directory"));
    let input = fixture::pdf();
    let document = PdfEngine::default().open(&input, OpenOptions::default())?;
    assert_eq!(document.inspect()?.page_count, 1);
    assert_eq!(document.query_text("ALPHA", 0)?.text, "ALPHA");
    assert_eq!(document.extract_page_text_spans(0)?.spans[0].text, "ALPHA");
    for (name, mode) in [
        ("incremental", PdfWriterMode::Incremental),
        ("compact", PdfWriterMode::Compact),
    ] {
        let ctx = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: 1_000_000,
                max_work_units: 100_000_000,
            }),
            CancellationToken::new(),
        );
        let mut session = PdfSession::open(
            input.clone(),
            EngineConfig {
                limits: Limits {
                    max_output_bytes: 65_536,
                    ..Limits::default()
                },
            },
            ctx.clone(),
        )?;
        let before = session.snapshot()?;
        let plan = session.prepare_text_edit(
            SurgicalTextEditRequest {
                old_text: "ALPHA".into(),
                replacement: "OMEGA".into(),
                match_index: 0,
            },
            mode,
            1,
        )?;
        assert_eq!(session.snapshot()?, before);
        assert_eq!(session.extract_page_text_spans(0)?.spans[0].text, "ALPHA");
        let receipt = session.accept_text_edit(&plan)?;
        assert_eq!(receipt.before(), before);
        assert_eq!(receipt.after().revision().value(), 1);
        assert_eq!(
            session.accept_text_edit(&plan).unwrap_err().code(),
            "stale_plan"
        );
        let exported = session.export(mode)?;
        assert_eq!(exported.snapshot, receipt.after());
        assert_eq!(session.extract_page_text_spans(0)?.spans[0].text, "OMEGA");
        let reopened =
            PdfEngine::default().open(exported.output.as_bytes(), OpenOptions::default())?;
        assert_eq!(reopened.query_text("OMEGA", 0)?.text, "OMEGA");
        std::fs::write(
            output_dir.join(format!("consumer-{name}.pdf")),
            exported.output.as_bytes(),
        )?;
        session.close();
        assert_eq!(ctx.budget().snapshot().live_bytes, 0);
        assert!(ctx.budget().snapshot().work_units > 0);
        assert_eq!(session.snapshot().unwrap_err().code(), "session_closed");
        // Returned export owns its source independently of the closed session.
        assert_eq!(reopened.extract_text_spans()?.spans[0].text, "OMEGA");
    }
    println!("artifact-only selected/session/legacy consumption passed");
    Ok(())
}
