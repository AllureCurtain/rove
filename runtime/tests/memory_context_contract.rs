use rove_models::Message;
use rove_runtime::compaction::{COMPACTION_PROMPT_VERSION, StructuredSummary};
use rove_runtime::context::{
    ContextManager, compact_summary_message, durable_memory_message, session_summary_message,
};
use rove_runtime::memory::layered::load_prompt_memory_from_paths_sync;
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::memory::session::write_session_summary_to_dir_sync;
use rove_runtime::{SessionId, Workspace};

#[test]
fn runtime_context_preserves_authority_order_and_compaction_contract() {
    let manager = ContextManager::with_max_history("system policy".to_string(), 1);
    let working_memory = vec![
        durable_memory_message("project fact"),
        session_summary_message("previous outcome"),
    ];
    let history = vec![Message::user("old"), Message::assistant("recent")];

    let built = manager.build_with_checkpoint(
        "current request",
        &working_memory,
        Some("open task"),
        &history,
    );
    let contents: Vec<_> = built
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();

    assert_eq!(
        contents,
        vec![
            "system policy",
            "Durable memory:\nproject fact",
            "Session summary: previous outcome",
            "Compact summary: open task",
            "recent",
            "current request",
        ]
    );
    assert_eq!(built.included_history_messages, 1);
    assert_eq!(built.dropped_history_messages, 1);
    assert_eq!(COMPACTION_PROMPT_VERSION, "rove.compaction.v2");
    assert_eq!(
        StructuredSummary::parse("Goal: continue migration").goal,
        "continue migration"
    );
    assert_eq!(
        compact_summary_message("checkpoint").content,
        "Compact summary: checkpoint"
    );
}

#[test]
fn runtime_memory_paths_and_session_summary_round_trip() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let paths = MemoryPaths::from_workspace(&workspace, 4);
    let session_id = SessionId::new();

    write_session_summary_to_dir_sync(&paths.session_dir, session_id, "verified state").unwrap();
    let prompt_memory =
        load_prompt_memory_from_paths_sync(&paths, session_id, None, "unmatched query").unwrap();

    assert_eq!(
        prompt_memory.session_summary.as_deref(),
        Some("verified state")
    );
    assert!(prompt_memory.durable_index.is_none());
    assert_eq!(paths.recall_limit, 4);
    assert!(paths.session_dir.starts_with(&workspace.state_dir));
    assert!(paths.durable_dir.starts_with(&workspace.state_dir));
}
