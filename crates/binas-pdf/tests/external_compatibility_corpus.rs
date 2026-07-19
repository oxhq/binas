use std::{
    collections::BTreeSet,
    env, fs,
    path::{Component, Path, PathBuf},
};

use binas_pdf::{OpenOptions, PdfEngine};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Manifest {
    version: u32,
    corpus_root: String,
    fixture_count: usize,
    coverage_status: String,
    fixtures: Vec<Fixture>,
}

#[derive(Deserialize)]
struct Fixture {
    id: String,
    path: String,
    sha256: String,
    source: String,
    license: String,
    roles: Vec<String>,
    parse: ParseExpectation,
    expected_pages: Option<usize>,
    expected_error_code: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ParseExpectation {
    Opens,
    Rejects,
}

#[test]
fn validates_an_externally_supplied_licensed_compatibility_corpus() {
    let required = env::var_os("BINAS_REQUIRE_COMPAT_CORPUS").is_some();
    let Some(manifest_path) = env::var_os("BINAS_COMPAT_CORPUS_MANIFEST") else {
        assert!(
            !required,
            "BINAS_REQUIRE_COMPAT_CORPUS requires BINAS_COMPAT_CORPUS_MANIFEST"
        );
        return;
    };

    let manifest_path = PathBuf::from(manifest_path);
    let manifest_directory = manifest_path
        .parent()
        .expect("compatibility corpus manifest must have a parent directory");
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(&manifest_path).expect("read compatibility corpus manifest"),
    )
    .expect("parse compatibility corpus manifest");

    assert_eq!(manifest.version, 1);
    assert_eq!(
        manifest.coverage_status,
        "licensed_compatibility_and_adversarial"
    );
    assert!(!manifest.fixtures.is_empty(), "corpus manifest is empty");
    assert_eq!(manifest.fixture_count, manifest.fixtures.len());

    let corpus_root = resolve_relative(manifest_directory, &manifest.corpus_root, "corpus_root");
    assert!(corpus_root.is_dir(), "corpus_root must be a directory");
    let canonical_root = fs::canonicalize(&corpus_root).expect("canonicalize corpus_root");
    let engine = PdfEngine::default();
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();

    for fixture in manifest.fixtures {
        assert!(!fixture.id.trim().is_empty(), "fixture id is empty");
        assert!(
            ids.insert(fixture.id.clone()),
            "duplicate fixture id: {}",
            fixture.id
        );
        assert!(
            !fixture.source.trim().is_empty(),
            "{} has no source",
            fixture.id
        );
        assert!(
            !fixture.license.trim().is_empty(),
            "{} has no license",
            fixture.id
        );
        assert!(
            !fixture.roles.is_empty() && fixture.roles.iter().all(|role| !role.trim().is_empty()),
            "{} needs at least one non-empty compatibility or adversarial role",
            fixture.id
        );
        assert!(
            fixture.sha256.len() == 64
                && fixture.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
                && fixture
                    .sha256
                    .bytes()
                    .all(|byte| !byte.is_ascii_uppercase()),
            "{} has an invalid lowercase SHA-256",
            fixture.id
        );
        assert!(
            fixture.path.ends_with(".pdf"),
            "{} is not a PDF",
            fixture.id
        );
        assert!(
            paths.insert(fixture.path.clone()),
            "duplicate fixture path: {}",
            fixture.path
        );

        let path = resolve_relative(&corpus_root, &fixture.path, "fixture path");
        let canonical_path = fs::canonicalize(&path).expect("canonicalize fixture path");
        assert!(
            canonical_path.starts_with(&canonical_root),
            "{} escapes corpus_root",
            fixture.id
        );
        let bytes = fs::read(&canonical_path).expect("read fixture");
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            fixture.sha256,
            "fixture digest changed: {}",
            fixture.id
        );

        match fixture.parse {
            ParseExpectation::Opens => {
                let document = engine
                    .open(&bytes, OpenOptions::default())
                    .unwrap_or_else(|error| panic!("{} should open: {error}", fixture.id));
                let inspection = document.inspect().expect("inspect opened fixture");
                let validation = document.validate().expect("validate opened fixture");
                assert!(validation.valid, "{} did not validate", fixture.id);
                if let Some(expected_pages) = fixture.expected_pages {
                    assert_eq!(
                        inspection.page_count, expected_pages,
                        "{} page count",
                        fixture.id
                    );
                }
                assert!(
                    fixture.expected_error_code.is_none(),
                    "{} opens but declares an error code",
                    fixture.id
                );
            }
            ParseExpectation::Rejects => {
                let error = engine
                    .open(&bytes, OpenOptions::default())
                    .expect_err("fixture marked rejects unexpectedly opened");
                let expected = fixture
                    .expected_error_code
                    .as_deref()
                    .expect("rejected fixture needs expected_error_code");
                assert_eq!(
                    error.code.as_str(),
                    expected,
                    "{} rejection code",
                    fixture.id
                );
                assert!(
                    fixture.expected_pages.is_none(),
                    "{} rejects but declares a page count",
                    fixture.id
                );
            }
        }
    }
}

fn resolve_relative(root: &Path, value: &str, label: &str) -> PathBuf {
    let path = Path::new(value);
    assert!(!value.trim().is_empty(), "{label} is empty");
    assert!(!path.is_absolute(), "{label} must be relative");
    assert!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir)),
        "{label} contains an unsafe path component"
    );
    root.join(path)
}
