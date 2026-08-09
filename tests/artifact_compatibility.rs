use rove_core::{CallId, ToolExecutionMetadata, ToolResult as RuntimeToolResult};
use rove_models::{
    AssistantTurn, InternalCallId, Message, StopReason, ToolCall, ToolCallRef, ToolResultStatus,
    Usage, WireCallReference,
};
use rove_runtime::context::ContextManager;
use rove_runtime::events::StreamEvent;
use rove_runtime::session::SessionEntry;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::report::RunReport;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{JobId, RunId, SessionId, TaskState};

#[test]
fn pre_lifecycle_task_state_fixture_keeps_additive_defaults() {
    let state: TaskState = serde_json::from_str(include_str!(
        "fixtures/artifacts/pre-lifecycle-task-state.json"
    ))
    .unwrap();

    assert!(state.checkpoint.is_none());
    assert!(state.runtime_identity.is_none());
    assert!(state.step_ledger.is_empty());
}

#[test]
fn pre_lifecycle_report_fixture_keeps_additive_defaults() {
    let report: RunReport =
        serde_json::from_str(include_str!("fixtures/artifacts/pre-lifecycle-report.json")).unwrap();

    assert!(report.step_records.is_empty());
    assert!(report.plan_decisions.is_empty());
    assert!(report.plan_revisions.is_empty());
}

#[test]
fn pre_lifecycle_trace_fixture_remains_readable() {
    let events = include_str!("fixtures/artifacts/pre-lifecycle-trace.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<StreamEvent>(line).unwrap())
        .collect::<Vec<_>>();

    assert!(matches!(
        &events[0],
        StreamEvent::PlanCreated {
            identity,
            plan_revision: None,
            ..
        } if !identity.is_complete()
    ));
    assert!(matches!(
        &events[1],
        StreamEvent::PlanStepStarted { attempt, .. } if !attempt.is_complete()
    ));
}

#[test]
fn additive_message_identity_and_result_status_round_trip() {
    let message = Message::tool_with_status(
        "permission denied",
        Some("wire-call-1".to_string()),
        Some(InternalCallId::new("internal-call-1").unwrap()),
        Some("write_file".to_string()),
        ToolResultStatus::Rejected,
    );
    let encoded = serde_json::to_string(&message).unwrap();
    let decoded: Message = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, message);
    assert_eq!(decoded.tool_result_status, Some(ToolResultStatus::Rejected));
}

