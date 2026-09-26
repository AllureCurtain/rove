//! Offline contract tests for abort salvage (`R2b`).
//!
//! Cancelling a run used to drop whatever the model had already streamed: a
//! turn cut short mid-answer produced no assistant message in resumable
//! history and no `llm_message` event, so a user who had *seen* text lost it
//! on the next resume. R2b keeps that text, bounded by a short salvage window.
//!
//! Everything below runs against the deterministic Fake provider, so the race
//! is decided by the test and not by the scheduler:
//!
//! - [`FakeTurn::Gate`] emits text and then stays in flight until released, so
//!   "the request finished inside the window" and "the request outlived the
//!   window" are two explicit test choices rather than two timing hopes;
//! - the cancel is always raised by the test at a named observable event.
//!
//! The assertions cover the four surfaces the contract names: the stream event
//! (`aborted`), resumable history (`task_state.json`), the trace file, and the
//! terminal reason, which stays `cancelled` in every salvage case.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::Notify;

use rove_core::ToolRegistry;
use rove_models::{
    FakeModelClient, FakeTurn, Message, ModelClient, ModelError, ModelEvent, ModelToolSchema, Role,
    StopReason,
};
use rove_runtime::Workspace;
use rove_runtime::context::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig};
use rove_runtime::events::{StreamEvent, TraceEntry};
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::store::StateStore;
use rove_runtime::state::trace_reader::read_trace_content;
use rove_runtime::types::{
    ApprovalPolicy, JobId, RunId, RunRequest, SessionId, TaskState, TerminationReason,
};

/// The user message every scripted run answers.
const GOAL: &str = "answer the request";

/// One observed `llm_message` event, reduced to the facts these tests assert.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedMessage {
    full: String,
    aborted: bool,
    tool_calls: usize,
}

/// Everything one scripted run produced.
struct SalvagedRun {
    events: Vec<StreamEvent>,
    state: TaskState,
    trace: String,
    /// Whether the test's cancel predicate actually fired.
    cancel_sent: bool,
}

impl SalvagedRun {
    fn messages(&self) -> Vec<ObservedMessage> {
        self.events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::LlmMessage {
                    full,
                    aborted,
                    tool_calls,
                    ..
                } => Some(ObservedMessage {
                    full: full.clone(),
                    aborted: *aborted,
                    tool_calls: tool_calls.len(),
                }),
                _ => None,
            })
            .collect()
    }

    fn aborted_messages(&self) -> Vec<ObservedMessage> {
        self.messages()
            .into_iter()
            .filter(|message| message.aborted)
            .collect()
    }

    fn completed(&self, reason: TerminationReason) -> bool {
        self.events.iter().any(|event| {
            matches!(event, StreamEvent::RunCompleted { reason: actual, .. } if *actual == reason)
        })
    }

    fn saw_text(&self, needle: &str) -> bool {
        self.events.iter().any(|event| match event {
            StreamEvent::LlmChunk { delta } => delta.contains(needle),
            StreamEvent::LlmMessage { full, .. } => full.contains(needle),
            _ => false,
        })
    }

    /// Assistant entries in resumable history whose content contains `needle`.
    fn assistant_history(&self, needle: &str) -> Vec<String> {
        self.state
            .history
            .iter()
            .filter(|message| message.role == Role::Assistant && message.content.contains(needle))
            .map(|message| message.content.clone())
            .collect()
    }

    /// Every assistant entry in resumable history, whatever it says.
    fn all_assistant_history(&self) -> Vec<String> {
        self.state
            .history
            .iter()
            .filter(|message| message.role == Role::Assistant)
            .map(|message| message.content.clone())
            .collect()
    }

    /// `llm_message` facts read back out of `trace.jsonl`.
    fn trace_messages(&self) -> Vec<ObservedMessage> {
        read_trace_content(&self.trace)
            .entries
            .iter()
            .filter_map(|record| match &record.entry {
                TraceEntry::Ui(StreamEvent::LlmMessage {
                    full,
                    aborted,
                    tool_calls,
                    ..
                }) => Some(ObservedMessage {
                    full: full.clone(),
                    aborted: *aborted,
                    tool_calls: tool_calls.len(),
                }),
                _ => None,
            })
            .collect()
    }
}

/// A workspace, a state directory, and the store that persists runs into it.
struct Harness {
    _tmp: tempfile::TempDir,
    state_store: StateStore,
    workspace: Workspace,
}

