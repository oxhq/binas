//! Synchronous owned sessions for the conditional raw-token contract v1.
use binas_core::{
    BudgetLimits, CancellationToken, OperationContext, ResourceBudget, verification::CheckStatus,
};
use binas_pdf::{
    EngineConfig, Limits, PdfSession, PdfSessionError, PdfSessionPlanHandle, PdfWriterMode,
    SurgicalTextEditRequest,
};
use serde_json::json;
use wasm_bindgen::prelude::*;

/// JS must call close() to release payloads deterministically, then free() to
/// release the wrapper. Cancellation cannot interrupt a blocked JS event loop.
#[wasm_bindgen(js_name = BinasPdfSession)]
pub struct WasmPdfSession {
    inner: PdfSession,
}

/// Opaque capability owned by one session; it retains no document/plan payload.
/// Dropping/freeing this token does not discard its session's pending plan.
#[wasm_bindgen(js_name = BinasPdfSessionPlan)]
pub struct WasmPdfSessionPlan {
    inner: PdfSessionPlanHandle,
}

fn error(error: PdfSessionError) -> JsValue {
    let details = match &error {
        PdfSessionError::StalePlan { planned, current } => {
            json!({"planned": planned, "current": current})
        }
        PdfSessionError::Text(binas_pdf::PdfTextEditError::ContractVersion {
            expected,
            actual,
        }) => json!({"expected": expected, "actual": actual}),
        _ => serde_json::Value::Null,
    };
    super::json_value(
        &json!({"code": error.code(), "message": error.to_string(), "details": details}),
    )
}

fn mode(value: &str) -> Result<PdfWriterMode, JsValue> {
    match value {
        "incremental" => Ok(PdfWriterMode::Incremental),
        "compact" => Ok(PdfWriterMode::Compact),
        _ => Err(super::json_value(&json!({
            "code": "unsupported_writer_mode", "message": "expected incremental or compact"
        }))),
    }
}

#[wasm_bindgen(js_class = BinasPdfSession)]
impl WasmPdfSession {
    /// Budget counters are logical reservations/work, not JS/WASM heap limits.
    /// Input is copied across the JS boundary; max input is checked before the
    /// retained Rust copy, but wasm-bindgen has already copied the JS argument.
    #[wasm_bindgen(constructor)]
    pub fn new(input: &[u8], max_live_bytes: u32, max_work_units: u32) -> Result<Self, JsValue> {
        if input.len() > super::WASM_MAX_INPUT_BYTES {
            return Err(super::json_value(&json!({
                "code": "resource_limit", "message": "input exceeds 8 MiB"
            })));
        }
        let context = OperationContext::new(
            ResourceBudget::new(BudgetLimits {
                max_live_bytes: u64::from(max_live_bytes),
                max_work_units: u64::from(max_work_units),
            }),
            CancellationToken::new(),
        );
        let config = EngineConfig {
            limits: Limits {
                max_input_bytes: super::WASM_MAX_INPUT_BYTES,
                max_stream_bytes: super::WASM_MAX_DECODED_BYTES,
                max_total_decoded_bytes: super::WASM_MAX_DECODED_BYTES,
                max_output_bytes: super::WASM_MAX_OUTPUT_BYTES,
                max_decode_cache_bytes: super::WASM_MAX_DECODED_BYTES,
                max_decode_live_bytes: super::WASM_MAX_DECODED_BYTES,
                ..Limits::default()
            },
        };
        Ok(Self {
            inner: PdfSession::open(input.to_vec(), config, context).map_err(error)?,
        })
    }

    pub fn snapshot(&self) -> Result<String, JsValue> {
        self.inner
            .snapshot()
            .map(|value| json!(value).to_string())
            .map_err(error)
    }

    #[wasm_bindgen(js_name = extractPageText)]
    pub fn extract_page_text(&self, page_index: u32) -> Result<String, JsValue> {
        self.inner
            .extract_page_text_spans(page_index as usize)
            .map(|value| json!(value).to_string())
            .map_err(error)
    }

