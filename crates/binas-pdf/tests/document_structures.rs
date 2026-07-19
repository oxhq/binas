use std::collections::BTreeMap;

use binas_pdf::{
    DocumentInfoUpdate, EmbeddedAttachmentUpdate, EngineConfig, Limits, NamedDestinationUpdate,
    OpenOptions, OutlineCreateRequest, OutlineRemoveRequest, PageLabel, PageLabelSpec,
    PageLabelStyle, PageLabelUpdate, PdfEngine, PdfErrorCode, XmpMetadataUpdate,
    read_document_info, read_embedded_attachment_bytes, read_embedded_attachments,
    read_named_destinations, read_outlines, read_page_labels, read_xmp_metadata,
};

fn pdf() -> Vec<u8> {
    let xmp = b"<?xpacket begin=''?><x:xmpmeta xmlns:x='adobe:ns:meta/'/>";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R /Metadata 6 0 R /Names << /Custom 7 >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_vec(),
        b"<< /Length 5 >>\nstream\nBT ET\nendstream".to_vec(),
        b"<< /Title (Old title) /Producer (Binas fixture) /Custom 42 >>".to_vec(),
        [
            format!(
                "<< /Type /Metadata /Subtype /XML /Custom 7 /Length {} >>\nstream\n",
                xmp.len()
            )
            .into_bytes(),
            xmp.to_vec(),
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n")
            .as_bytes(),
    );
    bytes
}

fn xmp_pdf(stream: Vec<u8>) -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Metadata 4 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        stream,
    ])
}

fn xmp_stream(filter: &str, encoded: &[u8]) -> Vec<u8> {
    [
        format!(
            "<< /Type /Metadata /Subtype /XML {filter} /Length {} >>\nstream\n",
            encoded.len()
        )
        .into_bytes(),
        encoded.to_vec(),
        b"\nendstream".to_vec(),
    ]
    .concat()
}

fn ascii_hex(bytes: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = Vec::with_capacity(bytes.len() * 2 + 1);
    for &byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)]);
        encoded.push(HEX[usize::from(byte & 15)]);
    }
    encoded.push(b'>');
    encoded
}

fn open(bytes: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap()
}

fn classic(objects: &[Vec<u8>]) -> Vec<u8> {
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

fn attachment_pdf(stream: Vec<u8>) -> Vec<u8> {
    classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 4 0 R >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Names [(payload.bin) 5 0 R] >>".to_vec(),
        b"<< /Type /Filespec /EF << /F 6 0 R >> >>".to_vec(),
        stream,
    ])
}

fn named_tree_pdf(root: &str, children: &[&str]) -> Vec<u8> {
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R /Names << /Dests 5 0 R >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< >>".to_vec(),
        root.as_bytes().to_vec(),
    ];
    objects.extend(children.iter().map(|child| child.as_bytes().to_vec()));
    classic(&objects)
}

#[test]
fn reads_and_updates_info_without_losing_unknown_entries() {
    let document = open(&pdf());
    let before = read_document_info(&document).unwrap();
    assert_eq!(before.title.as_deref(), Some("Old title"));
    assert_eq!(before.producer.as_deref(), Some("Binas fixture"));
    assert_eq!(before.total_entries, 3);

    let outcome = document
        .update_document_info(DocumentInfoUpdate {
            entries: BTreeMap::from([
                ("Title".into(), Some("New title".into())),
                ("Author".into(), Some("OxHQ".into())),
                ("Producer".into(), None),
            ]),
        })
        .unwrap();
    assert!(outcome.verification.passed);
    assert!(outcome.verification.unknown_entries_preserved);
    let after = read_document_info(&open(&outcome.bytes)).unwrap();
    assert_eq!(after.title.as_deref(), Some("New title"));
    assert_eq!(after.author.as_deref(), Some("OxHQ"));
    assert_eq!(after.producer, None);
    assert!(
        outcome
            .bytes
            .windows(b"/Custom 42".len())
            .any(|value| value == b"/Custom 42")
    );
}

