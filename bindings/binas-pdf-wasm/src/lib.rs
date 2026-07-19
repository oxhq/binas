use binas_pdf::{
    EngineConfig, FilteredEditOutcome, FilteredTextEditRequest, FontEditOutcome,
    FontTextEditRequest, IncrementalEditOutcome, IncrementalTextEditRequest, Limits, OpenOptions,
    PdfEngine, PdfError, PdfErrorCode, SurgicalEditOutcome, SurgicalTextEditRequest,
    inspect_encryption, inspect_signatures,
};
use serde_json::json;
use wasm_bindgen::prelude::*;

const WASM_MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
const WASM_MAX_DECODED_BYTES: usize = 16 * 1024 * 1024;
const WASM_MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[wasm_bindgen(js_name = binasInspectPDF)]
pub fn inspect_pdf(input: &[u8]) -> String {
    inspect_json(input)
}

#[wasm_bindgen(js_name = binasQueryPDF)]
pub fn query_pdf(input: &[u8], text: &str) -> String {
    query_json(input, text)
}

#[wasm_bindgen(js_name = binasExtractPDFText)]
pub fn extract_pdf_text(input: &[u8]) -> String {
    extract_text_json(input)
}

#[wasm_bindgen(js_name = binasEditPDFText)]
pub fn edit_pdf_text(input: &[u8], old_text: &str, new_text: &str) -> JsValue {
    match edit(input, old_text, new_text) {
        Ok(outcome) => edit_outcome_to_js(outcome),
        Err(error) => error_to_js(&error),
    }
}

#[derive(Debug)]
enum EditOutcome {
    Surgical(SurgicalEditOutcome),
    Incremental(IncrementalEditOutcome),
    Filtered(FilteredEditOutcome),
    Font(FontEditOutcome),
}

fn edit(input: &[u8], old_text: &str, replacement: &str) -> Result<EditOutcome, PdfError> {
    let document = wasm_engine().open(input, OpenOptions::default())?;
    if document.query_text(old_text, 0)?.font_name.is_some() {
        return document
            .font_text_edit(FontTextEditRequest {
                old_text: old_text.into(),
                replacement: replacement.into(),
                match_index: 0,
            })
            .map(EditOutcome::Font);
    }
    let request = SurgicalTextEditRequest {
        old_text: old_text.into(),
        replacement: replacement.into(),
        match_index: 0,
    };
    match document.surgical_text_edit(request) {
        Ok(outcome) => Ok(EditOutcome::Surgical(outcome)),
        Err(error) if error.code == PdfErrorCode::UnsafeRewrite => match document
            .incremental_text_edit(IncrementalTextEditRequest {
                old_text: old_text.into(),
                replacement: replacement.into(),
                match_index: 0,
            }) {
            Ok(outcome) => Ok(EditOutcome::Incremental(outcome)),
            Err(error) if error.code == PdfErrorCode::UnsafeRewrite => document
                .filtered_text_edit(FilteredTextEditRequest {
                    old_text: old_text.into(),
                    replacement: replacement.into(),
                    match_index: 0,
                })
                .map(EditOutcome::Filtered),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    }
}

fn edit_outcome_to_js(outcome: EditOutcome) -> JsValue {
    match outcome {
        EditOutcome::Surgical(outcome) => {
            edit_parts_to_js(&outcome.bytes, &outcome.report, &outcome.verification)
        }
        EditOutcome::Incremental(outcome) => {
            edit_parts_to_js(&outcome.bytes, &outcome.report, &outcome.verification)
        }
        EditOutcome::Filtered(outcome) => {
            edit_parts_to_js(&outcome.bytes, &outcome.report, &outcome.verification)
        }
        EditOutcome::Font(outcome) => {
            edit_parts_to_js(&outcome.bytes, &outcome.report, &outcome.verification)
        }
    }
}

fn edit_parts_to_js(
    bytes: &[u8],
    report: &impl serde::Serialize,
    verification: &impl serde::Serialize,
) -> JsValue {
    let object = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&object, &JsValue::from_str("ok"), &JsValue::from_bool(true));
    let _ = js_sys::Reflect::set(
        &object,
        &JsValue::from_str("bytes"),
        &js_sys::Uint8Array::from(bytes),
    );
    let _ = js_sys::Reflect::set(&object, &JsValue::from_str("report"), &json_value(report));
    let _ = js_sys::Reflect::set(
        &object,
        &JsValue::from_str("verification"),
        &json_value(verification),
    );
    object.into()
}

