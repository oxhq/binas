use binas_pdf::{OpenOptions, PdfEngine, PdfErrorCode, read_javascript_actions};

fn classic(objects: &[&[u8]]) -> Vec<u8> {
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

fn open(bytes: &[u8]) -> binas_pdf::PdfDocument {
    PdfEngine::default()
        .open(bytes, OpenOptions::default())
        .unwrap()
}

#[test]
fn inventories_direct_and_catalog_reachable_javascript_without_execution() {
    let document = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Names 4 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        b"<< /JavaScript 5 0 R >>",
        b"<< /Kids [6 0 R] >>",
        b"<< /Names [(ReachableJS) 7 0 R (Utf16JS) 8 0 R] /Limits [(ReachableJS) (Utf16JS)] >>",
        b"<< /S /JavaScript /JS (app.alert('reachable')) >>",
        b"<< /S /JavaScript /JS <FEFF0061006C006500720074002800270068006900270029> >>",
        b"<< /S /JavaScript /JS (app.alert('orphan')) >>",
    ]));

    let inventory = read_javascript_actions(&document).unwrap();
    assert_eq!(inventory.direct.len(), 3);
    assert_eq!(
        inventory
            .direct
            .iter()
            .map(|action| (
                action.object_number,
                action.object_generation,
                action.name.as_deref(),
                action.script.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (7, 0, None, "app.alert('reachable')"),
            (8, 0, None, "alert('hi')"),
            (9, 0, None, "app.alert('orphan')"),
        ]
    );
    assert_eq!(
        inventory
            .name_tree
            .iter()
            .map(|action| (
                action.object_number,
                action.name.as_deref(),
                action.script.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (7, Some("ReachableJS"), "app.alert('reachable')"),
            (8, Some("Utf16JS"), "alert('hi')"),
        ]
    );
}

#[test]
fn refuses_unreadable_scripts_and_nonreference_name_tree_values() {
    let unreadable_script = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        b"<< /S /JavaScript /JS <FF> >>",
    ]));
    assert_eq!(
        read_javascript_actions(&unreadable_script)
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );

    let malformed_utf16 = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        b"<< /S /JavaScript /JS <FEFF00> >>",
    ]));
    assert_eq!(
        read_javascript_actions(&malformed_utf16).unwrap_err().code,
        PdfErrorCode::UnsupportedFeature
    );

    let inline_name_tree_value = open(&classic(&[
        b"<< /Type /Catalog /Pages 2 0 R /Names 4 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>",
        b"<< /JavaScript 5 0 R >>",
        b"<< /Names [(inline) << /S /JavaScript /JS (never execute) >>] >>",
    ]));
    assert_eq!(
        read_javascript_actions(&inline_name_tree_value)
            .unwrap_err()
            .code,
        PdfErrorCode::UnsupportedFeature
    );
}
