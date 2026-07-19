use binas_pdf::{
    OpenOptions, PdfEngine, PdfErrorCode, XfaDatasetSetRequest, XfaReplaceRequest,
    XfaTemplateDatasetMapping, inspect_xfa_dynamic, list_xfa_dataset_fields, list_xfa_packets,
    list_xfa_template_dataset_mappings,
};

fn pdf() -> Vec<u8> {
    let template = b"<template><subform name=\"root\"/></template>";
    let config = b"<config><dynamicRender>required</dynamicRender></config>";
    xfa_packets_pdf(&[("template", template), ("config", config)])
}

fn xfa_packets_pdf(packets: &[(&str, &[u8])]) -> Vec<u8> {
    let xfa = packets
        .iter()
        .enumerate()
        .map(|(index, (label, _))| format!("({label}) {} 0 R", index + 6))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /AcroForm 5 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>".to_vec(),
        b"null".to_vec(),
        format!("<< /XFA [{xfa}] >>").into_bytes(),
    ];
    objects.extend(packets.iter().map(|(_, packet)| {
        [
            format!("<< /Length {} >>\nstream\n", packet.len()).into_bytes(),
            packet.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat()
    }));
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

fn open(input: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(input, OpenOptions::default())
        .unwrap()
}

fn static_dataset_pdf(dataset: &[u8]) -> Vec<u8> {
    static_template_dataset_pdf(b"<template><subform name=\"form\"/></template>", dataset)
}

fn static_template_dataset_pdf(template: &[u8], dataset: &[u8]) -> Vec<u8> {
    xfa_packets_pdf(&[("template", template), ("datasets", dataset)])
}

#[test]
fn lists_detects_dynamic_and_replaces_xfa_packets() {
    let input = pdf();
    let document = open(&input);
    let packets = list_xfa_packets(&document).unwrap();
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[0].label, "template");
    assert_eq!(packets[0].root_element.as_deref(), Some("template"));
    let dynamic = inspect_xfa_dynamic(&document).unwrap();
    assert!(dynamic.present && dynamic.dynamic && !dynamic.static_packets);

    let outcome = document
        .replace_xfa_text(XfaReplaceRequest {
            old_text: "root".into(),
            new_text: "customer".into(),
            packet_index: 0,
        })
        .unwrap();
    assert!(outcome.verification.passed);
    let packets = list_xfa_packets(&open(&outcome.bytes)).unwrap();
    assert!(packets[0].preview.contains("customer"));
}

#[test]
fn xfa_replacement_fails_closed_for_bad_selection_and_malformed_xml() {
    let document = open(&pdf());
    assert!(
        document
            .replace_xfa_text(XfaReplaceRequest {
                old_text: "missing".into(),
                new_text: "new".into(),
                packet_index: 0,
            })
            .is_err()
    );
    assert!(
        document
            .replace_xfa_text(XfaReplaceRequest {
                old_text: "<subform name=\"root\"/>".into(),
                new_text: "<subform".into(),
                packet_index: 0,
            })
            .is_err()
    );
}

