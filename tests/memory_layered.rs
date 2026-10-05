use rove_runtime::memory::durable::recall_durable_memory_sync;
use rove_runtime::memory::layered::load_prompt_memory_sync;
use rove_runtime::types::SessionId;
use rove_runtime::workspace::{Workspace, WorkspaceKind};

fn test_workspace(root: &std::path::Path) -> Workspace {
    Workspace {
        root: root.to_path_buf(),
        kind: WorkspaceKind::Folder,
        state_dir: root.join(".rove"),
    }
}

#[test]
fn prompt_memory_loads_relevant_durable_recall_and_session_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = test_workspace(tmp.path());
    let session_id = SessionId::new();
    let memory_dir = workspace.state_dir.join("memory");
    let sessions_dir = memory_dir.join("sessions");
    let topics_dir = memory_dir.join("topics");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&topics_dir).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Project Facts](topics/project-facts.md) - project memory\n- [User Preferences](topics/user-preferences.md) - user memory\n",
    )
    .unwrap();
    std::fs::write(
        topics_dir.join("project-facts.md"),
        "---\ntitle: Project Facts\ntype: project\n---\n\nUse SQLite for the state index.\n",
    )
    .unwrap();
    std::fs::write(
        topics_dir.join("user-preferences.md"),
        "---\ntitle: User Preferences\ntype: user\n---\n\nPrefers quiet output.\n",
    )
    .unwrap();
    std::fs::write(
        sessions_dir.join(format!("{session_id}.md")),
        "session preference",
    )
    .unwrap();

    let memory =
        load_prompt_memory_sync(&workspace, session_id, None, "state index project", 1).unwrap();

    let durable = memory.durable_index.unwrap();
    assert!(durable.contains("Project Facts"));
    assert!(durable.contains("Use SQLite for the state index."));
    assert!(!durable.contains("User Preferences"));
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

    let memory = load_prompt_memory_sync(
        &workspace,
        session_id,
        Some("resume summary wins"),
        "current task",
        8,
    )
    .unwrap();

    assert_eq!(memory.durable_index, None);
    assert_eq!(
        memory.session_summary.as_deref(),
        Some("resume summary wins")
    );
}

#[test]
fn durable_recall_returns_none_when_query_has_no_relevant_topic() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = test_workspace(tmp.path());
    let memory_dir = workspace.state_dir.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Project Facts](topics/project-facts.md) - project memory\n",
    )
    .unwrap();

    let recalled = recall_durable_memory_sync(&workspace, "unrelated weather", 8).unwrap();

    assert_eq!(recalled, None);
}
