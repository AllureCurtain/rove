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

    for package in packages {
        let package_name = package["name"].as_str().unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            let dependency_name = dependency["name"].as_str().unwrap();
            if local_names.contains(dependency_name)
                && !local_dependency_is_allowed(package_name, dependency_name)
            {
                violations
                    .entry(package_name.to_string())
                    .or_default()
                    .push(dependency_name.to_string());
            }
        }
    }

    assert!(
        violations.is_empty(),
        "local package dependency direction violations: {violations:?}"
    );
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
