use rove::core::types::SessionId;
use rove::core::workspace::{Workspace, WorkspaceKind};
use rove::memory::layered::load_prompt_memory_sync;

fn test_workspace(root: &std::path::Path) -> Workspace {
    Workspace {
        root: root.to_path_buf(),
        kind: WorkspaceKind::Folder,
        state_dir: root.join(".rove"),
    }
}

#[test]
fn prompt_memory_loads_durable_index_and_session_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = test_workspace(tmp.path());
    let session_id = SessionId::new();
    let memory_dir = workspace.state_dir.join("memory");
    let sessions_dir = memory_dir.join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::write(memory_dir.join("MEMORY.md"), "durable project facts").unwrap();
    std::fs::write(
        sessions_dir.join(format!("{session_id}.md")),
        "session preference",
    )
    .unwrap();

    let memory = load_prompt_memory_sync(&workspace, session_id, None).unwrap();

    assert_eq!(
        memory.durable_index.as_deref(),
        Some("durable project facts")
    );
    assert_eq!(
        memory.session_summary.as_deref(),
        Some("session preference")
    );
}

#[test]
fn prompt_memory_resume_summary_takes_precedence_over_session_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = test_workspace(tmp.path());
    let session_id = SessionId::new();
    let sessions_dir = workspace.state_dir.join("memory").join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::write(
        sessions_dir.join(format!("{session_id}.md")),
        "stale session preference",
    )
    .unwrap();

    let memory =
        load_prompt_memory_sync(&workspace, session_id, Some("resume summary wins")).unwrap();

    assert_eq!(memory.durable_index, None);
    assert_eq!(
        memory.session_summary.as_deref(),
        Some("resume summary wins")
    );
}
