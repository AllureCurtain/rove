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
//! - [`FakeTurn::GateThenFail`] does the same and then *fails*, so "the provider
//!   errored inside the window" is a third explicit choice instead of a race;
//! - the cancel is always raised by the test at a named observable event.
//!
//! The assertions cover the four surfaces the contract names: the stream event
//! (`aborted`), resumable history (`task_state.json`), the trace file, and the
//! terminal reason, which stays `cancelled` in every salvage case.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
use rove_runtime::session::SessionEntry;
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
    /// The run these facts belong to, so a test can rewrite or reload the
    /// snapshot it left on disk.
    run_id: RunId,
    /// `task_state.json` exactly as it was written, so a test can assert on the
    /// bytes a reader of the durable state actually sees.
    state_json: String,
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

    /// Assistant entries in the persisted session, with the salvage marker.
    ///
    /// The flat `history` projection is a `Message` list and cannot carry the
    /// flag; the canonical session entry is where the durable state keeps it.
    fn session_assistant_entries(&self) -> Vec<(String, bool)> {
        session_assistant_entries(&self.state)
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

/// Assistant entries in a persisted session's canonical entries, with the
/// salvage marker. The flat `history` projection is a `Message` list and cannot
/// carry the flag; the canonical session entry is where the durable state keeps
/// it.
fn session_assistant_entries(state: &TaskState) -> Vec<(String, bool)> {
    state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.session.as_ref())
        .map(|session| {
            session
                .entries
                .iter()
                .filter_map(|entry| match entry {
                    SessionEntry::Assistant { turn, aborted, .. } => Some((
                        turn.content
                            .iter()
                            .filter_map(rove_models::ContentBlock::text_value)
                            .collect::<String>(),
                        *aborted,
                    )),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Remove every `aborted` key from a JSON tree, producing the snapshot a runtime
/// that had no such field would have written.
fn strip_aborted_keys(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("aborted");
            for child in map.values_mut() {
                strip_aborted_keys(child);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items.iter_mut() {
                strip_aborted_keys(child);
            }
        }
        _ => {}
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
                content_blocks: Vec::new(),
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
        let state_json = std::fs::read_to_string(run_dir.join("task_state.json")).unwrap();
        let trace = std::fs::read_to_string(run_dir.join("trace.jsonl")).unwrap();
        SalvagedRun {
            events,
            state,
            run_id,
            state_json,
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

/// The stream fact and the resumable state have to agree. A stop that kept
/// partial text marks the durable assistant message too, so a session that is
/// resumed later reports the same stop a live one did instead of looking like a
/// complete answer.
#[tokio::test]
async fn a_salvaged_partial_marks_the_resumable_session() {
    let harness = Harness::new();
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

    // The live stream and the durable snapshot are two readings of one fact, so
    // the same run must say the same thing on both.
    assert_eq!(
        run.aborted_messages(),
        vec![ObservedMessage {
            full: "half an answer".to_string(),
            aborted: true,
            tool_calls: 0,
        }],
        "the stream this snapshot was written from marked the same text"
    );
    assert_eq!(
        run.session_assistant_entries(),
        vec![("half an answer".to_string(), true)],
        "the persisted session must carry the marker the stream published: {}",
        run.state_json
    );
    assert!(
        run.state_json.contains(r#""aborted": true"#),
        "the marker is in the bytes of task_state.json: {}",
        run.state_json
    );
    assert!(
        !run.state_json.contains(r#""aborted": false"#),
        "an unset marker is omitted, never written as false: {}",
        run.state_json
    );
}

/// Every message the user did not stop keeps the durable state it had before
/// the marker existed: one that arrived whole inside the salvage window, and a
/// stop that produced no text at all.
#[tokio::test]
async fn an_unmarked_run_keeps_the_resumable_state_bytes_it_had() {
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let complete = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "the whole answer".to_string(),
                release: release.clone(),
            }],
            cancel_on_first_chunk,
            Some(release),
        )
        .await;

    assert_eq!(
        complete.session_assistant_entries(),
        vec![("the whole answer".to_string(), false)],
        "a request that finished inside the window is not marked"
    );
    assert!(
        !complete.state_json.contains("aborted"),
        "an unmarked message must not grow the key: {}",
        complete.state_json
    );
    // Decoding and re-encoding the state of an unmarked message reproduces the
    // file byte-for-byte, which is exactly what a pre-marker writer produced.
    let decoded: TaskState = serde_json::from_str(&complete.state_json).unwrap();
    assert_eq!(
        serde_json::to_string_pretty(&decoded).unwrap(),
        complete.state_json.trim_end(),
        "the unmarked snapshot stays byte-identical through a decode/encode cycle"
    );

    let empty = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "text the user never saw".to_string(),
                release: Arc::new(Notify::new()),
            }],
            |event| matches!(event, StreamEvent::ModelStatus { .. }),
            None,
        )
        .await;
    assert!(
        empty.session_assistant_entries().is_empty(),
        "a stop with nothing on screen writes no message at all: {}",
        empty.state_json
    );
    assert!(
        !empty.state_json.contains("aborted"),
        "and therefore no marker either: {}",
        empty.state_json
    );
}

/// A snapshot is not rewritten by the field's arrival. Removing every `aborted`
/// key produces the `task_state.json` a pre-marker runtime wrote for the same
/// history; that file must still load through the state store, still decode the
/// kept text as a complete message, and still resume exactly as it did before —
/// visible once, never replayed, and never retroactively marked.
#[tokio::test]
async fn a_snapshot_written_before_the_marker_resumes_exactly_as_before() {
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let first = harness
        .run_scripted(
            vec![FakeTurn::Gate {
                text: "partial from before".to_string(),
                release,
            }],
            cancel_on_first_chunk,
            None,
        )
        .await;
    assert_eq!(
        first.session_assistant_entries(),
        vec![("partial from before".to_string(), true)],
        "the run under test did salvage, so there is a key to remove"
    );

    let mut value: serde_json::Value = serde_json::from_str(&first.state_json).unwrap();
    strip_aborted_keys(&mut value);
    let legacy = serde_json::to_string_pretty(&value).unwrap();
    assert!(!legacy.contains("aborted"), "{legacy}");
    std::fs::write(
        harness
            .state_store
            .run_store
            .run_dir(&first.run_id)
            .join("task_state.json"),
        &legacy,
    )
    .unwrap();

    let loaded = harness
        .state_store
        .load_task_state(first.run_id)
        .await
        .expect("a snapshot without the field still loads");
    assert_eq!(
        session_assistant_entries(&loaded),
        vec![("partial from before".to_string(), false)],
        "the missing field decodes as the pre-marker meaning, not as aborted"
    );

    // Resume from that legacy snapshot and watch what the model is handed.
    let captured = Arc::new(Mutex::new(None));
    let engine = harness.engine(Box::new(RecordingModel {
        captured: captured.clone(),
    }));
    let second = harness.run(&engine, Some(loaded), |_| false, None).await;

    let messages = captured.lock().unwrap().take().expect("model was called");
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.content.contains("partial from before"))
            .count(),
        1,
        "a legacy snapshot still shows the kept text exactly once"
    );
    assert_eq!(
        second.session_assistant_entries(),
        vec![
            ("partial from before".to_string(), false),
            ("resumed".to_string(), false),
        ],
        "nothing is retroactively marked, and the resumed run adds no key: {}",
        second.state_json
    );
    assert!(
        !second.state_json.contains("aborted"),
        "a legacy snapshot keeps its legacy shape after resume: {}",
        second.state_json
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
    assert_eq!(
        first.session_assistant_entries(),
        vec![("partial worth keeping".to_string(), true)],
        "the salvaged partial is marked in the state the resume reads"
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
    // The marker rode along with the entry, so the resumed snapshot still says
    // how that answer ended; the message this run produced itself is unmarked.
    assert_eq!(
        second.session_assistant_entries(),
        vec![
            ("partial worth keeping".to_string(), true),
            ("resumed".to_string(), false),
        ],
        "the marker survives resume and never spreads: {}",
        second.state_json
    );
    assert!(
        !second.state_json.contains(r#""aborted": false"#),
        "the unmarked message stays omitted after resume: {}",
        second.state_json
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
async fn a_provider_failure_inside_the_window_still_keeps_the_text_the_user_saw() {
    // The request the window is waiting on is deliberately kept alive past the
    // cancel, so it is allowed to end either way. If it fails, that error must
    // not win: it would discard text the user had already read and report their
    // explicit stop as a model error.
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let first = harness
        .run_scripted(
            vec![FakeTurn::GateThenFail {
                text: "half an answer".to_string(),
                release: release.clone(),
                error: ModelError::StreamInterrupted("connection reset by peer".to_string()),
            }],
            cancel_on_first_chunk,
            Some(release),
        )
        .await;

    assert!(first.cancel_sent, "the cancel must have been observed");
    assert!(
        first.completed(TerminationReason::Cancelled),
        "a stop stays a stop when the request it waited on failed: {:?}",
        first.events
    );
    assert!(
        !first.completed(TerminationReason::Error),
        "the provider error must not overwrite the user's stop: {:?}",
        first.events
    );

    let salvaged = vec![ObservedMessage {
        full: "half an answer".to_string(),
        aborted: true,
        tool_calls: 0,
    }];
    assert_eq!(
        first.messages(),
        salvaged,
        "exactly one aborted, tool-free message carries the text the user read: {:?}",
        first.messages()
    );
    assert_eq!(
        first.assistant_history("half an answer"),
        vec!["half an answer".to_string()],
        "the salvaged text is in resumable history exactly once: {:?}",
        first.state.history
    );
    assert_eq!(
        first.trace_messages(),
        salvaged,
        "trace.jsonl records the same fact, not the discarded provider error"
    );
    assert!(
        !first.trace.contains("connection reset by peer"),
        "the discarded error must not leak into the trace: {}",
        first.trace
    );

    // Resume the salvaged run: the text is visible to the model once and is
    // never replayed as a new event or a second history entry.
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
        .filter(|message| message.content.contains("half an answer"))
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
            .all(|message| !message.full.contains("half an answer")),
        "a resumed run must not replay the salvaged message: {:?}",
        second.messages()
    );
    assert_eq!(
        second.assistant_history("half an answer"),
        vec!["half an answer".to_string()],
        "resuming must not duplicate the salvaged entry: {:?}",
        second.state.history
    );
}

#[tokio::test]
async fn the_same_provider_failure_without_a_cancel_is_still_a_model_error() {
    // The control for the case above: the identical script, with no stop at all.
    // Nothing here may change — a failed turn that already streamed text keeps
    // its pre-salvage behavior (no retry, no `llm_message`, terminal `Error`),
    // which is exactly why the salvage branch has to be cancel-gated.
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    // A `Notify` keeps its permit, so releasing before the run reaches the gate
    // is not lost: the gate opens immediately and the script fails.
    release.notify_one();
    let run = harness
        .run_scripted(
            vec![FakeTurn::GateThenFail {
                text: "half an answer".to_string(),
                release,
                error: ModelError::StreamInterrupted("connection reset by peer".to_string()),
            }],
            |_| false,
            None,
        )
        .await;

    assert!(!run.cancel_sent, "this control never cancels");
    assert!(
        run.completed(TerminationReason::Error),
        "an uncancelled failure still fails the run: {:?}",
        run.events
    );
    assert!(
        !run.completed(TerminationReason::Cancelled),
        "nothing may turn the failure into a cancellation: {:?}",
        run.events
    );
    assert!(
        run.messages().is_empty(),
        "a failed turn writes no llm_message: {:?}",
        run.messages()
    );
    assert!(
        run.all_assistant_history().is_empty(),
        "a failed turn writes no resumable assistant history: {:?}",
        run.state.history
    );
    assert!(
        run.trace_messages().is_empty(),
        "a failed turn writes no llm_message trace fact"
    );
}

/// The two marks a bounded stop leaves, each measured from the test's start:
/// when the test raised the stop, and when the run published its terminal event.
///
/// The interval between them is the stop latency, which is the interval the
/// salvage contract is about.
#[derive(Debug, Clone, Copy, Default)]
struct StopMarks {
    cancelled_at: Option<Duration>,
    completed_at: Option<Duration>,
}

/// How long a stop may take to become a terminal fact before the test calls it
/// unbounded.
///
/// This is not the salvage window: the window is the 1500 ms policy constant in
/// `runtime/src/engine/model_turn.rs`. This is the interval the contract names —
/// from the cancel the test raises to the terminal event the run publishes —
/// with room for a loaded runner. Measured with 64 CPU burners on a 16-CPU host
/// it ran 1.7-3.6 s across 42 runs, while the whole test's wall clock, which also
/// carries harness setup before the cancel and this test's own persistence after
/// the stream ends, reached 5.6 s and failed the bound this replaces in 4 of the
/// 54 loaded runs that exercised it. It is deliberately not the only upper bound:
/// a stop that never becomes terminal cannot pass either, and cannot even pass
/// silently — see [`STOP_DEADLINE`].
const MAX_STOP_LATENCY: Duration = Duration::from_secs(5);

/// The deadline for the whole run. The stalled request in this test is never
/// released, so a stop that does not bound its wait hangs here; the deadline
/// turns that into a failure that names the marks it did observe instead of a
/// test binary that never returns.
const STOP_DEADLINE: Duration = Duration::from_secs(30);

#[tokio::test]
async fn the_salvage_window_is_bounded() {
    // The window is a policy constant, not a config knob, so this asserts the
    // contract the constant names instead of trusting the call site: the stop
    // really waits for the stalled request, and it really gives up. A run that
    // returned at once would mean no window at all; a run that never returned
    // would mean the bound is a comment.
    //
    // Both bounds read the stop latency, not this test's wall clock. The gate is
    // never released, so the only way the turn can end is the window expiring:
    // the interval is the wait under test, and everything the test does around it
    // — building the harness before the cancel, persisting the run after the
    // stream ends — is load the runner carries that the contract does not.
    let harness = Harness::new();
    let release = Arc::new(Notify::new());
    let marks = Arc::new(Mutex::new(StopMarks::default()));
    let observer = Arc::clone(&marks);
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        STOP_DEADLINE,
        harness.run_scripted(
            vec![FakeTurn::Gate {
                text: "bounded".to_string(),
                release,
            }],
            move |event| {
                let mut marks = observer.lock().unwrap();
                if matches!(event, StreamEvent::LlmChunk { .. }) {
                    marks.cancelled_at.get_or_insert_with(|| started.elapsed());
                    return true;
                }
                if matches!(event, StreamEvent::RunCompleted { .. }) {
                    marks.completed_at = Some(started.elapsed());
                }
                false
            },
            None,
        ),
    )
    .await;
    let elapsed = started.elapsed();
    let marks = *marks.lock().unwrap();
    let run = match outcome {
        Ok(run) => run,
        Err(_) => panic!(
            "a stalled request must not hold the run open indefinitely: no terminal event within \
             {STOP_DEADLINE:?}, marks after {elapsed:?}: {marks:?}"
        ),
    };
    let cancelled_at = marks
        .cancelled_at
        .expect("the turn published no LlmChunk, so the test raised no stop");
    let completed_at = marks
        .completed_at
        .expect("the run published no terminal event");
    let stop_latency = completed_at.saturating_sub(cancelled_at);

    assert!(run.aborted_messages().len() == 1);
    // 1500 ms is the window; the lower bound is slack for timer granularity (a
    // timer never fires before its deadline), and the upper bound keeps a
    // stalled request from holding the run open.
    assert!(
        stop_latency >= Duration::from_millis(1_400),
        "a stop with text on screen must wait for the in-flight request, waited {stop_latency:?} \
         (cancel at {cancelled_at:?}, terminal at {completed_at:?}, whole test {elapsed:?})"
    );
    assert!(
        stop_latency < MAX_STOP_LATENCY,
        "a stalled request must not hold the run open indefinitely, waited {stop_latency:?} \
         (cancel at {cancelled_at:?}, terminal at {completed_at:?}, whole test {elapsed:?})"
    );
}