impl Harness {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_store = StateStore::new(&tmp.path().join("state"));
        let workspace = Workspace::detect(tmp.path()).unwrap();
        Self {
            _tmp: tmp,
            state_store,
            workspace,
        }
    }

    fn engine(&self, model: Box<dyn ModelClient>) -> Engine {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(rove_runtime::tools::echo::EchoTool));
        Engine::with_workspace(
            model,
            registry,
            ContextManager::new("You are a test agent.".to_string()),
            EngineConfig::new(4, false),
            self.workspace.clone(),
            ApprovalPolicy::Auto,
        )
    }

    /// Drive one run to its end, persisting every event through the canonical
    /// artifact recorder, and cancel as soon as `cancel_when` says so.
    ///
    /// Cancelling inside the consumer loop is exactly what the API and CLI do,
    /// so the ordering under test is the production ordering: the event is
    /// observed first, the cancel follows.
    async fn run(
        &self,
        engine: &Engine,
        resume_state: Option<TaskState>,
        mut cancel_when: impl FnMut(&StreamEvent) -> bool,
        release_after_cancel: Option<Arc<Notify>>,
    ) -> SalvagedRun {
        let session_id = SessionId::new();
        let job_id = JobId::new();
        let run_id = RunId::new();
        let trace_writer = self.state_store.run_store.create_trace(&run_id).unwrap();
        let run_dir = self.state_store.run_store.run_dir(&run_id);
        let mut recorder = RunArtifactRecorder::new(
            session_id,
            job_id,
            run_id,
            GOAL.to_string(),
            resume_state.as_ref(),
            Some(engine.runtime_identity()),
        );

        let stream = engine.run(
            RunRequest {
                session_id,
                job_id,
                run_id,
                user_message: GOAL.to_string(),
                resume_state,
            },
            Some(trace_writer),
        );
        futures::pin_mut!(stream);

        let mut events = Vec::new();
        let mut cancel_sent = false;
        while let Some(event) = stream.next().await {
            if cancel_when(&event) {
                // Cancelling repeatedly is legal and must stay idempotent, so
                // the predicate may fire on more than one event.
                stream.cancel();
                if !cancel_sent {
                    cancel_sent = true;
                    // Releasing after the cancel is what makes "the request
                    // finished inside the window" deterministic: the salvage
                    // window is already open when the gate opens.
                    if let Some(release) = &release_after_cancel {
                        release.notify_one();
                    }
                }
            }
            recorder.record_event(&event, &self.state_store).await;
            events.push(event);
        }

        recorder
            .finalize(
                &self.state_store,
                engine.workspace(),
                engine.model_id(),
                &run_dir,
            )
            .await;
        let state = self.state_store.load_task_state(run_id).await.unwrap();
        let trace = std::fs::read_to_string(run_dir.join("trace.jsonl")).unwrap();
        SalvagedRun {
            events,
            state,
            trace,
            cancel_sent,
        }
    }

    /// The common case: one scripted Fake turn, cancelled at a named event.
    async fn run_scripted(
        &self,
        turns: Vec<FakeTurn>,
        cancel_when: impl FnMut(&StreamEvent) -> bool,
        release_after_cancel: Option<Arc<Notify>>,
    ) -> SalvagedRun {
        let engine = self.engine(Box::new(FakeModelClient::with_turns(
            "unscripted fallback response".to_string(),
            turns,
        )));
        self.run(&engine, None, cancel_when, release_after_cancel)
            .await
    }
}

/// Records the messages it is handed, so a resumed run can prove what the
/// model actually received.
struct RecordingModel {
    captured: Arc<Mutex<Option<Vec<Message>>>>,
}

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

/// Cancel as soon as the model has put any text on screen.
fn cancel_on_first_chunk(event: &StreamEvent) -> bool {
    matches!(event, StreamEvent::LlmChunk { .. })
}

