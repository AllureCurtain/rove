use rove_runtime::events::StreamEvent;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{JobId, RunId, SessionId, TaskState};
use rove_runtime::{Workspace, WorkspaceKind};

#[tokio::test]
async fn runtime_state_store_round_trips_task_state_and_indexed_trace() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let store = StateStore::new(&workspace.state_dir);
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let handle = store.start_run(session_id, job_id, run_id).unwrap();

    handle
        .trace_writer
        .append(&StreamEvent::RunStarted {
            run_id,
            job_id,
            user_message: "inspect state".to_string(),
        })
        .unwrap();

    let state = TaskState {
        schema_version: 1,
        session_id,
        job_id,
        run_id,
        goal: "inspect state".to_string(),
        step: 1,
        history: vec![rove_models::Message::user("inspect state")],
        summary: Some("state round trip".to_string()),
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
        execution_lifecycle: Default::default(),
    };
    store.write_task_state(&state).await.unwrap();

    let loaded = store.load_task_state(run_id).await.unwrap();
    assert_eq!(loaded.goal, state.goal);
    assert_eq!(loaded.session_id, session_id);

    let events = store.index.event_records(run_id).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_name, "run_started");
    assert_eq!(events[0].run_id, run_id);
    assert_eq!(workspace.kind, WorkspaceKind::Folder);
}