#[test]
fn replaces_and_removes_xmp_while_preserving_unknown_stream_keys() {
    let document = open(&pdf());
    assert!(read_xmp_metadata(&document).unwrap().is_some());
    let replacement =
        b"<?xpacket begin=''?><x:xmpmeta xmlns:x='adobe:ns:meta/'><test/></x:xmpmeta>".to_vec();
    let replaced = document
        .update_xmp_metadata(XmpMetadataUpdate {
            xml: Some(replacement.clone()),
        })
        .unwrap();
    assert!(replaced.verification.passed);
    assert!(replaced.verification.catalog_reachable);
    assert_eq!(
        read_xmp_metadata(&open(&replaced.bytes))
            .unwrap()
            .unwrap()
            .xml,
        replacement
    );
    assert!(
        replaced
            .bytes
            .windows(b"/Custom 7".len())
            .any(|value| value == b"/Custom 7")
    );

    let removed = open(&replaced.bytes)
        .update_xmp_metadata(XmpMetadataUpdate { xml: None })
        .unwrap();
    assert!(removed.verification.passed);
    assert!(read_xmp_metadata(&open(&removed.bytes)).unwrap().is_none());
    assert!(
        !removed
            .bytes
            .windows(b"/Type /Metadata".len())
            .any(|value| value == b"/Type /Metadata")
    );
}

#[test]
fn creates_xmp_when_catalog_has_no_metadata() {
    let source = pdf();
    let marker = b" /Metadata 6 0 R";
    let start = source
        .windows(marker.len())
        .position(|value| value == marker)
        .unwrap();
    let mut without = source;
    without[start..start + marker.len()].fill(b' ');
    let xml = b"<x:xmpmeta xmlns:x='adobe:ns:meta/'/>".to_vec();
    let outcome = open(&without)
        .update_xmp_metadata(XmpMetadataUpdate {
            xml: Some(xml.clone()),
        })
        .unwrap();
    assert_eq!(outcome.report.objects_added, 1);
    assert_eq!(
        read_xmp_metadata(&open(&outcome.bytes))
            .unwrap()
            .unwrap()
            .xml,
        xml
    );
}

#[test]
fn reads_supported_filtered_xmp_metadata() {
    let xml = b"<?xpacket begin=''?><x:xmpmeta xmlns:x='adobe:ns:meta/'/>";
    let document = open(&xmp_pdf(xmp_stream(
        "/Filter /ASCIIHexDecode",
        &ascii_hex(xml),
    )));

    assert_eq!(
        read_xmp_metadata(&document).unwrap(),
        Some(binas_pdf::XmpMetadata {
            xml: xml.to_vec(),
            object_number: 4,
            object_generation: 0,
        })
    );
}

#[test]
fn filtered_xmp_metadata_fails_closed_for_invalid_unsupported_and_over_budget_streams() {
    for (filter, encoded, code) in [
        (
            "/Filter /FlateDecode",
            b"not-a-zlib".as_slice(),
            PdfErrorCode::InvalidSyntax,
        ),
        (
            "/Filter /DCTDecode",
            b"jpeg".as_slice(),
            PdfErrorCode::UnsupportedFeature,
        ),
    ] {
        let error = read_xmp_metadata(&open(&xmp_pdf(xmp_stream(filter, encoded)))).unwrap_err();
        assert_eq!(error.code, code);
        assert_eq!(error.object, Some((4, 0)));
    }

    let limits = Limits {
        max_stream_bytes: 4,
        max_total_decoded_bytes: 4,
        ..Limits::default()
    };
    let document = PdfEngine::new(EngineConfig { limits })
        .open(
            &xmp_pdf(xmp_stream("/Filter /RunLengthDecode", &[252, b'A', 128])),
            OpenOptions::default(),
        )
        .unwrap();
    let error = read_xmp_metadata(&document).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    assert_eq!(error.object, Some((4, 0)));
}

