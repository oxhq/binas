use std::io::Write;

use binas_core::CancellationToken;
use binas_pdf::{EngineConfig, Limits, OpenOptions, PdfEngine, PdfErrorCode, list_inline_images};
use flate2::{Compression, write::ZlibEncoder};

fn pdf(content: &[u8]) -> Vec<u8> {
    let mut stream = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(content);
    stream.extend_from_slice(b"\nendstream");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        stream,
    ];
    let mut output = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        output.extend_from_slice(object);
        output.extend_from_slice(b"\nendobj\n");
    }
    let xref = output.len();
    output.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}

fn inline(filter: &[u8], encoded: &[u8]) -> Vec<u8> {
    let mut content = b"BI /W 1 /H 1 /BPC 8 /CS /G /F ".to_vec();
    content.extend_from_slice(filter);
    content.extend_from_slice(b" ID ");
    content.extend_from_slice(encoded);
    content.extend_from_slice(b" EI BT (kept) Tj ET");
    content
}

fn flate() -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"A").unwrap();
    encoder.finish().unwrap()
}

#[test]
fn inline_decoding_cannot_bypass_zero_lifetime_work_or_cancellation() {
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 0,
            ..Limits::default()
        },
    });
    let document = engine
        .open_owned(pdf(&inline(b"/AHx", b"41>")), OpenOptions::default())
        .unwrap();
    assert_eq!(
        list_inline_images(&document).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    let document = PdfEngine::default()
        .open_owned(pdf(&inline(b"/Fl", &flate())), OpenOptions::default())
        .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert_eq!(
        list_inline_images(&document.with_cancellation(cancellation))
            .unwrap_err()
            .code,
        PdfErrorCode::Cancelled
    );
}

#[test]
fn repeated_inline_decodes_and_clones_share_lifetime_work() {
    let input = pdf(&inline(b"/AHx", b"41>"));
    let limits = Limits {
        max_decode_cache_bytes: 0,
        ..Limits::default()
    };
    let measured = PdfEngine::new(EngineConfig {
        limits: limits.clone(),
    })
    .open(&input, OpenOptions::default())
    .unwrap();
    list_inline_images(&measured).unwrap();
    let cost = measured.decode_metrics().work_bytes as usize;
    assert!(cost > 3);
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 2 * cost,
            ..limits
        },
    });
    let document = engine.open_owned(input, OpenOptions::default()).unwrap();
    let identity = document.source_id();
    assert_eq!(list_inline_images(&document).unwrap().len(), 1);
    assert_eq!(list_inline_images(&document.clone()).unwrap().len(), 1);
    assert_eq!(
        list_inline_images(&document.with_cancellation(CancellationToken::new()))
            .unwrap_err()
            .code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(document.decode_metrics().work_bytes, (2 * cost) as u64);
    assert_eq!(document.source_id(), identity);
}

#[test]
fn flate_candidate_limits_are_propagated_instead_of_treated_as_bad_delimiters() {
    let input = pdf(&inline(b"/Fl", &flate()));
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_live_bytes: 0,
            ..Limits::default()
        },
    });
    let document = engine.open_owned(input, OpenOptions::default()).unwrap();
    assert_eq!(
        list_inline_images(&document).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert!(document.decode_metrics().work_bytes > 0);
    assert_eq!(document.decode_metrics().cache_entries, 0);
}

#[test]
fn per_call_sample_quota_includes_every_inline_image() {
    let mut content = inline(b"/AHx", b"41>");
    content.push(b' ');
    content.extend(inline(b"/AHx", b"41>"));
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_total_decoded_bytes: 1,
            ..Limits::default()
        },
    })
    .open_owned(pdf(&content), OpenOptions::default())
    .unwrap();
    assert_eq!(
        list_inline_images(&document).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
}

#[test]
fn noinline_and_raw_controls_do_not_require_image_decoder_work() {
    let noinline = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 0,
            ..Limits::default()
        },
    })
    .open_owned(pdf(b"BT (untouched) Tj ET"), OpenOptions::default())
    .unwrap();
    assert!(list_inline_images(&noinline).unwrap().is_empty());
    assert_eq!(noinline.decode_metrics().work_bytes, 0);
    let raw = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 1,
            ..Limits::default()
        },
    })
    .open_owned(
        pdf(b"BI /W 1 /H 1 /BPC 8 /CS /G ID A EI"),
        OpenOptions::default(),
    )
    .unwrap();
    assert_eq!(list_inline_images(&raw).unwrap().len(), 1);
    assert_eq!(raw.decode_metrics().cache_entries, 0);
    assert_eq!(raw.decode_metrics().work_bytes, 1);
}

#[test]
fn missing_ascii_terminator_cannot_hide_exhausted_scan_budget() {
    let mut content = b"BI /W 1 /H 1 /BPC 8 /CS /G /F /AHx ID ".to_vec();
    content.extend(vec![b' '; 4096]);
    let document = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_decode_work_bytes: 1,
            ..Limits::default()
        },
    })
    .open_owned(pdf(&content), OpenOptions::default())
    .unwrap();
    assert_eq!(
        list_inline_images(&document).unwrap_err().code,
        PdfErrorCode::ResourceLimit
    );
    assert_eq!(document.decode_metrics().work_bytes, 1);
}
