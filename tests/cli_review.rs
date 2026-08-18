use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use serde_json::Value;

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn rove_bin() -> PathBuf {
    static CURRENT_ROVE_BIN: OnceLock<PathBuf> = OnceLock::new();
    CURRENT_ROVE_BIN
        .get_or_init(|| {
            if let Ok(path) = std::env::var("CARGO_BIN_EXE_rove") {
                return PathBuf::from(path);
            }
            let root = workspace_root();
            let exe = if cfg!(windows) { "rove.exe" } else { "rove" };
            let candidate = root.join("target/debug").join(exe);
            // Cargo does not always expose CARGO_BIN_EXE_rove to a test in the
            // separate integration package. Rebuild once so the process test
            // never executes a stale binary left by an earlier command.
            let status = Command::new(env!("CARGO"))
                .args(["build", "-p", "rove-cli", "--bin", "rove"])
                .current_dir(&root)
                .status()
                .expect("failed to spawn cargo build for rove-cli");
            assert!(
                status.success(),
                "cargo build -p rove-cli --bin rove failed"
            );
            assert!(
                candidate.exists(),
                "expected built CLI binary at {}",
                candidate.display()
            );
            candidate
        })
        .clone()
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .expect("git should start");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_tree_omits(root: &std::path::Path, needle: &str) {
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            assert_tree_omits(&path, needle);
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes
                .windows(needle.len())
                .any(|window| window == needle.as_bytes()),
            "{} leaked Review source text",
            path.display()
        );
    }
}

#[test]
fn review_cli_persists_sanitized_artifacts_without_workspace_state() {
    let repo = tempfile::TempDir::new().unwrap();
    let user_config = tempfile::TempDir::new().unwrap();
    git(repo.path(), &["init", "-q"]);
    git(
        repo.path(),
        &["config", "user.email", "review@example.invalid"],
    );
    git(repo.path(), &["config", "user.name", "Review Test"]);
    std::fs::write(repo.path().join("tracked.txt"), "before\n").unwrap();
    git(repo.path(), &["add", "tracked.txt"]);
    git(repo.path(), &["commit", "-qm", "initial"]);
    let marker = "CLI_REVIEW_SOURCE_MARKER_8d71";
    std::fs::write(repo.path().join("tracked.txt"), format!("{marker}\n")).unwrap();

    let output = Command::new(rove_bin())
        .args([
            "--cwd",
            repo.path().to_str().unwrap(),
            "--model",
            "fake",
            "review",
            "--format",
            "json",
        ])
        .env("ROVE_CONFIG_ROOT", user_config.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["conclusion"], "pass");
    assert!(
        !output
            .stdout
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
    );
    assert!(!repo.path().join(".rove").exists());

    let review_id = result["review_id"].as_str().unwrap();
    let run_id = result["run_id"].as_str().unwrap();
    let state_root = std::env::temp_dir()
        .join("rove-review-state")
        .join(review_id);
    let run_dir = state_root.join("runs").join(run_id);
    assert!(state_root.join("target_snapshot.json").is_file());
    assert!(
        std::fs::read_to_string(state_root.join("target_snapshot.json"))
            .unwrap()
            .contains(marker)
    );
    for artifact in ["trace.jsonl", "task_state.json", "report.json"] {
        let text = std::fs::read_to_string(run_dir.join(artifact)).unwrap();
        assert!(!text.contains(marker), "{artifact} leaked source text");
    }
    assert_tree_omits(&run_dir, marker);
}

#[test]
fn review_cli_reports_provider_unavailable_with_exit_code_two() {
    let repo = tempfile::TempDir::new().unwrap();
    let user_config = tempfile::TempDir::new().unwrap();
    git(repo.path(), &["init", "-q"]);
    std::fs::write(repo.path().join("tracked.txt"), "initial\n").unwrap();
    git(repo.path(), &["add", "tracked.txt"]);
    git(
        repo.path(),
        &["config", "user.email", "review@example.invalid"],
    );
    git(repo.path(), &["config", "user.name", "Review Test"]);
    git(repo.path(), &["commit", "-qm", "initial"]);

    let output = Command::new(rove_bin())
        .args([
            "--cwd",
            repo.path().to_str().unwrap(),
            "--model",
            "external-model",
            "review",
            "--format",
            "json",
        ])
        .env("ROVE_CONFIG_ROOT", user_config.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["conclusion"], "unavailable");
    assert_eq!(
        result["warnings"],
        serde_json::json!(["provider_unavailable", "review_findings_not_submitted"])
    );
    assert!(!repo.path().join(".rove").exists());
}

#[test]
fn review_cli_reports_target_unavailable_with_exit_code_two() {
    let folder = tempfile::TempDir::new().unwrap();
    let user_config = tempfile::TempDir::new().unwrap();

    let output = Command::new(rove_bin())
        .args([
            "--cwd",
            folder.path().to_str().unwrap(),
            "--model",
            "fake",
            "review",
            "--format",
            "json",
        ])
        .env("ROVE_CONFIG_ROOT", user_config.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["conclusion"], "unavailable");
    assert_eq!(
        result["warnings"],
        serde_json::json!(["review_target_unavailable"])
    );
    assert_eq!(result["target"]["workspace_kind"], "folder");
    assert!(!folder.path().join(".rove").exists());
}