#[test]
fn creates_named_destinations_and_linked_outlines_then_removes_a_leaf() {
    let destination = open(&pdf())
        .update_named_destination(NamedDestinationUpdate {
            name: "chapter-one".into(),
            page_index: Some(0),
        })
        .unwrap();
    assert_eq!(
        read_named_destinations(&open(&destination.bytes)).unwrap(),
        vec![binas_pdf::NamedDestination {
            name: "chapter-one".into(),
            page_index: 0,
        }]
    );
    let first = open(&destination.bytes)
        .create_outline(OutlineCreateRequest {
            title: "First".into(),
            destination_name: "chapter-one".into(),
        })
        .unwrap();
    let second = open(&first.bytes)
        .create_outline(OutlineCreateRequest {
            title: "Second".into(),
            destination_name: "chapter-one".into(),
        })
        .unwrap();
    assert_eq!(
        read_outlines(&open(&second.bytes))
            .unwrap()
            .iter()
            .map(|value| value.title.as_str())
            .collect::<Vec<_>>(),
        vec!["First", "Second"]
    );
    let removed = open(&second.bytes)
        .remove_outline(OutlineRemoveRequest { outline_index: 1 })
        .unwrap();
    assert_eq!(read_outlines(&open(&removed.bytes)).unwrap().len(), 1);
}

#[test]
fn creates_and_removes_nested_outline_subtrees() {
    let destination = open(&pdf())
        .update_named_destination(NamedDestinationUpdate {
            name: "chapter-one".into(),
            page_index: Some(0),
        })
        .unwrap();
    let parent = open(&destination.bytes)
        .create_outline(OutlineCreateRequest {
            title: "Parent".into(),
            destination_name: "chapter-one".into(),
        })
        .unwrap();
    let child = open(&parent.bytes)
        .create_child_outline(
            0,
            OutlineCreateRequest {
                title: "Child".into(),
                destination_name: "chapter-one".into(),
            },
        )
        .unwrap();
    let sibling = open(&child.bytes)
        .create_child_outline(
            0,
            OutlineCreateRequest {
                title: "Sibling".into(),
                destination_name: "chapter-one".into(),
            },
        )
        .unwrap();
    let grandchild = open(&sibling.bytes)
        .create_child_outline(
            1,
            OutlineCreateRequest {
                title: "Grandchild".into(),
                destination_name: "chapter-one".into(),
            },
        )
        .unwrap();
    assert_eq!(
        read_outlines(&open(&grandchild.bytes))
            .unwrap()
            .iter()
            .map(|item| (item.title.as_str(), item.depth))
            .collect::<Vec<_>>(),
        [
            ("Parent", 0),
            ("Child", 1),
            ("Grandchild", 2),
            ("Sibling", 1)
        ]
    );

    let without_child = open(&grandchild.bytes)
        .remove_outline(OutlineRemoveRequest { outline_index: 1 })
        .unwrap();
    assert_eq!(without_child.report.objects_removed, 2);
    assert_eq!(
        read_outlines(&open(&without_child.bytes))
            .unwrap()
            .iter()
            .map(|item| item.title.as_str())
            .collect::<Vec<_>>(),
        ["Parent", "Sibling"]
    );

    let top_level = open(&without_child.bytes)
        .create_outline(OutlineCreateRequest {
            title: "Top level".into(),
            destination_name: "chapter-one".into(),
        })
        .unwrap();
    let without_parent = open(&top_level.bytes)
        .remove_outline(OutlineRemoveRequest { outline_index: 0 })
        .unwrap();
    assert_eq!(without_parent.report.objects_removed, 2);
    assert_eq!(
        read_outlines(&open(&without_parent.bytes))
            .unwrap()
            .iter()
            .map(|item| item.title.as_str())
            .collect::<Vec<_>>(),
        ["Top level"]
    );
}

#[test]
fn refuses_closed_or_shared_outline_subtrees() {
    let closed = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 2 >>".to_vec(),
        b"<< /Title (Parent) /Parent 4 0 R /First 6 0 R /Last 6 0 R /Count -1 >>".to_vec(),
        b"<< /Title (Child) /Parent 5 0 R >>".to_vec(),
    ]));
    assert_eq!(
        closed
            .remove_outline(OutlineRemoveRequest { outline_index: 0 })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );

    let shared = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Outlines 4 0 R /Custom 5 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Outlines /First 5 0 R /Last 5 0 R /Count 1 >>".to_vec(),
        b"<< /Title (Only) /Parent 4 0 R >>".to_vec(),
    ]));
    assert_eq!(
        shared
            .remove_outline(OutlineRemoveRequest { outline_index: 0 })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}

