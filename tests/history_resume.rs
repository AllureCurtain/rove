//! An explicit trace history stream is
//! sufficient to rebuild model context on resume.
//!
//! The plan's acceptance criterion is that resume no longer needs heuristic
//! classification of UI events to reconstruct what the model saw. This test
//! proves it end to end, with the snapshot deliberately emptied:
//!
//! 1. Run a real engine turn with a `TraceWriter`, so the trace carries
//!    explicit `TraceEntry::History` lines.
//! 2. Throw the durable snapshot history away — simulating a crash before the
//!    snapshot was written — and reconcile from the trace alone.
//! 3. Resume against a recording model and assert it receives the first run's
//!    conversation.
//!
//! If step 2 needed heuristics, step 3 could not reproduce the conversation.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use rove_core::ToolRegistry;
use rove_models::{
    Message, ModelClient, ModelError, ModelEvent, ModelToolSchema, Role, StopReason,
};
use rove_runtime::Workspace;
use rove_runtime::context::manager::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig};
use rove_runtime::session::Session;
use rove_runtime::state::reconcile::reconcile_task_state_with_trace;
use rove_runtime::state::trace::TraceWriter;
use rove_runtime::state::trace_reader::read_trace_content;
use rove_runtime::types::{
    ApprovalPolicy, JobId, PromptCheckpoint, RunId, RunRequest, SessionId, TaskState,
};

/// Emits one line of text and ends the turn.
struct SpeakingModel {
    text: &'static str,
}

#[async_trait]
impl ModelClient for SpeakingModel {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta {
                text: self.text.to_string(),
            }),
            Ok(ModelEvent::StopReason {
                reason: StopReason::EndTurn,
            }),
            Ok(ModelEvent::Done),
        ]))
    }

    fn model_id(&self) -> &str {
        "speaking-model"
    }

    fn requires_terminal_event(&self) -> bool {
        true
    }
}

/// Records the messages it is handed, then ends the turn.
struct RecordingModel {
    captured: Arc<Mutex<Option<Vec<Message>>>>,
}

#[async_trait]
impl ModelClient for RecordingModel {
    fn stream(
        &self,
        messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        *self.captured.lock().unwrap() = Some(messages.to_vec());
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta {
                text: "resumed".to_string(),
            }),
            Ok(ModelEvent::StopReason {
                reason: StopReason::EndTurn,
            }),
            Ok(ModelEvent::Done),
        ]))
    }

    fn model_id(&self) -> &str {
        "recording-model"
    }

    fn requires_terminal_event(&self) -> bool {
        true
    }
}

fn build_engine(model: Box<dyn ModelClient>, root: &std::path::Path) -> Engine {
    Engine::with_workspace(
        model,
        ToolRegistry::new(),
        ContextManager::new("system".to_string()),
        EngineConfig::new(3, false),
        Workspace::detect(root).unwrap(),
        ApprovalPolicy::Auto,
    )
}

fn blank_state(session_id: SessionId, goal: &str) -> TaskState {
    TaskState {
        schema_version: 1,
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: goal.to_string(),
        step: 1,
        history: Vec::new(),
        summary: None,
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        agent_profile: None,
        step_ledger: Default::default(),
        execution_lifecycle: Default::default(),
    }
}

#[tokio::test]
async fn resume_rebuilds_model_context_from_the_trace_history_stream_alone() {
    let tmp = tempfile::TempDir::new().unwrap();
    let run_dir = tmp.path();

    // --- Run 1: produce a trace carrying explicit history lines. ---
    let trace_writer = TraceWriter::new(run_dir).unwrap();
    let engine = build_engine(
        Box::new(SpeakingModel {
            text: "first answer",
        }),
        run_dir,
    );
    let stream = engine.ask("original question".to_string(), Some(trace_writer));
    futures::pin_mut!(stream);
    while stream.next().await.is_some() {}

    // The trace must carry the model-visible stream, not just UI events.
    let trace_body = std::fs::read_to_string(run_dir.join("trace.jsonl")).unwrap();
    let outcome = read_trace_content(&trace_body);
    assert!(
        outcome.has_explicit_history(),
        "run 1 must persist an explicit history stream"
    );
    let traced: Vec<String> = outcome
        .history_items
        .iter()
        .filter_map(|record| match &record.item {
            rove_core::history::HistoryItem::Message(message) => Some(message.content.clone()),
            _ => None,
        })
        .collect();
    assert!(
        traced.iter().any(|content| content == "original question"),
        "the user turn is model-visible history: {traced:?}"
    );
    assert!(
        traced.iter().any(|content| content == "first answer"),
        "the assistant turn is model-visible history: {traced:?}"
    );

    // --- Reconcile with an empty snapshot: the trace is the only source. ---
    let session_id = SessionId::new();
    let mut resume_state = blank_state(session_id, "original question");
    assert!(resume_state.history.is_empty());
    reconcile_task_state_with_trace(run_dir, &mut resume_state)
        .await
        .unwrap();
    assert!(
        !resume_state.history.is_empty(),
        "history must be rebuilt from the trace alone, with no snapshot to lean on"
    );

    // --- Run 2: resume and observe what the model actually receives. ---
    let captured = Arc::new(Mutex::new(None));
    let resume_tmp = tempfile::TempDir::new().unwrap();
    let resume_engine = build_engine(
        Box::new(RecordingModel {
            captured: captured.clone(),
        }),
        resume_tmp.path(),
    );
    let request = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "follow up".to_string(),
        content_blocks: Vec::new(),
        resume_state: Some(resume_state),
    };
    let stream = resume_engine.run(request, None);
    futures::pin_mut!(stream);
    while stream.next().await.is_some() {}

    let messages = captured.lock().unwrap().take().expect("model was called");
    let conversation: Vec<(Role, String)> = messages
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect();

    // The soul assertion: run 1's conversation reached the model on resume,
    // recovered from the trace history stream with no snapshot and no
    // heuristic classification of UI events.
    assert!(
        conversation
            .iter()
            .any(|(role, content)| *role == Role::User && content == "original question"),
        "resume lost the original user turn: {conversation:?}"
    );
    assert!(
        conversation
            .iter()
            .any(|(role, content)| *role == Role::Assistant && content == "first answer"),
        "resume lost the first assistant turn: {conversation:?}"
    );
    assert!(
        conversation
            .iter()
            .any(|(role, content)| *role == Role::User && content == "follow up"),
        "the new user turn must follow the recovered history: {conversation:?}"
    );
}