    /// Only pdf.text.retained_ascii contract 1; fonted input remains refused.
    #[wasm_bindgen(js_name = prepareTextEdit)]
    pub fn prepare_text_edit(
        &mut self,
        old_text: &str,
        replacement: &str,
        match_index: u32,
        writer_mode: &str,
        expected_contract: u32,
    ) -> Result<WasmPdfSessionPlan, JsValue> {
        let handle = self
            .inner
            .prepare_text_edit(
                SurgicalTextEditRequest {
                    old_text: old_text.into(),
                    replacement: replacement.into(),
                    match_index: match_index as usize,
                },
                mode(writer_mode)?,
                expected_contract,
            )
            .map_err(error)?;
        Ok(WasmPdfSessionPlan { inner: handle })
    }

    /// Returns observed receipt facts; no external/visual assurance is implied.
    #[wasm_bindgen(js_name = acceptTextEdit)]
    pub fn accept_text_edit(&mut self, plan: &WasmPdfSessionPlan) -> Result<String, JsValue> {
        let receipt = self.inner.accept_text_edit(&plan.inner).map_err(error)?;
        let required: Vec<_> = receipt
            .required_checks()
            .codes()
            .map(ToString::to_string)
            .collect();
        let request = receipt.request();
        let limits = receipt.context_limits();
        let changed: Vec<_> = receipt
            .changed_objects()
            .iter()
            .map(|reference| {
                json!({
                    "number": reference.number, "generation": reference.generation,
                })
            })
            .collect();
        let evidence: Vec<_> = receipt.evidence().checks().iter().map(|check| json!({
            "code": check.code.to_string(),
            "status": match check.status {
                CheckStatus::Passed => "passed", CheckStatus::Failed => "failed",
                CheckStatus::NotChecked => "not_checked", CheckStatus::Unsupported => "unsupported",
            }, "detail": check.detail,
        })).collect();
        Ok(json!({
            "before": receipt.before(), "after": receipt.after(),
            "input_id": receipt.input_id(), "input_len": receipt.input_len(),
            "output_id": receipt.output_id(), "output_len": receipt.output_len(),
            "contract_version": receipt.contract_version(), "engine_version": receipt.engine_version(),
            "operation": receipt.operation(), "evidence": evidence,
            "request": {"old_text": request.old_text, "replacement": request.replacement, "match_index": request.match_index},
            "mode": match receipt.mode() { PdfWriterMode::Incremental => "incremental", PdfWriterMode::Compact => "compact", PdfWriterMode::Surgical => "surgical" },
            "required_checks": required, "changed_objects": changed,
            "pdf_limits": receipt.pdf_limits(),
            "context_limits": {"max_live_bytes": limits.max_live_bytes, "max_work_units": limits.max_work_units},
        }).to_string())
    }

    #[wasm_bindgen(js_name = releasePlan)]
    pub fn release_plan(&mut self, plan: &WasmPdfSessionPlan) -> Result<(), JsValue> {
        self.inner.release_plan(&plan.inner).map_err(error)
    }

    /// Transfers a separate JS Uint8Array. Caller owns that copy after return;
    /// logical output capacity is released on return, not when JS drops it.
    pub fn export(&self, writer_mode: &str) -> Result<js_sys::Uint8Array, JsValue> {
        let result = self.inner.export(mode(writer_mode)?).map_err(error)?;
        Ok(js_sys::Uint8Array::from(result.output.as_bytes()))
    }

    pub fn cancel(&self) {
        self.inner.context().cancellation().cancel();
    }

    #[wasm_bindgen(js_name = renewCancellation)]
    pub fn renew_cancellation(&mut self) -> Result<(), JsValue> {
        self.inner.renew_cancellation().map_err(error)
    }

    pub fn budget(&self) -> String {
        let value = self.inner.context().budget().snapshot();
        json!({"live_bytes": value.live_bytes, "work_units": value.work_units}).to_string()
    }

    pub fn close(&mut self) {
        self.inner.close();
    }
}