#[test]
fn adds_reads_and_removes_an_embedded_attachment() {
    let data = b"bounded attachment bytes".to_vec();
    let added = open(&pdf())
        .update_embedded_attachment(EmbeddedAttachmentUpdate {
            name: "evidence.txt".into(),
            data: Some(data.clone()),
        })
        .unwrap();
    let attachments = read_embedded_attachments(&open(&added.bytes)).unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].name, "evidence.txt");
    assert_eq!(attachments[0].size, data.len());

    let removed = open(&added.bytes)
        .update_embedded_attachment(EmbeddedAttachmentUpdate {
            name: "evidence.txt".into(),
            data: None,
        })
        .unwrap();
    assert!(
        read_embedded_attachments(&open(&removed.bytes))
            .unwrap()
            .is_empty()
    );
    assert!(
        removed
            .bytes
            .windows(b"/Custom 7".len())
            .any(|value| value == b"/Custom 7")
    );
}

#[test]
fn reads_embedded_attachment_bytes_from_exact_inventory_entry() {
    let data = b"bounded attachment bytes".to_vec();
    let added = open(&pdf())
        .update_embedded_attachment(EmbeddedAttachmentUpdate {
            name: "evidence.txt".into(),
            data: Some(data.clone()),
        })
        .unwrap();
    let document = open(&added.bytes);
    let attachment = read_embedded_attachments(&document).unwrap().remove(0);

    assert_eq!(
        read_embedded_attachment_bytes(&document, &attachment).unwrap(),
        data
    );

    let mut stale = attachment;
    stale.name = "missing.txt".into();
    assert_eq!(
        read_embedded_attachment_bytes(&document, &stale)
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
}

#[test]
fn embedded_attachment_byte_reader_fails_closed() {
    let unsupported = open(&attachment_pdf(
        b"<< /Type /EmbeddedFile /Length 4 /Filter /DCTDecode >>\nstream\njpeg\nendstream".to_vec(),
    ));
    let attachment = read_embedded_attachments(&unsupported).unwrap().remove(0);
    let error = read_embedded_attachment_bytes(&unsupported, &attachment).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::UnsupportedFeature);
    assert_eq!(error.object, Some((6, 0)));

    let limited = PdfEngine::new(EngineConfig {
        limits: Limits {
            max_stream_bytes: 8,
            max_total_decoded_bytes: 4,
            ..Limits::default()
        },
    })
    .open(
        &attachment_pdf(
            [
                b"<< /Type /EmbeddedFile /Length 3 /Filter /RunLengthDecode >>\nstream\n"
                    .as_slice(),
                &[252, b'A', 128],
                b"\nendstream".as_slice(),
            ]
            .concat(),
        ),
        OpenOptions::default(),
    )
    .unwrap();
    let attachment = read_embedded_attachments(&limited).unwrap().remove(0);
    let error = read_embedded_attachment_bytes(&limited, &attachment).unwrap_err();
    assert_eq!(error.code, PdfErrorCode::ResourceLimit);
    assert_eq!(error.object, Some((6, 0)));

    let malformed = open(&attachment_pdf(b"<< /Type /EmbeddedFile >>".to_vec()));
    let error = read_embedded_attachment_bytes(
        &malformed,
        &binas_pdf::EmbeddedAttachment {
            name: "payload.bin".into(),
            size: 0,
            object_number: 5,
            object_generation: 0,
        },
    )
    .unwrap_err();
    assert_eq!(error.code, PdfErrorCode::InvalidSyntax);
}

