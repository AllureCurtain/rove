use rove_runtime::workspace::{Workspace, WorkspaceKind};

#[test]
fn workspace_kinds_exclude_browser_and_desktop() {
    let variants = serde_json::to_value([
        WorkspaceKind::Folder,
        WorkspaceKind::Repo,
        WorkspaceKind::Task,
    ])
    .unwrap();

    assert_eq!(variants, serde_json::json!(["folder", "repo", "task"]));
    assert!(!format!("{variants:?}").contains("browser"));
    assert!(!format!("{variants:?}").contains("desktop"));
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
