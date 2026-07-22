use rove_core::{ToolDescriptor, ToolError};
use rove_runtime::boundary::{
    check_tool_allowed, resolve_workspace_read_path, resolve_workspace_write_path,
};
use rove_runtime::types::{ApprovalPolicy, JobId, RunId, SessionId};
use rove_runtime::{Workspace, WorkspaceKind};

#[test]
fn ids_and_workspace_are_available_from_the_runtime_crate() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();

    assert_eq!(workspace.kind, WorkspaceKind::Folder);
    assert!(!SessionId::new().to_string().is_empty());
    assert!(!JobId::new().to_string().is_empty());
    assert!(!RunId::new().to_string().is_empty());
}

#[test]
fn workspace_paths_and_destructive_policy_remain_fail_closed() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::write(workspace.root.join("read.txt"), "runtime boundary").unwrap();

    assert_eq!(
        resolve_workspace_read_path(&workspace.root, "read.txt").unwrap(),
        workspace.root.join("read.txt")
    );
    assert_eq!(
        resolve_workspace_write_path(&workspace.root, "new.txt").unwrap(),
        workspace.root.join("new.txt")
    );
    assert!(matches!(
        resolve_workspace_write_path(&workspace.root, "../escape.txt"),
        Err(ToolError::PermissionDenied { .. })
    ));

    let descriptor = ToolDescriptor {
        name: "write".to_string(),
        description: "Write a file".to_string(),
        parameters: serde_json::json!({"type": "object"}),
        destructive: true,
        parallel_safe: false,
        capability: None,
    };
    assert!(matches!(
        check_tool_allowed(&descriptor, ApprovalPolicy::Never),
        Err(ToolError::PermissionDenied { .. })
    ));
}