#[test]
fn reads_hierarchical_name_trees_and_refuses_ambiguous_destination_insertions() {
    let document = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Names 5 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< >>".to_vec(),
        b"<< /Dests 6 0 R /EmbeddedFiles 9 0 R >>".to_vec(),
        b"<< /Kids [7 0 R 8 0 R] /Limits [(alpha) (zeta)] >>".to_vec(),
        b"<< /Names [(alpha) [3 0 R /Fit] (beta) [3 0 R /Fit]] /Limits [(alpha) (beta)] >>"
            .to_vec(),
        b"<< /Names [(zeta) << /D [3 0 R /Fit] >>] /Limits [(zeta) (zeta)] >>".to_vec(),
        b"<< /Kids [10 0 R] /Limits [(evidence.txt) (evidence.txt)] >>".to_vec(),
        b"<< /Names [(evidence.txt) 11 0 R] /Limits [(evidence.txt) (evidence.txt)] >>".to_vec(),
        b"<< /Type /Filespec /EF << /F 12 0 R >> >>".to_vec(),
        b"<< /Type /EmbeddedFile /Length 4 >>\nstream\nDATA\nendstream".to_vec(),
    ]));

    assert_eq!(
        read_named_destinations(&document)
            .unwrap()
            .into_iter()
            .map(|destination| destination.name)
            .collect::<Vec<_>>(),
        ["alpha", "beta", "zeta"]
    );
    assert_eq!(
        read_embedded_attachments(&document).unwrap(),
        vec![binas_pdf::EmbeddedAttachment {
            name: "evidence.txt".into(),
            size: 4,
            object_number: 11,
            object_generation: 0,
        }]
    );
    assert_eq!(
        document
            .update_named_destination(NamedDestinationUpdate {
                name: "middle".into(),
                page_index: Some(0),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn updates_and_prunes_hierarchical_destination_name_trees() {
    let document = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Names 5 0 R /CatalogCustom (keep) >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< >>".to_vec(),
        b"<< /Dests 6 0 R /NamesCustom (keep) >>".to_vec(),
        b"<< /Kids [7 0 R 8 0 R] /Limits [(alpha) (zeta)] /RootCustom (keep) >>".to_vec(),
        b"<< /Names [(alpha) [3 0 R /Fit] (beta) << /D [3 0 R /Fit] /EntryCustom (keep) >>] /Limits [(alpha) (beta)] /LeafCustom (keep) >>".to_vec(),
        b"<< /Kids [9 0 R] /Limits [(delta) (zeta)] /BranchCustom (keep) >>".to_vec(),
        b"<< /Names [(delta) [3 0 R /Fit] (zeta) [3 0 R /Fit]] /Limits [(delta) (zeta)] /LeafCustom (keep) >>".to_vec(),
    ]));

    let added = document
        .update_named_destination(NamedDestinationUpdate {
            name: "epsilon".into(),
            page_index: Some(0),
        })
        .unwrap();
    assert!(added.verification.passed);
    assert_eq!(
        read_named_destinations(&open(&added.bytes))
            .unwrap()
            .into_iter()
            .map(|destination| destination.name)
            .collect::<Vec<_>>(),
        ["alpha", "beta", "delta", "epsilon", "zeta"]
    );
    for marker in [
        b"/Dests 6 0 R".as_slice(),
        b"/CatalogCustom",
        b"/NamesCustom",
        b"/RootCustom",
        b"/BranchCustom",
        b"/LeafCustom",
    ] {
        assert!(
            added
                .bytes
                .windows(marker.len())
                .any(|value| value == marker),
            "missing {marker:?}",
        );
    }

    let updated = open(&added.bytes)
        .update_named_destination(NamedDestinationUpdate {
            name: "beta".into(),
            page_index: Some(0),
        })
        .unwrap();
    assert!(
        updated
            .bytes
            .windows(b"/EntryCustom <6B656570>".len())
            .any(|value| value == b"/EntryCustom <6B656570>")
    );

    let without_epsilon = open(&updated.bytes)
        .update_named_destination(NamedDestinationUpdate {
            name: "epsilon".into(),
            page_index: None,
        })
        .unwrap();
    let without_delta = open(&without_epsilon.bytes)
        .update_named_destination(NamedDestinationUpdate {
            name: "delta".into(),
            page_index: None,
        })
        .unwrap();
    let without_branch = open(&without_delta.bytes)
        .update_named_destination(NamedDestinationUpdate {
            name: "zeta".into(),
            page_index: None,
        })
        .unwrap();
    assert_eq!(
        read_named_destinations(&open(&without_branch.bytes))
            .unwrap()
            .into_iter()
            .map(|destination| destination.name)
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    for marker in [
        b"/Dests 6 0 R".as_slice(),
        b"/Kids [7 0 R] /Limits [<616C706861> <62657461>]",
        b"/RootCustom",
        b"/BranchCustom",
    ] {
        assert!(
            without_branch
                .bytes
                .windows(marker.len())
                .any(|value| value == marker),
            "missing {marker:?}",
        );
    }

    let without_alpha = open(&without_branch.bytes)
        .update_named_destination(NamedDestinationUpdate {
            name: "alpha".into(),
            page_index: None,
        })
        .unwrap();
    let empty = open(&without_alpha.bytes)
        .update_named_destination(NamedDestinationUpdate {
            name: "beta".into(),
            page_index: None,
        })
        .unwrap();
    assert!(
        read_named_destinations(&open(&empty.bytes))
            .unwrap()
            .is_empty()
    );
    assert!(
        empty
            .bytes
            .windows(b"/RootCustom".len())
            .any(|value| value == b"/RootCustom")
    );
}

#[test]
fn refuses_indirect_hierarchical_destination_values() {
    let document = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Names << /Dests 5 0 R >> >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< >>".to_vec(),
        b"<< /Kids [6 0 R] /Limits [(alpha) (alpha)] >>".to_vec(),
        b"<< /Names [(alpha) 7 0 R] /Limits [(alpha) (alpha)] >>".to_vec(),
        b"<< /D [3 0 R /Fit] /Unknown (keep) >>".to_vec(),
    ]));

    assert_eq!(
        document
            .update_named_destination(NamedDestinationUpdate {
                name: "alpha".into(),
                page_index: Some(0),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn reads_recursive_page_label_number_trees() {
    let document = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 6 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Kids [7 0 R 8 0 R] /Limits [0 2] >>".to_vec(),
        b"<< /Nums [0 << /S /r /P (front-) /St 2 >>] /Limits [0 0] >>".to_vec(),
        b"<< /Nums [2 << /S /D /P (body-) /St 10 >>] /Limits [2 2] >>".to_vec(),
    ]));
    assert_eq!(
        read_page_labels(&document).unwrap(),
        vec![
            PageLabel {
                page_index: 0,
                label: "front-ii".into(),
            },
            PageLabel {
                page_index: 1,
                label: "front-iii".into(),
            },
            PageLabel {
                page_index: 2,
                label: "body-10".into(),
            },
        ]
    );
    assert_eq!(
        document
            .update_page_label(PageLabelUpdate {
                page_index: 1,
                spec: Some(PageLabelSpec {
                    style: Some(PageLabelStyle::Decimal),
                    prefix: "body-".into(),
                    start: 1,
                }),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn updates_and_removes_flat_page_label_specs() {
    let source = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 6 0 R /CatalogCustom (keep) >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Nums [0 << /S /r /P (front-) /St 2 /SpecCustom (keep) >>] /Limits [0 0] /TreeCustom (keep) >>".to_vec(),
    ]);
    let set = open(&source)
        .update_page_label(PageLabelUpdate {
            page_index: 1,
            spec: Some(PageLabelSpec {
                style: Some(PageLabelStyle::Decimal),
                prefix: "body-".into(),
                start: 10,
            }),
        })
        .unwrap();
    assert!(set.verification.passed);
    assert_eq!(
        read_page_labels(&open(&set.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["front-ii", "body-10", "body-11"]
    );
    assert!(
        set.bytes
            .windows(b"/CatalogCustom".len())
            .any(|value| value == b"/CatalogCustom")
    );
    assert!(
        set.bytes
            .windows(b"/TreeCustom".len())
            .any(|value| value == b"/TreeCustom")
    );
    assert!(
        set.bytes
            .windows(b"/SpecCustom".len())
            .any(|value| value == b"/SpecCustom")
    );

    let removed = open(&set.bytes)
        .update_page_label(PageLabelUpdate {
            page_index: 1,
            spec: None,
        })
        .unwrap();
    assert_eq!(
        read_page_labels(&open(&removed.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["front-ii", "front-iii", "front-iv"]
    );
    let prefix_only = open(&removed.bytes)
        .update_page_label(PageLabelUpdate {
            page_index: 1,
            spec: Some(PageLabelSpec {
                style: None,
                prefix: "cover".into(),
                start: 1,
            }),
        })
        .unwrap();
    assert_eq!(
        read_page_labels(&open(&prefix_only.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["front-ii", "cover", "cover"]
    );
    let cleared = open(&removed.bytes)
        .update_page_label(PageLabelUpdate {
            page_index: 0,
            spec: None,
        })
        .unwrap();
    assert_eq!(
        read_page_labels(&open(&cleared.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["1", "2", "3"]
    );
    assert!(
        cleared
            .bytes
            .windows(b"/CatalogCustom".len())
            .any(|value| value == b"/CatalogCustom")
    );
    assert!(
        cleared
            .bytes
            .windows(b"/TreeCustom".len())
            .any(|value| value == b"/TreeCustom")
    );
    assert_eq!(
        open(&source)
            .update_page_label(PageLabelUpdate {
                page_index: 3,
                spec: None,
            })
            .unwrap_err()
            .code,
        PdfErrorCode::SelectionNotFound
    );
    assert_eq!(
        open(&source)
            .update_page_label(PageLabelUpdate {
                page_index: 0,
                spec: Some(PageLabelSpec {
                    style: Some(PageLabelStyle::Decimal),
                    prefix: String::new(),
                    start: 0,
                }),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsafeRewrite
    );
}

#[test]
fn updates_and_prunes_nested_page_label_number_trees() {
    let source = classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 8 0 R /CatalogCustom (keep) >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R 7 0 R] /Count 5 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Kids [9 0 R 10 0 R] /Limits [0 4] /TreeCustom (keep) >>".to_vec(),
        b"<< /Nums [0 << /S /r /P (front-) /St 1 >>] /Limits [0 0] >>".to_vec(),
        b"<< /Kids [11 0 R] /Limits [2 4] /BranchCustom (keep) >>".to_vec(),
        b"<< /Nums [2 << /S /D /P (body-) /St 1 /LeafCustom (keep) >> 4 << /S /A /P (appendix-) /St 1 >>] /Limits [2 4] >>".to_vec(),
    ]);
    let inserted = open(&source)
        .update_page_label(PageLabelUpdate {
            page_index: 3,
            spec: Some(PageLabelSpec {
                style: Some(PageLabelStyle::Decimal),
                prefix: "mid-".into(),
                start: 10,
            }),
        })
        .unwrap();
    assert_eq!(
        read_page_labels(&open(&inserted.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["front-i", "front-ii", "body-1", "mid-10", "appendix-A"]
    );
    for key in [
        b"/CatalogCustom".as_slice(),
        b"/TreeCustom",
        b"/BranchCustom",
        b"/LeafCustom",
    ] {
        assert!(inserted.bytes.windows(key.len()).any(|value| value == key));
    }

    let without_mid = open(&inserted.bytes)
        .update_page_label(PageLabelUpdate {
            page_index: 3,
            spec: None,
        })
        .unwrap();
    assert_eq!(
        read_page_labels(&open(&without_mid.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["front-i", "front-ii", "body-1", "body-2", "appendix-A"]
    );
    let without_body = open(&without_mid.bytes)
        .update_page_label(PageLabelUpdate {
            page_index: 2,
            spec: None,
        })
        .unwrap();
    let collapsed = open(&without_body.bytes)
        .update_page_label(PageLabelUpdate {
            page_index: 4,
            spec: None,
        })
        .unwrap();
    assert_eq!(
        read_page_labels(&open(&collapsed.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["front-i", "front-ii", "front-iii", "front-iv", "front-v"]
    );
    let cleared = open(&collapsed.bytes)
        .update_page_label(PageLabelUpdate {
            page_index: 0,
            spec: None,
        })
        .unwrap();
    assert_eq!(
        read_page_labels(&open(&cleared.bytes))
            .unwrap()
            .into_iter()
            .map(|label| label.label)
            .collect::<Vec<_>>(),
        ["1", "2", "3", "4", "5"]
    );
    for key in [b"/CatalogCustom".as_slice(), b"/TreeCustom"] {
        assert!(cleared.bytes.windows(key.len()).any(|value| value == key));
    }
}

#[test]
fn refuses_ambiguous_hierarchical_page_label_insertions() {
    let document = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 6 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< /Kids [7 0 R 8 0 R] /Limits [0 2] >>".to_vec(),
        b"<< /Nums [0 << /S /D >>] /Limits [0 0] >>".to_vec(),
        b"<< /Nums [2 << /S /D >>] /Limits [2 2] >>".to_vec(),
    ]));
    assert_eq!(
        document
            .update_page_label(PageLabelUpdate {
                page_index: 1,
                spec: Some(PageLabelSpec {
                    style: Some(PageLabelStyle::Decimal),
                    prefix: "middle-".into(),
                    start: 1,
                }),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn refuses_stream_backed_page_label_specs() {
    let document = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 6 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< >>".to_vec(),
        b"<< >>".to_vec(),
        b"<< /Nums [0 7 0 R] >>".to_vec(),
        b"<< /S /D /Length 4 >>\nstream\nkeep\nendstream".to_vec(),
    ]));
    assert_eq!(
        document
            .update_page_label(PageLabelUpdate {
                page_index: 0,
                spec: Some(PageLabelSpec {
                    style: Some(PageLabelStyle::Decimal),
                    prefix: String::new(),
                    start: 1,
                }),
            })
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}

#[test]
fn rejects_malformed_hierarchical_name_trees() {
    let unsorted = open(&named_tree_pdf(
        "<< /Names [(beta) [3 0 R /Fit] (alpha) [3 0 R /Fit]] >>",
        &[],
    ));
    assert_eq!(
        read_named_destinations(&unsorted).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );

    let bad_limits = open(&named_tree_pdf(
        "<< /Names [(alpha) [3 0 R /Fit]] /Limits [(beta) (beta)] >>",
        &[],
    ));
    assert_eq!(
        read_named_destinations(&bad_limits).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );

    let unordered_children = open(&named_tree_pdf(
        "<< /Kids [6 0 R 7 0 R] >>",
        &[
            "<< /Names [(zeta) [3 0 R /Fit]] >>",
            "<< /Names [(alpha) [3 0 R /Fit]] >>",
        ],
    ));
    assert_eq!(
        read_named_destinations(&unordered_children)
            .unwrap_err()
            .code,
        PdfErrorCode::InvalidSyntax
    );

    let cyclic = open(&named_tree_pdf(
        "<< /Kids [6 0 R] >>",
        &["<< /Kids [5 0 R] >>"],
    ));
    assert_eq!(
        read_named_destinations(&cyclic).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );

    let invalid_labels = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /PageLabels 5 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R >>".to_vec(),
        b"<< >>".to_vec(),
        b"<< /Nums [0 << /S /D >>] /Limits [1 1] >>".to_vec(),
    ]));
    assert_eq!(
        read_page_labels(&invalid_labels).unwrap_err().code,
        PdfErrorCode::InvalidSyntax
    );
}

#[test]
fn updates_flat_name_tree_limits_without_rebuilding_the_tree() {
    let outcome = open(&named_tree_pdf(
        "<< /Names [(old) [3 0 R /Fit]] /Limits [(old) (old)] >>",
        &[],
    ))
    .update_named_destination(NamedDestinationUpdate {
        name: "new".into(),
        page_index: Some(0),
    })
    .unwrap();

    assert_eq!(
        read_named_destinations(&open(&outcome.bytes))
            .unwrap()
            .into_iter()
            .map(|destination| destination.name)
            .collect::<Vec<_>>(),
        ["new", "old"]
    );
}