fn error_to_js(error: &PdfError) -> JsValue {
    let object = js_sys::Object::new();
    let _ = js_sys::Reflect::set(
        &object,
        &JsValue::from_str("ok"),
        &JsValue::from_bool(false),
    );
    let _ = js_sys::Reflect::set(
        &object,
        &JsValue::from_str("code"),
        &JsValue::from_str(error.code.as_str()),
    );
    let _ = js_sys::Reflect::set(
        &object,
        &JsValue::from_str("error"),
        &JsValue::from_str(&error.to_string()),
    );
    object.into()
}

fn json_value(value: &impl serde::Serialize) -> JsValue {
    serde_json::to_string(value)
        .ok()
        .and_then(|json| js_sys::JSON::parse(&json).ok())
        .unwrap_or(JsValue::NULL)
}

fn inspect_json(input: &[u8]) -> String {
    let document = match wasm_engine().open(input, OpenOptions::default()) {
        Ok(document) => document,
        Err(error) => return error_json(&error),
    };
    let result = match document.inspect() {
        Ok(result) => result,
        Err(error) => return error_json(&error),
    };
    let encryption = match inspect_encryption(&document) {
        Ok(encryption) => encryption,
        Err(error) => return error_json(&error),
    };
    let signatures = match inspect_signatures(&document) {
        Ok(signatures) => signatures,
        Err(error) => return error_json(&error),
    };
    json!({
        "ok": true,
        "format": "pdf",
        "nodes": result.object_count,
        "version": result.version,
        "pages": result.page_count,
        "xref_revisions": result.xref_revisions,
        "encryption": encryption,
        "signatures": signatures,
    })
    .to_string()
}

fn query_json(input: &[u8], text: &str) -> String {
    let document = match wasm_engine().open(input, OpenOptions::default()) {
        Ok(document) => document,
        Err(error) => return error_json(&error),
    };
    let matches = match document.query_text_all(text) {
        Ok(matches) => matches,
        Err(error) => return error_json(&error),
    };
    json!({ "ok": true, "count": matches.len(), "matches": matches }).to_string()
}

fn extract_text_json(input: &[u8]) -> String {
    let document = match wasm_engine().open(input, OpenOptions::default()) {
        Ok(document) => document,
        Err(error) => return error_json(&error),
    };
    match document.extract_text_spans() {
        Ok(extraction) => json!({ "ok": true, "extraction": extraction }).to_string(),
        Err(error) => error_json(&error),
    }
}

fn wasm_engine() -> PdfEngine {
    PdfEngine::new(EngineConfig {
        limits: Limits {
            max_input_bytes: WASM_MAX_INPUT_BYTES,
            max_stream_bytes: WASM_MAX_DECODED_BYTES,
            max_total_decoded_bytes: WASM_MAX_DECODED_BYTES,
            max_output_bytes: WASM_MAX_OUTPUT_BYTES,
            ..Limits::default()
        },
    })
}

