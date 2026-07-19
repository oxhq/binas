use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

#[derive(Deserialize)]
struct Manifest {
    version: u32,
    expected_tests: usize,
    categories: Vec<Category>,
}

#[derive(Deserialize)]
struct Category {
    name: String,
    package: String,
    expected_tests: usize,
    tests: Vec<String>,
    coverage: String,
    evidence: String,
    limitation: String,
}

#[derive(Deserialize)]
struct GoEvent {
    #[serde(rename = "Action")]
    action: String,
    #[serde(rename = "Package")]
    package: Option<String>,
    #[serde(rename = "Output")]
    output: Option<String>,
}

#[test]
fn live_go_inventory_is_categorized_without_one_to_one_port_claims() {
    let root = workspace_root();
    let has_go_source = root.join("go.mod").is_file();
    let manifest: Manifest = serde_json::from_slice(
        &fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/go-test-map.json"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(manifest.version, 2);

    let mut declared = BTreeMap::new();
    let mut expected_counts = BTreeMap::new();
    for category in &manifest.categories {
        assert!(!category.name.is_empty());
        assert!(!category.coverage.is_empty());
        assert!(!category.limitation.is_empty());
        assert_eq!(category.tests.len(), category.expected_tests);
        if has_go_source {
            assert!(
                root.join(&category.evidence).exists(),
                "missing {}",
                category.evidence
            );
        }
        assert!(
            expected_counts
                .insert(category.name.as_str(), category.expected_tests)
                .is_none(),
            "duplicate category {}",
            category.name
        );
        for test in &category.tests {
            assert!(test.starts_with("Test"));
            assert!(
                declared
                    .insert(
                        (category.package.as_str(), test.as_str()),
                        category.name.as_str()
                    )
                    .is_none(),
                "duplicate Go inventory entry {}:{}",
                category.package,
                test
            );
        }
    }
    assert_eq!(declared.len(), manifest.expected_tests);
    assert!(manifest.categories.iter().any(|category| {
        category.coverage == "differential_vector" && category.expected_tests > 0
    }));
    if !has_go_source {
        return;
    }

    let result = Command::new("go")
        .args(["test", "-json", "-list", "^Test", "./..."])
        .current_dir(&root)
        .output()
        .expect("Go is required while the frozen oracle remains a cutover gate");
    assert!(
        result.status.success(),
        "Go test inventory failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let observed = String::from_utf8(result.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<GoEvent>(line).ok())
        .filter_map(|event| {
            let output = event.output?.trim().to_owned();
            (event.action == "output" && output.starts_with("Test"))
                .then(|| (event.package.unwrap(), output))
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        observed.len(),
        manifest.expected_tests,
        "Go test inventory changed; update the categorized inventory intentionally"
    );

    let mut actual_counts = BTreeMap::new();
    for (package, test) in observed {
        let category = declared
            .get(&(package.as_str(), test.as_str()))
            .unwrap_or_else(|| panic!("uncategorized Go test {package}:{test}"));
        *actual_counts.entry(*category).or_insert(0_usize) += 1;
    }
    assert_eq!(actual_counts, expected_counts);
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}