/// The seed a fork-at-message child starts from.
///
/// The child keeps the parent run as its resume identity, but its canonical
/// session is the prefix the edit preserves. `history_pruned` is what tells the
/// engine that an empty history is the child's own answer rather than a
/// snapshot that never landed.
fn pruned_fork_seed(
    session_id: SessionId,
    parent_run_id: RunId,
    goal: &str,
    history_pruned: bool,
) -> TaskState {
    TaskState {
        schema_version: 1,
        session_id,
        job_id: JobId::new(),
        run_id: parent_run_id,
        goal: goal.to_string(),
        step: 1,
        history: Vec::new(),
        summary: None,
        checkpoint: Some(PromptCheckpoint {
            summary: None,
            preserved_tail: Vec::new(),
            session: Some(Session::with_id(session_id)),
            plan: None,
            session_memory_pointer: None,
            durable_memory_pointer: None,
            last_step: 1,
            last_event_seq: None,
            token_estimate: 0,
            compacted_history_messages: 0,
            compaction: Default::default(),
            runtime_identity: None,
            agent_profile: None,
            step_ledger: Default::default(),
            execution_lifecycle: Default::default(),
            message_deliveries: Vec::new(),
            history_pruned,
        }),
        plan: None,
        runtime_identity: None,
        agent_profile: None,
        step_ledger: Default::default(),
        execution_lifecycle: Default::default(),
    }
}

/// Run one child turn from a fork-shaped seed and report what the model saw.
async fn run_fork_child_seed(
    root: &std::path::Path,
    parent_run_id: RunId,
    history_pruned: bool,
) -> Vec<(Role, String)> {
    let session_id = SessionId::new();
    let captured = Arc::new(Mutex::new(None));
    let engine = build_engine(
        Box::new(RecordingModel {
            captured: captured.clone(),
        }),
        root,
    );
    let request = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "edited question".to_string(),
        content_blocks: Vec::new(),
        resume_state: Some(pruned_fork_seed(
            session_id,
            parent_run_id,
            "edited question",
            history_pruned,
        )),
    };
    let stream = engine.run(request, None);
    futures::pin_mut!(stream);
    while stream.next().await.is_some() {}

    let messages = captured.lock().unwrap().take().expect("model was called");
    messages
        .iter()
        .map(|message| (message.role.clone(), message.content.clone()))
        .collect()
}

/// A fork-at-message child must not be refilled from the run it edited.
///
/// The trace fallback exists so a run killed before its checkpoint landed still
/// resumes with context. A pruned fork seed looks identical from the outside —
/// empty history, a resume identity — but its emptiness is the edit's result,
/// and the parent run that identity points at still holds everything the edit
/// removed. This test runs the same seed twice: without the fork marker the
/// fallback must still refill, which is what makes the marked assertion below
/// evidence instead of a tautology.
#[tokio::test]
async fn a_pruned_fork_seed_is_not_refilled_from_the_parent_trace() {
    let root = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(root.path()).unwrap();

    // The parent run, whose trace is the history source the child's resume
    // identity resolves to. It is written exactly where the engine looks for a
    // run's trace: `<state_dir>/runs/<run_id>/trace.jsonl`.
    let parent_run_id = RunId::new();
    let parent_run_dir = workspace
        .state_dir
        .join("runs")
        .join(parent_run_id.to_string());
    std::fs::create_dir_all(&parent_run_dir).unwrap();
    let engine = build_engine(
        Box::new(SpeakingModel {
            text: "removed answer",
        }),
        root.path(),
    );
    let stream = engine.ask(
        "removed question".to_string(),
        Some(TraceWriter::new(&parent_run_dir).unwrap()),
    );
    futures::pin_mut!(stream);
    while stream.next().await.is_some() {}
    let trace = std::fs::read_to_string(parent_run_dir.join("trace.jsonl")).unwrap();
    assert!(
        read_trace_content(&trace).has_explicit_history(),
        "the parent run must leave model-visible history in its trace"
    );

    // Control: an ordinary empty-history resume still recovers the parent run.
    let refilled = run_fork_child_seed(root.path(), parent_run_id, false).await;
    assert!(
        refilled
            .iter()
            .any(|(role, content)| *role == Role::User && content == "removed question"),
        "the trace fallback must still refill an ordinary empty history: {refilled:?}"
    );
    assert!(
        refilled
            .iter()
            .any(|(role, content)| *role == Role::Assistant && content == "removed answer"),
        "the trace fallback must still recover the parent's answer: {refilled:?}"
    );

    // The child: same resume identity, but its empty seed is deliberate.
    let child = run_fork_child_seed(root.path(), parent_run_id, true).await;
    assert!(
        !child
            .iter()
            .any(|(_, content)| content.contains("removed question")
                || content.contains("removed answer")),
        "the edited message and everything after it must not reach the child's model call: {child:?}"
    );
    assert!(
        child
            .iter()
            .any(|(role, content)| *role == Role::User && content == "edited question"),
        "the child still runs its own turn: {child:?}"
    );
}