#[test]
fn static_dataset_paths_get_set_and_remove_with_reopen_verification() {
    let input = static_dataset_pdf(
        br#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form><name>Alice &amp; Bob</name><address><city>TJ</city></address><blank/></form></xfa:data></xfa:datasets>"#,
    );
    let document = open(&input);
    let dynamic = inspect_xfa_dynamic(&document).unwrap();
    assert!(dynamic.present && !dynamic.dynamic && dynamic.static_packets);
    let fields = list_xfa_dataset_fields(&document).unwrap();
    assert_eq!(
        fields
            .iter()
            .map(|field| (field.path.as_str(), field.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("form.name", "Alice & Bob"),
            ("form.address.city", "TJ"),
            ("form.blank", ""),
        ],
    );
    assert_eq!(
        document.get_xfa_dataset_field("form.name").unwrap().value,
        "Alice & Bob"
    );

    let renamed = document
        .set_xfa_dataset_field(XfaDatasetSetRequest {
            path: "form.name".into(),
            value: "Ana & Co".into(),
        })
        .unwrap();
    assert!(renamed.verification.passed);
    let document = open(&renamed.bytes);
    assert_eq!(
        document.get_xfa_dataset_field("form.name").unwrap().value,
        "Ana & Co"
    );

    let initialized = document
        .set_xfa_dataset_field(XfaDatasetSetRequest {
            path: "form.blank".into(),
            value: "ready".into(),
        })
        .unwrap();
    assert!(initialized.verification.passed);
    let document = open(&initialized.bytes);
    assert_eq!(
        document.get_xfa_dataset_field("form.blank").unwrap().value,
        "ready"
    );

    let removed = document
        .remove_xfa_dataset_field("form.address.city")
        .unwrap();
    assert!(removed.verification.passed);
    let document = open(&removed.bytes);
    assert_eq!(
        document.get_xfa_dataset_field("form.name").unwrap().value,
        "Ana & Co"
    );
    assert_eq!(
        document
            .get_xfa_dataset_field("form.address.city")
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound,
    );
}

#[test]
fn static_dataset_mutation_refuses_flowed_and_nonstatic_packet_families() {
    let datasets = b"<datasets><data><name>Alice</name></data></datasets>";
    let flowed = open(&xfa_packets_pdf(&[
        (
            "template",
            b"<template><subform layout=\"flowed\"><field name=\"name\"/></subform></template>",
        ),
        ("datasets", datasets),
    ]));
    assert_eq!(
        flowed
            .set_xfa_dataset_field(XfaDatasetSetRequest {
                path: "name".into(),
                value: "Ana".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite,
    );

    let nonstatic = open(&xfa_packets_pdf(&[
        ("config", b"<config><present/></config>"),
        ("datasets", datasets),
    ]));
    assert_eq!(
        nonstatic
            .set_xfa_dataset_field(XfaDatasetSetRequest {
                path: "name".into(),
                value: "Ana".into(),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite,
    );
}

#[test]
fn static_dataset_paths_refuse_dynamic_ambiguous_and_container_forms() {
    assert_eq!(
        open(&pdf())
            .get_xfa_dataset_field("form.name")
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite,
    );

    let ambiguous = open(&static_dataset_pdf(
        br#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form><name>one</name><name>two</name></form></xfa:data></xfa:datasets>"#,
    ));
    assert_eq!(
        ambiguous
            .get_xfa_dataset_field("form.name")
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite,
    );
    assert_eq!(
        ambiguous.get_xfa_dataset_field("form").unwrap_err().code,
        PdfErrorCode::UnsafeRewrite,
    );

    let unsafe_xml = open(&static_dataset_pdf(
        br#"<!DOCTYPE datasets><xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data><form><name>one</name></form></xfa:data></xfa:datasets>"#,
    ));
    assert_eq!(
        unsafe_xml
            .get_xfa_dataset_field("form.name")
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite,
    );
}

#[test]
fn static_xfa_template_dataset_mappings_match_exact_field_paths() {
    let document = open(&static_template_dataset_pdf(
        b"<template><field name=\"form.payer.name\"/></template>",
        b"<datasets><data><form><payer><name>David</name></payer></form></data></datasets>",
    ));

    assert_eq!(
        list_xfa_template_dataset_mappings(&document).unwrap(),
        vec![XfaTemplateDatasetMapping {
            field_name: "form.payer.name".into(),
            dataset_path: "form.payer.name".into(),
            value: "David".into(),
            template_packet_index: 0,
            dataset_packet_index: 1,
            label: "template".into(),
        }]
    );
}

#[test]
fn static_xfa_template_dataset_mappings_use_enclosing_named_subforms() {
    let document = open(&static_template_dataset_pdf(
        b"<template><subform name=\"form\"><subform name=\"payer\"><field name=\"email\"/></subform></subform></template>",
        b"<datasets><data><form><payer><email>david@example.test</email></payer></form></data></datasets>",
    ));

    assert_eq!(
        list_xfa_template_dataset_mappings(&document).unwrap(),
        vec![XfaTemplateDatasetMapping {
            field_name: "email".into(),
            dataset_path: "form.payer.email".into(),
            value: "david@example.test".into(),
            template_packet_index: 0,
            dataset_packet_index: 1,
            label: "template".into(),
        }]
    );
}

#[test]
fn static_xfa_template_dataset_mappings_omit_ambiguous_dataset_paths() {
    let document = open(&static_template_dataset_pdf(
        b"<template><field name=\"form.name\"/></template>",
        b"<datasets><data><form><name>David</name><name>Ana</name></form></data></datasets>",
    ));

    assert!(
        list_xfa_template_dataset_mappings(&document)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn static_xfa_template_dataset_mappings_ignore_namespaced_name_attributes() {
    let document = open(&static_template_dataset_pdf(
        b"<template xmlns:x=\"urn:test\"><field x:name=\"form.payer.name\"/></template>",
        b"<datasets><data><form><payer><name>David</name></payer></form></data></datasets>",
    ));

    assert!(
        list_xfa_template_dataset_mappings(&document)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn static_xfa_template_dataset_mappings_require_exact_simple_name_attributes() {
    let document = open(&static_template_dataset_pdf(
        b"<template><field name=\" form.payer.name \"/></template>",
        b"<datasets><data><form><payer><name>David</name></payer></form></data></datasets>",
    ));

    assert!(
        list_xfa_template_dataset_mappings(&document)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn static_xfa_template_dataset_mappings_refuse_dynamic_and_unsafe_xml() {
    let dynamic = open(&xfa_packets_pdf(&[
        (
            "template",
            b"<template><subform layout=\"flowed\"><field name=\"name\"/></subform></template>",
        ),
        (
            "datasets",
            b"<datasets><data><name>Alice</name></data></datasets>",
        ),
    ]));
    assert_eq!(
        list_xfa_template_dataset_mappings(&dynamic)
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite,
    );

    let unsafe_xml = open(&static_template_dataset_pdf(
        b"<!DOCTYPE template><template><field name=\"name\"/></template>",
        b"<datasets><data><name>Alice</name></data></datasets>",
    ));
    assert_eq!(
        list_xfa_template_dataset_mappings(&unsafe_xml)
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite,
    );
}
