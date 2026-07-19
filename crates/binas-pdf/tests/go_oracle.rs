use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use binas_pdf::{
    AnnotationContentsMutationRequest, FilteredTextEditRequest, FontTextEditRequest,
    FormValueMutationRequest, IncrementalTextEditRequest, OpenOptions, PdfDocument, PdfEngine,
    PdfErrorCode, SurgicalTextEditRequest, list_annotations, list_form_fields,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Manifest {
    version: u32,
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
struct Vector {
    file: String,
    queries: Vec<String>,
    expected_error: Option<String>,
    #[serde(default)]
    repair: bool,
    edit: Option<EditVector>,
    known_difference: Option<String>,
    #[serde(default)]
    go_parse_error: bool,
}

#[derive(Deserialize)]
struct EditVector {
    old: String,
    replacement: String,
}

#[test]
fn matches_the_frozen_go_oracle_for_the_redistributable_corpus() {
    let root = workspace_root();
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(package_root.join("tests/corpus/go-oracle.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.version, 1);
    let named = manifest
        .vectors
        .iter()
        .map(|vector| vector.file.as_str())
        .collect::<BTreeSet<_>>();
    let fixture_root = package_root.join("tests/fixtures/pdf");
    let files = fs::read_dir(&fixture_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.ends_with(".pdf"))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        named,
        files.iter().map(String::as_str).collect(),
        "every redistributable PDF must have a named differential vector"
    );
    let oracle = root.join("go.mod").is_file().then(|| build_oracle(&root));
    let engine = PdfEngine::default();

    for vector in manifest.vectors {
        let path = fixture_root.join(&vector.file);
        let bytes = fs::read(&path).unwrap();
        if let Some(expected) = vector.expected_error {
            let rust_error = engine.open(&bytes, OpenOptions::default()).unwrap_err();
            assert_eq!(rust_error.code.as_str(), expected, "Rust: {}", vector.file);
            if let Some(oracle) = &oracle {
                let go = go_json(
                    oracle,
                    &root,
                    [
                        "inspect",
                        path.to_str().unwrap(),
                        "--format",
                        "pdf",
                        "--json",
                    ],
                );
                assert_eq!(
                    go.get("parse_error").is_some(),
                    vector.go_parse_error,
                    "unexpected Go parse status for {}: {go}",
                    vector.file
                );
            }
            continue;
        }

        let document = engine
            .open(
                &bytes,
                OpenOptions {
                    repair: vector.repair,
                },
            )
            .unwrap_or_else(|error| panic!("Rust failed to open {}: {error}", vector.file));
        if let Some(oracle) = &oracle {
            let go = go_json(
                oracle,
                &root,
                [
                    "inspect",
                    path.to_str().unwrap(),
                    "--format",
                    "pdf",
                    "--json",
                ],
            );
            assert_eq!(
                go.get("parse_error").is_some(),
                vector.go_parse_error,
                "unexpected Go parse status for {}: {go}",
                vector.file
            );
        }
        for query in vector.queries {
            let rust_count = document.query_text_all(&query).unwrap().len();
            if let Some(oracle) = &oracle {
                let go = go_json(
                    oracle,
                    &root,
                    [
                        "query",
                        path.to_str().unwrap(),
                        "--format",
                        "pdf",
                        "--kind",
                        "pdf.content.text_show",
                        "--text",
                        &query,
                        "--json",
                    ],
                );
                assert_eq!(
                    rust_count,
                    go["count"].as_u64().unwrap() as usize,
                    "query parity for {} in {}",
                    query,
                    vector.file,
                );
            }
        }
        if let Some(edit) = vector.edit {
            if let Some(oracle) = &oracle {
                compare_edit(oracle, &root, &path, &document, &vector.file, edit);
            } else {
                compare_rust_edit(&document, edit);
            }
        }
        if let Some(difference) = vector.known_difference {
            assert!(
                !difference.trim().is_empty(),
                "empty difference for {}",
                vector.file
            );
        }
    }

    if let Some(oracle) = oracle {
        compare_interactive_mutations(&oracle, &root);
        let _ = fs::remove_file(oracle);
    }
}

fn compare_rust_edit(document: &PdfDocument, edit: EditVector) {
    let bytes = rust_edit(document, &edit.old, &edit.replacement);
    let reopened = PdfEngine::default()
        .open(&bytes, OpenOptions::default())
        .unwrap();
    assert!(reopened.query_text_all(&edit.old).unwrap().is_empty());
    assert_eq!(reopened.query_text_all(&edit.replacement).unwrap().len(), 1);
}

fn compare_interactive_mutations(oracle: &Path, root: &Path) {
    let input = interactive_pdf();
    let base = format!("binas-interactive-differential-{}", std::process::id());
    let input_path = std::env::temp_dir().join(format!("input-{base}.pdf"));
    let rust_form = std::env::temp_dir().join(format!("rust-form-{base}.pdf"));
    let go_form = std::env::temp_dir().join(format!("go-form-{base}.pdf"));
    let rust_annotation = std::env::temp_dir().join(format!("rust-annotation-{base}.pdf"));
    let go_annotation = std::env::temp_dir().join(format!("go-annotation-{base}.pdf"));
    fs::write(&input_path, &input).unwrap();

    let document = PdfEngine::default()
        .open(&input, OpenOptions::default())
        .unwrap();
    let form = document
        .set_form_field_value(FormValueMutationRequest {
            field_name: "dup".into(),
            value: "second updated".into(),
            match_index: 1,
        })
        .unwrap();
    fs::write(&rust_form, form.bytes).unwrap();
    run_go(
        oracle,
        root,
        [
            "form",
            "set",
            input_path.to_str().unwrap(),
            "--field",
            "dup",
            "--value",
            "second updated",
            "--match-index",
            "1",
            "-o",
            go_form.to_str().unwrap(),
            "--json",
        ],
    );
    for output in [&rust_form, &go_form] {
        let bytes = fs::read(output).unwrap();
        let reopened = PdfEngine::default()
            .open(&bytes, OpenOptions::default())
            .unwrap();
        assert_eq!(
            list_form_fields(&reopened).unwrap()[1].value.as_deref(),
            Some("second updated")
        );
        let go = go_json(
            oracle,
            root,
            [
                "form",
                "list",
                output.to_str().unwrap(),
                "--format",
                "pdf",
                "--json",
            ],
        );
        assert_eq!(go["count"], 2);
    }

    let annotation = document
        .set_annotation_contents(AnnotationContentsMutationRequest {
            annotation_index: 0,
            contents: "updated note".into(),
        })
        .unwrap();
    fs::write(&rust_annotation, annotation.bytes).unwrap();
    run_go(
        oracle,
        root,
        [
            "annot",
            "set-contents",
            input_path.to_str().unwrap(),
            "--index",
            "0",
            "--contents",
            "updated note",
            "-o",
            go_annotation.to_str().unwrap(),
            "--json",
        ],
    );
    for output in [&rust_annotation, &go_annotation] {
        let bytes = fs::read(output).unwrap();
        let reopened = PdfEngine::default()
            .open(&bytes, OpenOptions::default())
            .unwrap();
        assert_eq!(
            list_annotations(&reopened).unwrap()[0].contents.as_deref(),
            Some("updated note")
        );
        let go = go_json(
            oracle,
            root,
            [
                "annot",
                "list",
                output.to_str().unwrap(),
                "--format",
                "pdf",
                "--json",
            ],
        );
        assert_eq!(go["count"], 1);
    }

    for path in [
        input_path,
        rust_form,
        go_form,
        rust_annotation,
        go_annotation,
    ] {
        let _ = fs::remove_file(path);
    }
}

fn compare_edit(
    oracle: &Path,
    root: &Path,
    input: &Path,
    document: &PdfDocument,
    file: &str,
    edit: EditVector,
) {
    let rust_bytes = rust_edit(document, &edit.old, &edit.replacement);
    let base = format!("binas-differential-{}-{}", std::process::id(), file);
    let rust_output = std::env::temp_dir().join(format!("rust-{base}"));
    let go_output = std::env::temp_dir().join(format!("go-{base}"));
    fs::write(&rust_output, rust_bytes).unwrap();
    let result = Command::new(oracle)
        .args([
            "edit",
            input.to_str().unwrap(),
            "--format",
            "pdf",
            "--kind",
            "pdf.content.text_show",
            "--text",
            &edit.old,
            "--replace",
            &edit.replacement,
            "-o",
            go_output.to_str().unwrap(),
            "--json",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Go edit failed for {file}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    for output in [&rust_output, &go_output] {
        let old = go_query_count(oracle, root, output, &edit.old);
        let replacement = go_query_count(oracle, root, output, &edit.replacement);
        assert_eq!(
            (old, replacement),
            (0, 1),
            "Go reparse of {}",
            output.display()
        );
        let bytes = fs::read(output).unwrap();
        let reopened = PdfEngine::default()
            .open(&bytes, OpenOptions::default())
            .unwrap_or_else(|error| panic!("Rust reparse of {} failed: {error}", output.display()));
        assert_eq!(reopened.query_text_all(&edit.old).unwrap().len(), 0);
        assert_eq!(reopened.query_text_all(&edit.replacement).unwrap().len(), 1);
    }
    let _ = fs::remove_file(rust_output);
    let _ = fs::remove_file(go_output);
}

fn rust_edit(document: &PdfDocument, old: &str, replacement: &str) -> Vec<u8> {
    if document.query_text(old, 0).unwrap().font_name.is_some() {
        return document
            .font_text_edit(FontTextEditRequest {
                old_text: old.into(),
                replacement: replacement.into(),
                match_index: 0,
            })
            .unwrap()
            .bytes;
    }
    match document.surgical_text_edit(SurgicalTextEditRequest {
        old_text: old.into(),
        replacement: replacement.into(),
        match_index: 0,
    }) {
        Ok(outcome) => outcome.bytes,
        Err(error) if error.code == PdfErrorCode::UnsafeRewrite => {
            match document.incremental_text_edit(IncrementalTextEditRequest {
                old_text: old.into(),
                replacement: replacement.into(),
                match_index: 0,
            }) {
                Ok(outcome) => outcome.bytes,
                Err(error) if error.code == PdfErrorCode::UnsafeRewrite => {
                    document
                        .filtered_text_edit(FilteredTextEditRequest {
                            old_text: old.into(),
                            replacement: replacement.into(),
                            match_index: 0,
                        })
                        .unwrap()
                        .bytes
                }
                Err(error) => panic!("incremental edit failed: {error}"),
            }
        }
        Err(error) => panic!("surgical edit failed: {error}"),
    }
}

fn go_query_count(oracle: &Path, root: &Path, input: &Path, text: &str) -> usize {
    go_json(
        oracle,
        root,
        [
            "query",
            input.to_str().unwrap(),
            "--format",
            "pdf",
            "--kind",
            "pdf.content.text_show",
            "--text",
            text,
            "--json",
        ],
    )["count"]
        .as_u64()
        .unwrap() as usize
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn build_oracle(root: &Path) -> PathBuf {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let output =
        std::env::temp_dir().join(format!("binas-go-oracle-{}{}", std::process::id(), suffix));
    let result = Command::new("go")
        .args(["build", "-o", output.to_str().unwrap(), "./cmd/binas"])
        .current_dir(root)
        .output()
        .expect("Go is required while the frozen oracle remains a cutover gate");
    assert!(
        result.status.success(),
        "Go oracle build failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    output
}

fn go_json<const N: usize>(oracle: &Path, root: &Path, args: [&str; N]) -> serde_json::Value {
    let result = run_go(oracle, root, args);
    serde_json::from_slice(&result.stdout).unwrap()
}

fn run_go<const N: usize>(oracle: &Path, root: &Path, args: [&str; N]) -> std::process::Output {
    let result = Command::new(oracle)
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "Go oracle failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

fn interactive_pdf() -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /Annots [7 0 R] >>",
        "<< /NeedAppearances true /Fields [5 0 R 6 0 R] >>",
        "<< /T (dup) /FT /Tx /V (one) >>",
        "<< /T (dup) /FT /Tx /V (two) >>",
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] /Contents (old) >>",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

#[test]
fn error_code_name_used_by_the_manifest_is_stable() {
    assert_eq!(PdfErrorCode::InvalidSyntax.as_str(), "invalid_syntax");
}
