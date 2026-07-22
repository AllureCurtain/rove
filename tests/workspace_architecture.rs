use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

#[test]
fn local_package_dependencies_follow_the_modular_workspace_direction() {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo metadata should run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let packages = metadata["packages"].as_array().unwrap();
    let local_names: BTreeSet<_> = packages
        .iter()
        .filter_map(|package| package["name"].as_str())
        .map(str::to_string)
        .collect();
    let mut violations = BTreeMap::<String, Vec<String>>::new();
    let mut local_dependencies = BTreeMap::<String, BTreeSet<String>>::new();

    for package in packages {
        let package_name = package["name"].as_str().unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            let dependency_name = dependency["name"].as_str().unwrap();
            if local_names.contains(dependency_name) {
                local_dependencies
                    .entry(package_name.to_string())
                    .or_default()
                    .insert(dependency_name.to_string());
                if !local_dependency_is_allowed(package_name, dependency_name) {
                    violations
                        .entry(package_name.to_string())
                        .or_default()
                        .push(dependency_name.to_string());
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "local package dependency direction violations: {violations:?}"
    );
    assert_eq!(
        local_dependencies
            .get("rove-models")
            .cloned()
            .unwrap_or_default(),
        BTreeSet::new(),
        "rove-models must not depend on another local package"
    );
    assert_eq!(
        local_dependencies
            .get("rove-core")
            .cloned()
            .unwrap_or_default(),
        BTreeSet::from(["rove-models".to_string()]),
        "rove-core must depend only on rove-models among local packages"
    );
    assert_eq!(
        local_dependencies
            .get("rove-runtime")
            .cloned()
            .unwrap_or_default(),
        BTreeSet::from(["rove-core".to_string(), "rove-models".to_string()]),
        "rove-runtime must depend only on rove-models and rove-core among local packages"
    );

    let tree = Command::new(env!("CARGO"))
        .args(["tree", "-p", "rove-core", "--prefix", "none"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree for rove-core should run");
    assert!(
        tree.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&tree.stderr)
    );
    let tree = String::from_utf8(tree.stdout).unwrap();
    for forbidden in ["rusqlite", "axum", "clap", "ratatui", "lancedb"] {
        assert!(
            !tree
                .lines()
                .any(|line| line.split_whitespace().next() == Some(forbidden)),
            "rove-core dependency tree must exclude {forbidden}:\n{tree}"
        );
    }
}

fn local_dependency_is_allowed(package: &str, dependency: &str) -> bool {
    match package {
        // Temporary compatibility facade during physical extraction.
        "rove" => true,
        "rove-models" => false,
        "rove-core" => dependency == "rove-models",
        "rove-runtime" => matches!(dependency, "rove-models" | "rove-core"),
        "rove-app-bootstrap" => {
            matches!(dependency, "rove-models" | "rove-core" | "rove-runtime")
        }
        "rove-bench" => matches!(
            dependency,
            "rove-models" | "rove-core" | "rove-runtime" | "rove-app-bootstrap"
        ),
        "rove-api" => matches!(
            dependency,
            "rove-models" | "rove-core" | "rove-runtime" | "rove-app-bootstrap" | "rove-bench"
        ),
        "rove-cli" => matches!(
            dependency,
            "rove-models" | "rove-core" | "rove-runtime" | "rove-app-bootstrap"
        ),
        "rove-integration-tests" => true,
        _ => false,
    }
}
