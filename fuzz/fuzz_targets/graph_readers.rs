#![no_main]

use binas_pdf::{
    EngineConfig, Limits, OpenOptions, PdfEngine, inspect_xfa_dynamic, list_annotations,
    list_form_fields, list_image_xobjects, list_xfa_dataset_fields, list_xfa_packets,
    list_xfa_template_dataset_mappings, read_embedded_attachment_bytes, read_embedded_attachments,
    read_jpeg_xobject_bytes, read_named_destinations, read_outlines, read_page_labels,
    read_raw_flate_image_samples, read_xmp_metadata,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let engine = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_input_bytes: 64 * 1024,
            max_output_bytes: 128 * 1024,
            max_objects: 512,
            max_xref_entries: 1024,
            max_xref_revisions: 8,
            max_parser_depth: 32,
            max_container_items: 1024,
            max_token_bytes: 8 * 1024,
            max_stream_bytes: 16 * 1024,
            max_total_decoded_bytes: 32 * 1024,
            max_pages: 64,
        },
    });
    if let Ok(document) = engine.open(&data[..data.len().min(64 * 1024)], OpenOptions::default()) {
        let _ = document.inspect();
        let _ = document.validate();
        let _ = document.extract_text_spans();
        let _ = document.query_text_all("fuzz");
        let _ = read_page_labels(&document);
        let _ = read_outlines(&document);
        let _ = read_named_destinations(&document);
        let _ = read_xmp_metadata(&document);
        if let Ok(attachments) = read_embedded_attachments(&document) {
            for attachment in attachments {
                let _ = read_embedded_attachment_bytes(&document, &attachment);
            }
        }
        if let Ok(images) = list_image_xobjects(&document) {
            for image in images {
                let _ = read_jpeg_xobject_bytes(&document, &image);
                let _ = read_raw_flate_image_samples(&document, &image);
            }
        }
        let _ = list_xfa_packets(&document);
        let _ = inspect_xfa_dynamic(&document);
        let _ = list_xfa_dataset_fields(&document);
        let _ = list_xfa_template_dataset_mappings(&document);
        let _ = list_form_fields(&document);
        let _ = list_annotations(&document);
    }
});