fn error_json(error: &PdfError) -> String {
    json!({
        "ok": false,
        "code": error.code.as_str(),
        "error": error.to_string(),
    })
    .to_string()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn edit_refuses_invalid_input_without_claiming_success() {
        let error = edit(b"not a pdf", "old", "new").unwrap_err();
        assert_eq!(error.code.as_str(), "invalid_syntax");
    }

    #[test]
    fn edit_helper_returns_verified_pdf_bytes() {
        let input = include_bytes!("../../../testdata/pdf/multiple-streams.pdf");
        let outcome = edit(input, "first", "third").unwrap();
        let EditOutcome::Surgical(outcome) = outcome else {
            panic!("same-length edit should be surgical");
        };
        assert!(outcome.verification.passed);
        assert_eq!(outcome.bytes.len(), input.len());
    }

    #[test]
    fn edit_helper_falls_back_to_incremental_for_length_change() {
        let input = include_bytes!("../../../testdata/pdf/multiple-streams.pdf");
        let outcome = edit(input, "first", "longer replacement").unwrap();
        let EditOutcome::Incremental(outcome) = outcome else {
            panic!("length-changing edit should be incremental");
        };
        assert!(outcome.verification.passed);
        assert!(outcome.bytes.starts_with(input));
    }

    #[test]
    fn invalid_pdf_returns_json_error() {
        let value: serde_json::Value = serde_json::from_str(&inspect_json(b"not a pdf")).unwrap();
        assert_eq!(value["ok"], false);
        assert!(value["error"].is_string());
    }

    #[test]
    fn structured_text_extraction_returns_spans() {
        let input = include_bytes!("../../../testdata/pdf/multiple-streams.pdf");
        let value: serde_json::Value = serde_json::from_str(&extract_text_json(input)).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["extraction"]["spans"].as_array().unwrap().len(), 2);
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod browser_tests {
    use super::*;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    const FIXTURE: &[u8] = include_bytes!("../../../testdata/pdf/multiple-streams.pdf");

    #[wasm_bindgen_test]
    fn exports_keep_the_rust_browser_contract_shapes() {
        let inspect = parse(inspect_pdf(FIXTURE));
        assert_eq!(
            keys(&inspect),
            [
                "encryption",
                "format",
                "nodes",
                "ok",
                "pages",
                "signatures",
                "version",
                "xref_revisions"
            ]
        );
        assert_eq!(inspect["ok"], true);
        assert_eq!(inspect["format"], "pdf");
        assert_eq!(inspect["nodes"], 5);
        assert_eq!(inspect["pages"], 1);
        assert_eq!(inspect["version"], "1.3");
        assert_eq!(inspect["xref_revisions"], 1);
        assert_eq!(
            keys(&inspect["encryption"]),
            [
                "encrypt_metadata",
                "encrypted",
                "filter",
                "key_length_bits",
                "object_generation",
                "object_number",
                "permissions",
                "revision",
                "stream_filter",
                "string_filter",
                "sub_filter",
                "version"
            ]
        );
        assert_eq!(inspect["encryption"]["encrypted"], false);
        assert_eq!(inspect["signatures"], serde_json::json!([]));

        let query = parse(query_pdf(FIXTURE, "first"));
        assert_eq!(keys(&query), ["count", "matches", "ok"]);
        assert_eq!(query["ok"], true);
        assert_eq!(query["count"], 1);
        let found = &query["matches"][0];
        assert_eq!(
            keys(found),
            [
                "decoded_span",
                "font_name",
                "generation",
                "match_index",
                "object_number",
                "operator",
                "source_span",
                "text",
                "to_unicode"
            ]
        );
        assert_eq!(found["text"], "first");
        assert_eq!(found["operator"], "Tj");

        let extracted = parse(extract_pdf_text(FIXTURE));
        assert_eq!(keys(&extracted), ["extraction", "ok"]);
        assert_eq!(extracted["ok"], true);
        assert_eq!(keys(&extracted["extraction"]), ["spans", "warnings"]);
        assert_eq!(extracted["extraction"]["warnings"], serde_json::json!([]));
        assert_eq!(
            extracted["extraction"]["spans"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            keys(&extracted["extraction"]["spans"][0]),
            [
                "decoded_span",
                "font_name",
                "generation",
                "geometry",
                "object_number",
                "operator",
                "page_index",
                "source_span",
                "text",
                "to_unicode",
            ]
        );
        assert_eq!(extracted["extraction"]["spans"][0]["text"], "first");

        let edited = edit_pdf_text(FIXTURE, "first", "third");
        assert_eq!(property(&edited, "ok").as_bool(), Some(true));
        let bytes = js_sys::Uint8Array::new(&property(&edited, "bytes")).to_vec();
        assert!(!bytes.is_empty());
        assert_eq!(
            property(&property(&edited, "verification"), "passed").as_bool(),
            Some(true)
        );
        assert_eq!(parse(query_pdf(&bytes, "third"))["count"], 1);
    }

    #[wasm_bindgen_test]
    fn exports_refuse_input_above_the_wasm_budget() {
        let oversized = vec![0_u8; WASM_MAX_INPUT_BYTES + 1];
        for value in [
            inspect_pdf(&oversized),
            query_pdf(&oversized, "needle"),
            extract_pdf_text(&oversized),
        ] {
            let error = parse(value);
            assert_eq!(keys(&error), ["code", "error", "ok"]);
            assert_eq!(error["ok"], false);
            assert_eq!(error["code"], "resource_limit");
        }
        let error = edit_pdf_text(&oversized, "old", "new");
        assert_eq!(property(&error, "ok").as_bool(), Some(false));
        assert_eq!(
            property(&error, "code").as_string().as_deref(),
            Some("resource_limit")
        );
    }

    fn parse(value: String) -> serde_json::Value {
        serde_json::from_str(&value).unwrap()
    }

    fn keys(value: &serde_json::Value) -> Vec<&str> {
        let mut keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        keys
    }

    fn property(value: &JsValue, key: &str) -> JsValue {
        js_sys::Reflect::get(value, &JsValue::from_str(key)).unwrap()
    }
}