#[tokio::test]
async fn a_cancelled_turn_keeps_the_text_the_user_already_saw() {
    let harness = Harness::new();
    // The gate is never released, so the request outlives the salvage window.
    let release = Arc::new(Notify::new());
    let run = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "half an answer".to_string(),
                release,
            }],
            cancel_on_first_chunk,
            None,
        )
        .await;

    assert!(run.cancel_sent, "the cancel must have been observed");
    assert!(
        run.completed(TerminationReason::Cancelled),
        "a salvaged run is still a cancelled run: {:?}",
        run.events
    );

    let salvaged = run.aborted_messages();
    assert_eq!(
        salvaged,
        vec![ObservedMessage {
            full: "half an answer".to_string(),
            aborted: true,
            tool_calls: 0,
        }],
        "exactly one aborted message carries the accumulated text: {:?}",
        run.messages()
    );

    // Persistence surface 1: resumable history.
    assert_eq!(
        run.assistant_history("half an answer"),
        vec!["half an answer".to_string()],
        "the salvaged text is in resumable history exactly once: {:?}",
        run.state.history
    );
    // Persistence surface 2: the trace file carries the fact, not just the UI.
    assert_eq!(
        run.trace_messages(),
        vec![ObservedMessage {
            full: "half an answer".to_string(),
            aborted: true,
            tool_calls: 0,
        }],
        "trace.jsonl records the aborted message"
    );
    // The wire shape is explicit, so an old reader that ignores unknown fields
    // sees the same message it always did.
    assert!(
        run.trace.contains(r#""type":"llm_message""#),
        "the event name is unchanged: {}",
        run.trace
    );
    assert!(
        run.trace.contains(r#""aborted":true"#),
        "the marker is serialized on the wire: {}",
        run.trace
    );
}

#[tokio::test]
async fn a_request_that_finishes_inside_the_window_keeps_a_complete_message() {
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let run = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "the whole answer".to_string(),
                release: release.clone(),
            }],
            cancel_on_first_chunk,
            Some(release),
        )
        .await;

    assert!(run.cancel_sent, "the cancel must have been observed");
    assert!(
        run.completed(TerminationReason::Cancelled),
        "the run stays cancelled even when the message arrived whole: {:?}",
        run.events
    );

    let messages = run.messages();
    assert_eq!(
        messages,
        vec![ObservedMessage {
            full: "the whole answer".to_string(),
            aborted: false,
            tool_calls: 0,
        }],
        "a request that finished inside the window is not marked aborted"
    );
    assert!(
        run.aborted_messages().is_empty(),
        "nothing may be marked aborted when the message completed"
    );
    assert_eq!(
        run.assistant_history("the whole answer"),
        vec!["the whole answer".to_string()],
        "the complete message is persisted once"
    );
}

#[tokio::test]
async fn a_cancel_with_no_accumulated_text_still_writes_nothing() {
    let harness = Harness::new();
    // Text is available inside the turn, but the cancel lands before any of it
    // reaches the screen, so this must stay exactly the pre-salvage behavior.
    let release = Arc::new(Notify::new());
    let run = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "text the user never saw".to_string(),
                release,
            }],
            |event| matches!(event, StreamEvent::ModelStatus { .. }),
            None,
        )
        .await;

    assert!(run.cancel_sent, "the cancel must have been observed");
    assert!(
        run.completed(TerminationReason::Cancelled),
        "the run is cancelled: {:?}",
        run.events
    );
    assert!(
        run.messages().is_empty(),
        "no model message may be invented for an empty stop: {:?}",
        run.messages()
    );
    assert!(
        run.all_assistant_history().is_empty(),
        "nothing may be written to resumable history: {:?}",
        run.state.history
    );
    assert!(
        run.trace_messages().is_empty(),
        "nothing may be written to the trace"
    );
    assert!(
        !run.saw_text("text the user never saw"),
        "the unsent text must not leak into any event: {:?}",
        run.events
    );
}

#[tokio::test]
async fn a_second_cancel_neither_restarts_the_window_nor_writes_twice() {
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let engine = harness.engine(Box::new(FakeModelClient::with_turns(
        "unscripted fallback response".to_string(),
        vec![FakeTurn::Gate {
            text: "one partial only".to_string(),
            release,
        }],
    )));

    // Cancel on the observable text and again on every event after it, which
    // is what a user pressing stop repeatedly produces.
    let mut text_seen = false;
    let run = harness
        .run(
            &engine,
            None,
            |event| {
                if matches!(event, StreamEvent::LlmChunk { .. }) {
                    text_seen = true;
                }
                text_seen
            },
            None,
        )
        .await;

    assert!(
        run.completed(TerminationReason::Cancelled),
        "the run is cancelled once: {:?}",
        run.events
    );
    assert_eq!(
        run.aborted_messages(),
        vec![ObservedMessage {
            full: "one partial only".to_string(),
            aborted: true,
            tool_calls: 0,
        }],
        "a repeated cancel must not write a second partial: {:?}",
        run.messages()
    );
    assert_eq!(
        run.trace_messages().len(),
        1,
        "a repeated cancel must not write a second trace fact"
    );
    assert_eq!(
        run.assistant_history("one partial only"),
        vec!["one partial only".to_string()],
        "a repeated cancel must not duplicate the history entry"
    );
}

