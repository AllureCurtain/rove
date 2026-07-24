use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn workspace_path(rel: impl AsRef<Path>) -> PathBuf {
    workspace_root().join(rel)
}

use rove_runtime::workspace::{Workspace, WorkspaceKind};

#[test]
fn browser_and_desktop_are_future_specs_not_runtime_workspace_stubs() {
    let variants = serde_json::to_value([
        WorkspaceKind::Folder,
        WorkspaceKind::Repo,
        WorkspaceKind::Task,
    ])
    .unwrap();

    assert_eq!(variants, serde_json::json!(["folder", "repo", "task"]));
    assert!(!format!("{variants:?}").contains("browser"));
    assert!(!format!("{variants:?}").contains("desktop"));
    assert!(workspace_path("docs/runtime/browser-workspace-spec.md").is_file());
    assert!(workspace_path("docs/runtime/desktop-workspace-spec.md").is_file());
}

#[test]
fn folder_and_repo_detection_remain_unchanged() {
    let folder = tempfile::TempDir::new().unwrap();
    let folder_ws = Workspace::detect(folder.path()).unwrap();
    assert_eq!(folder_ws.kind, WorkspaceKind::Folder);

    let repo = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(repo.path().join(".git")).unwrap();
    let nested = repo.path().join("src");
    std::fs::create_dir(&nested).unwrap();
    let repo_ws = Workspace::detect(&nested).unwrap();

    assert_eq!(repo_ws.kind, WorkspaceKind::Repo);
    assert_eq!(repo_ws.root, repo.path().canonicalize().unwrap());
}

#[test]
fn task_workspace_docs_cover_cli_api_lifecycle_and_cleanup() {
    let guide =
        std::fs::read_to_string(workspace_path("docs/runtime/implementation-guide.md")).unwrap();

    assert!(guide.contains("--task-workspace"));
    assert!(guide.contains("--task-base"));
    assert!(guide.contains(r#""workspace": {"#));
    assert!(guide.contains("Task workspace lifecycle"));
    assert!(guide.contains("delete the task workspace directory"));
}
