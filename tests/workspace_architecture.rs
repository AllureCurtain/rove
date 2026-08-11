use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
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
        "rove-desktop" => dependency == "rove-api",
        "rove-cli" => matches!(
            dependency,
            "rove-models" | "rove-core" | "rove-runtime" | "rove-app-bootstrap"
        ),
        "rove-integration-tests" => true,
        _ => false,
    }
}

#[test]
fn first_party_products_do_not_construct_private_agent_loops() {
    for relative_root in [
        "apps/cli/src",
        "apps/api/src",
        "apps/bench/src",
        "apps/desktop/src",
    ] {
        for path in rust_files(&workspace_root().join(relative_root)) {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            assert!(
                !source.contains("rove_core::Agent") && !source.contains("Agent::new("),
                "{} must assemble the shared runtime Engine instead of a private Agent loop",
                path.display()
            );
        }
    }
}

#[test]
fn runtime_model_turns_use_the_core_normalization_boundary() {
    let adapter_path = workspace_root().join("runtime/src/engine/model_turn.rs");
    let adapter = std::fs::read_to_string(&adapter_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", adapter_path.display()));
    assert!(
        adapter.contains("rove_core::model_turn::run_model_turn"),
        "the durable Runtime model adapter must delegate provider normalization to Core"
    );

    for path in rust_files(&workspace_root().join("runtime/src/engine")) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assert!(
            !source.contains("model.stream("),
            "{} must not bypass the Core model-turn boundary",
            path.display()
        );
    }
}

#[test]
fn embedded_and_durable_execution_share_the_core_agent_kernel() {
    let root = workspace_root();
    for relative in [
        "core/src/agent.rs",
        "runtime/src/engine/run_loop.rs",
        "runtime/src/engine/step_runner.rs",
    ] {
        let path = root.join(relative);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        assert!(
            source.contains("run_agent_kernel"),
            "{relative} must delegate multi-turn coordination to the shared Core kernel"
        );
        assert!(
            !source.contains("match model_turn.action") && !source.contains("match turn.action"),
            "{relative} must not retain a private model-action coordinator"
        );
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("integration-test package should be below the workspace root")
        .to_path_buf()
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
        {
            let path = entry.expect("directory entry should be readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