#[tokio::test]
async fn canonical_session_persists_restarts_and_reprojects_provider_identity_atomically() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let runtime_call_1 = CallId::new();
    let runtime_call_2 = CallId::new();
    let internal_1 = InternalCallId::new("canonical-call-1").unwrap();
    let internal_2 = InternalCallId::new("canonical-call-2").unwrap();
    let wire_1 = "openai-wire-1";
    let wire_2 = "openai-wire-2";
    let assistant_turn = AssistantTurn {
        tool_calls: vec![
            ToolCall {
                internal_call_id: internal_1.clone(),
                name: "echo".to_string(),
                arguments: serde_json::json!({"message":"one"}),
                wire_reference: Some(WireCallReference::new("openai-completions", wire_1).unwrap()),
            },
            ToolCall {
                internal_call_id: internal_2.clone(),
                name: "echo".to_string(),
                arguments: serde_json::json!({"message":"two"}),
                wire_reference: Some(WireCallReference::new("openai-completions", wire_2).unwrap()),
            },
        ],
        stop_reason: StopReason::ToolUse,
        ..AssistantTurn::default()
    };
    let tool_refs = vec![
        ToolCallRef {
            id: wire_1.to_string(),
            name: "echo".to_string(),
            args: serde_json::json!({"message":"one"}),
        },
        ToolCallRef {
            id: wire_2.to_string(),
            name: "echo".to_string(),
            args: serde_json::json!({"message":"two"}),
        },
    ];
    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        "run both".to_string(),
        None,
        None,
    );

    recorder
        .record_event(
            &StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "run both".to_string(),
            },
            &store,
        )
        .await;
    recorder
        .record_event(
            &StreamEvent::LlmMessage {
                full: String::new(),
                usage: Usage::default(),
                tool_calls: tool_refs,
                assistant_turn: Some(Box::new(assistant_turn)),
            },
            &store,
        )
        .await;
    for (call_id, wire, message) in [
        (runtime_call_1, wire_1, "one"),
        (runtime_call_2, wire_2, "two"),
    ] {
        recorder
            .record_event(
                &StreamEvent::ToolCallStarted {
                    call_id,
                    tool_use_id: Some(wire.to_string()),
                    name: "echo".to_string(),
                    args: serde_json::json!({"message":message}),
                },
                &store,
            )
            .await;
    }
    for (call_id, output) in [(runtime_call_1, "one"), (runtime_call_2, "two")] {
        recorder
            .record_event(
                &StreamEvent::ToolCallCompleted {
                    call_id,
                    result: RuntimeToolResult {
                        call_id,
                        output: output.to_string(),
                        mutations: Vec::new(),
                        metadata: ToolExecutionMetadata::default(),
                        envelope: None,
                    },
                },
                &store,
            )
            .await;
    }

    let state = store.load_task_state(run_id).await.unwrap();
    let checkpoint = state.checkpoint.as_ref().unwrap();
    let session = checkpoint
        .session
        .as_ref()
        .expect("new writer stores session");
    assert_eq!(session.schema_version, 1);
    let assistant = session
        .entries
        .iter()
        .find_map(|entry| match entry {
            SessionEntry::Assistant { turn, .. } => Some(turn),
            _ => None,
        })
        .unwrap();
    assert_eq!(assistant.tool_calls[0].internal_call_id, internal_1);
    assert_eq!(assistant.tool_calls[1].internal_call_id, internal_2);
    assert_eq!(
        assistant.tool_calls[0]
            .wire_reference
            .as_ref()
            .unwrap()
            .value,
        wire_1
    );
    assert!(
        state
            .history
            .iter()
            .filter(|message| message.role == rove_models::Role::Tool)
            .all(|message| message.internal_call_id.is_some()
                && message.tool_name.as_deref() == Some("echo")
                && message.tool_result_status == Some(ToolResultStatus::Ok))
    );

    let openai = session.messages_for_provider("openai-completions").unwrap();
    let anthropic = session.messages_for_provider("anthropic-messages").unwrap();
    let ollama = session.messages_for_provider("ollama-chat").unwrap();
    assert_eq!(openai[1].tool_calls[0].id, wire_1);
    assert_ne!(anthropic[1].tool_calls[0].id, wire_1);
    for projected in [&anthropic, &ollama] {
        assert_eq!(projected[1].tool_calls.len(), 2);
        assert_eq!(
            projected[1].tool_calls[0].id,
            projected[2].tool_call_id.clone().unwrap()
        );
        assert_eq!(
            projected[1].tool_calls[1].id,
            projected[3].tool_call_id.clone().unwrap()
        );
    }

    let trimmed =
        ContextManager::with_max_history("system".to_string(), 2).build("next", &[], &anthropic);
    assert!(
        trimmed
            .iter()
            .all(|message| message.role != rove_models::Role::Tool)
    );
    let atomic =
        ContextManager::with_max_history("system".to_string(), 3).build("next", &[], &anthropic);
    let atomic_round = atomic
        .iter()
        .filter(|message| !message.tool_calls.is_empty() || message.role == rove_models::Role::Tool)
        .collect::<Vec<_>>();
    assert_eq!(atomic_round.len(), 3);

    let resumed_run = RunId::new();
    let resumed_job = JobId::new();
    let mut resumed = RunArtifactRecorder::new(
        session_id,
        resumed_job,
        resumed_run,
        "continue after restart".to_string(),
        Some(&state),
        None,
    );
    resumed
        .record_event(
            &StreamEvent::RunStarted {
                run_id: resumed_run,
                job_id: resumed_job,
                user_message: "continue after restart".to_string(),
            },
            &store,
        )
        .await;
    let restarted_state = store.load_task_state(resumed_run).await.unwrap();
    let restarted_session = restarted_state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.session.as_ref())
        .unwrap();
    assert_eq!(
        restarted_session
            .entries
            .iter()
            .filter(|entry| matches!(entry, SessionEntry::ToolResult { .. }))
            .count(),
        2
    );
    assert!(matches!(
        restarted_session.entries.last(),
        Some(SessionEntry::User { content, .. })
            if content.iter().any(|block| block.text_value() == Some("continue after restart"))
    ));
}

#[tokio::test]
async fn legacy_message_only_state_dual_reads_then_writes_one_canonical_session() {
    let session_id = SessionId::new();
    let legacy = TaskState {
        schema_version: 1,
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "legacy".to_string(),
        step: 1,
        history: vec![Message::user("legacy"), Message::assistant("old answer")],
        summary: None,
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
        execution_lifecycle: Default::default(),
    };
    let legacy_json = serde_json::to_string(&legacy).unwrap();
    assert!(!legacy_json.contains("\"session\""));
    let decoded: TaskState = serde_json::from_str(&legacy_json).unwrap();

    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let run_id = RunId::new();
    let job_id = JobId::new();
    let mut recorder = RunArtifactRecorder::new(
        session_id,
        job_id,
        run_id,
        "continue".to_string(),
        Some(&decoded),
        None,
    );
    recorder
        .record_event(
            &StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "continue".to_string(),
            },
            &store,
        )
        .await;

    let migrated = store.load_task_state(run_id).await.unwrap();
    let migrated_session = migrated
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.session.as_ref())
        .expect("dual-read migration writes canonical session");
    assert_eq!(migrated_session.schema_version, 1);
    assert_eq!(migrated.history.len(), 3);
    assert_eq!(
        migrated_session
            .entries
            .iter()
            .filter(|entry| matches!(entry, SessionEntry::Legacy { .. }))
            .count(),
        2
    );
}