#[tokio::test]
async fn a_cancel_during_a_tool_call_never_salvages() {
    let harness = Harness::new();
    let run = harness
        .run_scripted(
            vec![
                FakeTurn::ToolUse {
                    id: "call-1".to_string(),
                    name: "echo".to_string(),
                    args: serde_json::json!({ "message": "hello" }),
                },
                FakeTurn::Text("must never be reached after cancellation".to_string()),
            ],
            |event| matches!(event, StreamEvent::ToolCallStarted { .. }),
            None,
        )
        .await;

    assert!(run.cancel_sent, "the cancel must have been observed");
    assert!(
        run.completed(TerminationReason::Cancelled),
        "cancelling during a tool call still terminates as cancelled: {:?}",
        run.events
    );
    assert!(
        run.aborted_messages().is_empty(),
        "the tool path must not gain a salvaged message: {:?}",
        run.messages()
    );
    assert_eq!(
        run.messages(),
        vec![ObservedMessage {
            full: String::new(),
            aborted: false,
            tool_calls: 1,
        }],
        "the model turn that requested the tool is unchanged"
    );
    assert!(
        !run.saw_text("must never be reached after cancellation"),
        "no further model call may run after the cancel: {:?}",
        run.events
    );
    assert!(
        !run.events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolCallCompleted { .. })),
        "an interrupted tool call may not report completion: {:?}",
        run.events
    );
}

#[tokio::test]
async fn resume_sees_a_salvaged_message_once_and_never_replays_it() {
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let first = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "partial worth keeping".to_string(),
                release,
            }],
            cancel_on_first_chunk,
            None,
        )
        .await;
    assert_eq!(
        first.assistant_history("partial worth keeping"),
        vec!["partial worth keeping".to_string()],
        "the first run salvaged its partial"
    );

    // Resume the salvaged run and watch what the model is actually handed.
    let captured = Arc::new(Mutex::new(None));
    let engine = harness.engine(Box::new(RecordingModel {
        captured: captured.clone(),
    }));
    let second = harness
        .run(&engine, Some(first.state), |_| false, None)
        .await;

    let messages = captured.lock().unwrap().take().expect("model was called");
    let seen: Vec<&Message> = messages
        .iter()
        .filter(|message| message.content.contains("partial worth keeping"))
        .collect();
    assert_eq!(
        seen.len(),
        1,
        "the salvaged partial is visible to the model exactly once: {:?}",
        messages
            .iter()
            .map(|message| (message.role.clone(), message.content.clone()))
            .collect::<Vec<_>>()
    );
    assert_eq!(seen[0].role, Role::Assistant);
    assert!(
        second
            .messages()
            .iter()
            .all(|message| !message.full.contains("partial worth keeping")),
        "a resumed run must not replay the salvaged message as a new event: {:?}",
        second.messages()
    );
    assert_eq!(
        second.assistant_history("partial worth keeping"),
        vec!["partial worth keeping".to_string()],
        "resuming must not duplicate the salvaged entry: {:?}",
        second.state.history
    );
}

#[tokio::test]
async fn a_trace_line_without_the_field_is_a_complete_message() {
    // Hand-written pre-R2b line: no `aborted` key anywhere. Readers must
    // reconstruct it as a normal, complete assistant message.
    let legacy = concat!(
        r#"{"type":"llm_message","full":"legacy answer","#,
        r#""usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3,"cached_tokens":0},"#,
        r#""tool_calls":[],"assistant_turn":null}"#
    );
    let entry: TraceEntry = serde_json::from_str(legacy).expect("legacy line must still decode");
    match entry {
        TraceEntry::Ui(StreamEvent::LlmMessage { full, aborted, .. }) => {
            assert_eq!(full, "legacy answer");
            assert!(!aborted, "a legacy line means complete, not aborted");
        }
        other => panic!("expected an llm_message event, got {other:?}"),
    }

    // The default is also what an omitted field means when the field is added
    // by a reader, so the additive change stays backward compatible.
    let with_field: TraceEntry = serde_json::from_str(
        r#"{"type":"llm_message","full":"cut short","usage":{"prompt_tokens":0,"completion_tokens":0,"total_tokens":0},"aborted":true}"#,
    )
    .expect("an explicit marker must decode");
    match with_field {
        TraceEntry::Ui(StreamEvent::LlmMessage { aborted, .. }) => assert!(aborted),
        other => panic!("expected an llm_message event, got {other:?}"),
    }
}

#[tokio::test]
async fn the_salvage_window_is_bounded() {
    // The window is a policy constant, not a config knob, so this asserts the
    // bound the contract names instead of trusting the call site.
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let started = std::time::Instant::now();
    let run = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "bounded".to_string(),
                release,
            }],
            cancel_on_first_chunk,
            None,
        )
        .await;
    let elapsed = started.elapsed();

    assert!(run.aborted_messages().len() == 1);
    assert!(
        elapsed < Duration::from_secs(5),
        "a stalled request must not hold the run open indefinitely, took {elapsed:?}"
    );
}
