use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Manifest {
    version: u32,
    corpus_root: String,
    fixture_count: usize,
    coverage_status: String,
    repository_license_path: String,
    fixtures: Vec<Fixture>,
}

#[derive(Deserialize)]
struct Fixture {
    path: String,
    sha256: String,
    source_status: String,
    license_status: String,
}

#[test]
fn fixture_manifest_freezes_integrity_without_provenance_claims() {
    let root = crate_root();
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/fixture-provenance.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.corpus_root, "tests/fixtures/pdf");
    assert_eq!(
        manifest.coverage_status,
        "unit_boundary_only_not_compatibility_or_adversarial"
    );
    assert!(
        root.join(&manifest.repository_license_path).is_file()
            || workspace_root()
                .join(&manifest.repository_license_path)
                .is_file()
    );
    assert_eq!(manifest.fixtures.len(), manifest.fixture_count);

    let mut declared = BTreeSet::new();
    for fixture in &manifest.fixtures {
        assert!(fixture.path.starts_with("tests/fixtures/pdf/"));
        assert!(!fixture.path.contains('\\'));
        assert!(
            fixture.sha256.len() == 64
                && fixture.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        );
        assert_eq!(fixture.source_status, "unverified");
        assert_eq!(fixture.license_status, "unverified");
        assert!(declared.insert(fixture.path.as_str()));

        let bytes = fs::read(root.join(&fixture.path)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), fixture.sha256);
    }

    let actual = fs::read_dir(root.join(&manifest.corpus_root))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".pdf"))
        .map(|name| format!("{}/{}", manifest.corpus_root, name))
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, declared.into_iter().map(str::to_owned).collect());
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}
