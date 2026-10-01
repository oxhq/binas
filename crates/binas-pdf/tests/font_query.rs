use std::io::Write;

use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode};
use flate2::{Compression, write::ZlibEncoder};

fn flate(input: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(input).unwrap();
    encoder.finish().unwrap()
}

fn stream(data: &[u8], filtered: bool) -> Vec<u8> {
    format!(
        "<< /Length {}{} >>\nstream\n",
        data.len(),
        if filtered {
            " /Filter /FlateDecode"
        } else {
            ""
        }
    )
    .into_bytes()
    .into_iter()
    .chain(data.iter().copied())
    .chain(b"\nendstream".iter().copied())
    .collect()
}

fn pdf(content: &[u8], cmap: Option<&[u8]>, filtered: bool) -> Vec<u8> {
    let content_data = if filtered {
        flate(content)
    } else {
        content.to_vec()
    };
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /Resources 5 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream(&content_data, filtered),
        b"<< /Font << /F1 6 0 R >> >>".to_vec(),
        if cmap.is_some() {
            b"<< /Type /Font /Subtype /Type0 /ToUnicode 7 0 R >>".to_vec()
        } else {
            b"<< /Type /Font /Subtype /Type1 >>".to_vec()
        },
    ];
    if let Some(cmap) = cmap {
        let data = if filtered { flate(cmap) } else { cmap.to_vec() };
        objects.push(stream(&data, filtered));
    }

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
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    bytes
}

fn cmap() -> &'static [u8] {
    b"2 begincodespacerange <00> <ff> <0100> <01ff> endcodespacerange\n\
      2 beginbfchar <01> <0041> <0102> <03a9> endbfchar"
}

#[test]
fn resolves_inherited_fonts_and_filtered_variable_width_tj() {
    let content = b"BT /F1 12 Tf [<0102> -20 <01> <01>] TJ ET";
    let input = pdf(content, Some(cmap()), true);
    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();

    let omega = document.query_text("Omega", 0).unwrap_err();
    assert_eq!(omega.code, PdfErrorCode::SelectionNotFound);
    let omega = document.query_text("\u{3a9}", 0).unwrap();
    assert_eq!(omega.font_name.as_deref(), Some("F1"));
    assert!(omega.to_unicode);
    assert_eq!(omega.source_span, None);
    assert_eq!(
        omega.decoded_span.slice(content),
        Some(b"<0102>".as_slice())
    );

    let a = document.query_text_all("A").unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!([a[0].match_index, a[1].match_index], [0, 1]);
    assert!(a[0].decoded_span.start() < a[1].decoded_span.start());
    assert_eq!(a[0].decoded_span.slice(content), Some(b"<01>".as_slice()));
}

#[test]
fn reports_truthful_source_spans_and_strict_fallback() {
    let content = b"BT /F1 12 Tf (plain) Tj ET";
    let input = pdf(content, None, false);
    let found = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap()
        .query_text("plain", 0)
        .unwrap();

    assert!(!found.to_unicode);
    assert_eq!(found.font_name.as_deref(), Some("F1"));
    assert_eq!(
        found.decoded_span.slice(content),
        Some(b"(plain)".as_slice())
    );
    assert_eq!(
        found.source_span.unwrap().slice(&input),
        Some(b"(plain)".as_slice())
    );

    let invalid = pdf(b"BT /F1 12 Tf <ff> Tj ET", None, false);
    let error = PdfEngine::default()
        .open(&invalid, OpenOptions::default())
        .unwrap()
        .query_text_all("anything")
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    assert!(error.message.contains("not UTF-8"));
}

#[test]
fn missing_mappings_and_cumulative_decode_budget_fail_closed() {
    let missing = pdf(b"BT /F1 12 Tf <02> Tj ET", Some(cmap()), false);
    let error = PdfEngine::default()
        .open(&missing, OpenOptions::default())
        .unwrap()
        .query_text_all("anything")
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    assert!(error.message.contains("<02>"));

    let content = b"BT /F1 12 Tf <01> Tj ET";
    let input = pdf(content, Some(cmap()), true);
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_total_decoded_bytes: content.len() + cmap().len() - 1,
            ..Limits::default()
        },
    });
    let error = engine
        .open(&input, OpenOptions::default())
        .unwrap()
        .query_text_all("A")
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    assert!(error.message.contains("decoded"));
}

#[test]
fn unicode_expansion_is_bounded_before_retained_planning_can_publish() {
    use binas_core::{BudgetLimits, CancellationToken, OperationContext, ResourceBudget};
    use binas_pdf::{
        PdfPageTextEditRequest, PdfRevision, PdfRevisionRequest, PdfSharedEditMode,
        SurgicalTextEditRequest,
    };
    // Both encoded tokens fit the limit; the repeated one-byte code expands to
    // 128 UTF-8 bytes through a 16-character destination.
    let cmap = format!(
        "1 begincodespacerange <00> <ff> endcodespacerange 1 beginbfchar <41> <{}> endbfchar",
        "0041".repeat(16)
    );
    let input = pdf(
        b"BT /F1 12 Tf (AAAAAAAA) Tj ET",
        Some(cmap.as_bytes()),
        false,
    );
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_token_bytes: 96,
            ..Limits::default()
        },
    })
    .open_owned(input, OpenOptions::default())
    .unwrap();
    let first = document.extract_page_text_spans(0).unwrap_err();
    assert_eq!(first.code, PdfErrorCode::ResourceLimit);
    assert!(first.message.contains("max_token_bytes"));
    let spent = document.decode_metrics().work_bytes;
    let revision = PdfRevision::new(document);
    let snapshot = revision.snapshot();
    let context = OperationContext::new(
        ResourceBudget::new(BudgetLimits {
            max_live_bytes: 1_000_000,
            max_work_units: 1_000_000,
        }),
        CancellationToken::new(),
    );
    let error = revision
        .plan(
            PdfRevisionRequest {
                page_text: vec![PdfPageTextEditRequest {
                    page_index: 0,
                    edit: SurgicalTextEditRequest {
                        old_text: "A".into(),
                        replacement: "B".into(),
                        match_index: 0,
                    },
                    shared: PdfSharedEditMode::Reject,
                }],
                ..Default::default()
            },
            &context,
        )
        .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    assert_eq!(revision.snapshot(), snapshot);
    assert!(revision.decode_metrics().work_bytes > spent);
}

#[test]
fn repeated_cmap_extraction_spends_work_after_cache_hits() {
    let input = pdf(b"BT /F1 12 Tf <01> Tj ET", Some(cmap()), true);
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 20_000,
            ..Limits::default()
        },
    })
    .open_owned(input, OpenOptions::default())
    .unwrap();
    assert_eq!(
        document.extract_page_text_spans(0).unwrap().spans[0].text,
        "A"
    );
    let first = document.decode_metrics();
    assert_eq!(
        document.extract_page_text_spans(0).unwrap().spans[0].text,
        "A"
    );
    let second = document.decode_metrics();
    assert!(second.hits > first.hits);
    assert!(second.work_bytes > first.work_bytes);
    let mut exhausted = false;
    for _ in 0..100 {
        match document.clone().extract_page_text_spans(0) {
            Ok(_) => {}
            Err(error) => {
                assert_eq!(error.code, PdfErrorCode::ResourceLimit);
                exhausted = true;
                break;
            }
        }
    }
    assert!(
        exhausted,
        "cached CMap/text interpretation cannot reset cumulative work"
    );
    document.clear_decode_cache();
    assert_eq!(
        document.extract_page_text_spans(0).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}
