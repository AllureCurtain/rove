use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

fn assistant_tool_round(ids: &[&str]) -> Message {
    Message::assistant_with_tool_calls(
        "tool round",
        ids.iter()
            .map(|id| rove_runtime::types::ToolCallRef {
                id: (*id).to_string(),
                name: format!("tool_{id}"),
                args: serde_json::json!({ "id": id }),
            })
            .collect(),
    )
}

fn completed(output: Option<&str>) -> StreamEvent {
    StreamEvent::RunCompleted {
        reason: TerminationReason::Final,
        output: output.map(str::to_string),
    }
}

#[test]
fn the_successor_start_budget_backs_off_exponentially_and_stays_bounded() {
    // The initial failed start is attempt 1, so the re-drains it can still
    // schedule are attempts 2..=FOLLOWUP_START_MAX_ATTEMPTS.
    assert_eq!(followup_redrive_backoff(1), Duration::from_millis(1_000));
    assert_eq!(followup_redrive_backoff(2), Duration::from_millis(2_000));
    // The last attempt that can schedule a re-drain already sits on the cap,
    // so a larger budget could not turn into a longer wait.
    assert_eq!(
        followup_redrive_backoff(FOLLOWUP_START_MAX_ATTEMPTS - 1),
        FOLLOWUP_REDRIVE_MAX_BACKOFF
    );
    assert_eq!(followup_redrive_backoff(32), FOLLOWUP_REDRIVE_MAX_BACKOFF);
    assert_eq!(
        (1..FOLLOWUP_START_MAX_ATTEMPTS)
            .map(followup_redrive_backoff)
            .sum::<Duration>(),
        Duration::from_millis(7_000)
    );
}

#[test]
fn a_terminal_without_a_credential_must_match_exactly() {
    assert!(terminal_events_match(
        &completed(Some("the same answer")),
        &completed(Some("the same answer"))
    ));
    // No credential is involved, so redaction is the identity and any
    // divergence is a real one: this is the ordinary durability check.
    assert!(!terminal_events_match(
        &completed(Some("the same answer")),
        &completed(Some("a different answer"))
    ));
    assert!(!terminal_events_match(
        &completed(Some("an answer")),
        &completed(None)
    ));
    assert!(terminal_events_match(&completed(None), &completed(None)));
    // A reason change is a mismatch even when the text is identical.
    assert!(!terminal_events_match(
        &completed(Some("an answer")),
        &StreamEvent::RunCompleted {
            reason: TerminationReason::Error,
            output: Some("an answer".to_string()),
        }
    ));
}

#[test]
fn a_durable_terminal_must_be_the_redaction_of_the_live_one() {
    let canary = "durable-terminal-credential-canary-1f7c4b";
    assert!(rove_runtime::secrets::registry().register_value(canary));
    let live = completed(Some(&format!("keep-terminal-tail {canary}")));
    // The durable row is what the index stored: the same text, redacted.
    let durable = completed(Some(&format!(
        "keep-terminal-tail {}",
        rove_runtime::secrets::KNOWN_SECRET_MARKER
    )));
    assert!(
        terminal_events_match(&durable, &live),
        "the redacted durable copy is the persisted form of the live terminal"
    );
    // A difference the authority does not explain is still a mismatch, so
    // the fallback cannot absorb unrelated divergence.
    let unrelated = completed(Some(&format!(
        "a different tail {}",
        rove_runtime::secrets::KNOWN_SECRET_MARKER
    )));
    assert!(!terminal_events_match(&unrelated, &live));
    // And a live value with no credential in it cannot be explained by a
    // durable copy that has a marker.
    assert!(!terminal_events_match(
        &durable,
        &completed(Some("keep-terminal-tail"))
    ));
}

/// Whether the durability gate can meet a terminal the *declared-field* pass
/// rewrote rather than the value pass.
///
/// The gate requires [`KNOWN_SECRET_MARKER`] before it will treat the durable
/// row as a redacted copy. If a terminal could persist carrying only
/// [`DECLARED_SECRET_FIELD_MARKER`], that run would be judged lost. It cannot:
/// `RunCompleted.output` is a *string* in the trace envelope, and the
/// declared-field pass replaces only the value of a declared key at document
/// level (`"secret": <value>`) — it never descends into a string body. The
/// value pass is the only pass that rewrites string content, and its marker is
/// the one the gate looks for.
///
/// Written through the real trace writer and read back, rather than asserted
/// from the source, so this fails if a later pass starts reaching inside
/// output strings.
///
/// What the last two assertions pin is the gate's *deciding* conjunct: the
/// durable output must be exactly what redacting the live output produces.
/// They deliberately do not claim to pin the `contains(KNOWN_SECRET_MARKER)`
/// conjunct — see [`terminal_events_match`], where that conjunct is documented
/// as subsumed by this one for as long as the value pass writes the marker.
#[test]
fn a_terminal_output_cannot_persist_with_only_the_declared_field_marker() {
    use rove_runtime::state::trace::{TraceLine, TraceWriter};

    // Armed, so the convention field list participates at all.
    let canary = "declared-terminal-arming-canary-6a2e";
    assert!(rove_runtime::secrets::registry().register_value(canary));
    // A terminal answer that is itself JSON and names convention secret
    // fields, with one registered credential inside it.
    let live_output = format!(
        "{{\"answer\":\"keep-terminal-tail\",\"secret\":\"ordinary value\",\"api_key\":\"{canary}\"}}"
    );

    let temp = tempfile::TempDir::new().unwrap();
    let writer = TraceWriter::new(temp.path()).unwrap();
    writer.append(&completed(Some(&live_output))).unwrap();
    let line = std::fs::read_to_string(writer.path()).unwrap();
    let durable: TraceLine = serde_json::from_str(line.trim_end()).unwrap();
    let durable_event = match durable.event {
        rove_runtime::events::TraceEntry::Ui(event) => event,
        other => panic!("the terminal line round-trips as a UI event: {other:?}"),
    };
    let StreamEvent::RunCompleted {
        output: Some(durable_output),
        ..
    } = durable_event
    else {
        panic!("the terminal line round-trips as a terminal event");
    };

    assert!(
        !durable_output.contains(rove_runtime::secrets::DECLARED_SECRET_FIELD_MARKER),
        "the declared-field pass reached inside the output string: {durable_output}"
    );
    assert!(
        durable_output.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
        "{durable_output}"
    );
    // The gate accepts the pair instead of judging the run lost, and it does
    // so because redacting the live output reproduces the durable one byte
    // for byte — the deciding rule, asserted directly rather than inferred
    // from the acceptance.
    assert_eq!(
        rove_runtime::secrets::registry().redact_text(&live_output),
        durable_output
    );
    assert!(terminal_events_match(
        &completed(Some(&durable_output)),
        &completed(Some(&live_output))
    ));

    // The rule is equality, not "contains a marker": a durable row that
    // differs by one byte the authority does not account for is refused even
    // though it still carries the marker.
    let tampered = durable_output.replace("keep-terminal-tail", "keep-terminal-tai1");
    assert_ne!(tampered, durable_output);
    assert!(tampered.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER));
    assert!(
        !terminal_events_match(&completed(Some(&tampered)), &completed(Some(&live_output))),
        "a durable row the authority does not reproduce must be a mismatch"
    );
}

/// The residual this comparison cannot decide, pinned so it stays visible.
///
/// Redaction maps every registered credential to one marker and the durable
/// row keeps nothing else about it, so two terminals whose outputs differ
/// only in *which* credential they carry are indistinguishable here and are
/// accepted as the same terminal. Deciding that case would require
/// persisting a credential-derived fingerprint, which is a durable artifact
/// derived from secret material — the thing this registry exists to avoid.
/// The ambiguity is registered in design §14.3; this test fails loudly if
/// someone later narrows it, and documents the behavior if they do not.
#[test]
fn two_terminal_credentials_are_indistinguishable_once_redacted() {
    let first = "first-terminal-credential-canary-2b8d";
    let second = "second-terminal-credential-canary-5c31";
    let registry = rove_runtime::secrets::registry();
    assert!(registry.register_value(first));
    assert!(registry.register_value(second));
    let live = completed(Some(second));
    let durable_from_first = completed(Some(&registry.redact_text(first)));
    // Both durable copies are the same marker string: nothing distinguishes
    // the first credential from the second once it has been redacted.
    assert_eq!(registry.redact_text(first), registry.redact_text(second));
    assert!(
        terminal_events_match(&durable_from_first, &live),
        "documented residual: credential identity does not survive redaction"
    );
}

#[test]
fn product_follow_up_closes_only_missing_parallel_tool_results() {
    let assistant = assistant_tool_round(&["call-a", "call-b", "call-c"]);
    let result_b = Message::tool("durable result b", Some("call-b".to_string()));
    let result_a = Message::tool("durable result a", Some("call-a".to_string()));
    let next = Message::user("next turn");

    let closed = close_product_follow_up_tool_rounds(vec![
        assistant.clone(),
        result_b.clone(),
        result_a.clone(),
        next.clone(),
    ])
    .unwrap();

    assert_eq!(
        closed,
        vec![
            assistant,
            result_b,
            result_a,
            Message::tool(UNKNOWN_PRODUCT_TOOL_RESULT, Some("call-c".to_string())),
            next,
        ]
    );
}

#[test]
fn product_follow_up_closes_an_all_missing_tool_round_at_the_tail() {
    let assistant = assistant_tool_round(&["call-a", "call-b"]);

    let closed = close_product_follow_up_tool_rounds(vec![assistant.clone()]).unwrap();

    assert_eq!(
        closed,
        vec![
            assistant,
            Message::tool(UNKNOWN_PRODUCT_TOOL_RESULT, Some("call-a".to_string())),
            Message::tool(UNKNOWN_PRODUCT_TOOL_RESULT, Some("call-b".to_string())),
        ]
    );
}

#[test]
fn product_follow_up_preserves_a_complete_tool_round() {
    let messages = vec![
        assistant_tool_round(&["call-a", "call-b"]),
        Message::tool("durable result b", Some("call-b".to_string())),
        Message::tool("durable result a", Some("call-a".to_string())),
        Message::assistant("round complete"),
    ];

    assert_eq!(
        close_product_follow_up_tool_rounds(messages.clone()).unwrap(),
        messages
    );
}

#[test]
fn product_follow_up_drops_orphan_results_from_a_truncated_tail() {
    let next = Message::user("continue after checkpoint");

    let closed = close_product_follow_up_tool_rounds(vec![
        Message::tool("orphan result", Some("truncated-call".to_string())),
        next.clone(),
    ])
    .unwrap();

    assert_eq!(closed, vec![next]);
}

#[test]
fn product_follow_up_preserves_compatibility_tool_results_without_native_ids() {
    let messages = vec![
        Message::assistant("compatibility tool call"),
        Message::tool("durable compatibility result", None),
        Message::assistant("round complete"),
    ];

    assert_eq!(
        close_product_follow_up_tool_rounds(messages.clone()).unwrap(),
        messages
    );
}

#[test]
fn product_follow_up_rejects_duplicate_assistant_tool_call_ids() {
    let error = close_product_follow_up_tool_rounds(vec![assistant_tool_round(&[
        "duplicate",
        "duplicate",
    ])])
    .unwrap_err();

    assert_eq!(
        error.code,
        ProductErrorCode::ProductSessionRuntimeStateCorrupt.as_str()
    );
}

#[test]
fn product_follow_up_rejects_empty_native_tool_call_ids() {
    let assistant_error =
        close_product_follow_up_tool_rounds(vec![assistant_tool_round(&["  "])]).unwrap_err();
    assert_eq!(
        assistant_error.code,
        ProductErrorCode::ProductSessionRuntimeStateCorrupt.as_str()
    );

    let result_error = close_product_follow_up_tool_rounds(vec![Message::tool(
        "invalid native result",
        Some(String::new()),
    )])
    .unwrap_err();
    assert_eq!(
        result_error.code,
        ProductErrorCode::ProductSessionRuntimeStateCorrupt.as_str()
    );
}

/// `GET /jobs/{job_id}/state` is a typed response, so the salvage marker has
/// to be readable from it without parsing the opaque event payload: a client
/// must be able to tell a stop that kept partial text from a complete
/// answer, from a stop that produced nothing, and from a failed turn.
///
/// The marker is omitted, never sent as `false`, so every response for an
/// unmarked message keeps the bytes it had before the field existed.
#[test]
fn job_state_marks_only_a_salvaged_partial_answer() {
    let aborted = job_state_with(vec![llm_message("cut short", true)]);
    let aborted = serde_json::to_value(&aborted).unwrap();
    assert_eq!(
        aborted["answer_aborted"],
        serde_json::json!(true),
        "a salvaged partial is marked: {aborted}"
    );

    for (case, events) in [
        ("a complete answer", vec![llm_message("all of it", false)]),
        ("a stop that produced nothing", Vec::new()),
        (
            "a failed turn",
            vec![StreamEvent::RunCompleted {
                reason: rove_runtime::types::TerminationReason::Error,
                output: None,
            }],
        ),
    ] {
        let value = serde_json::to_value(job_state_with(events.clone())).unwrap();
        assert!(
            value.get("answer_aborted").is_none(),
            "{case} must not carry the marker: {value}"
        );
        let serialized = serde_json::to_string(&job_state_with(events)).unwrap();
        assert!(
            !serialized.contains("answer_aborted"),
            "{case} must keep its previous bytes: {serialized}"
        );
        let decoded: JobStateResponse = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            serialized,
            "{case} round-trips byte-for-byte"
        );
    }
}

fn llm_message(full: &str, aborted: bool) -> StreamEvent {
    StreamEvent::LlmMessage {
        full: full.to_string(),
        usage: rove_models::Usage::default(),
        tool_calls: Vec::new(),
        assistant_turn: None,
        aborted,
    }
}

/// `job_state_from_events` is the production constructor the route uses, so
/// this test fails if the route stops deriving the marker — including if the
/// field were wired as a constant.
fn job_state_with(events: Vec<StreamEvent>) -> JobStateResponse {
    let events = events
        .into_iter()
        .enumerate()
        .map(|(index, event)| JobStreamEvent {
            seq: index as u64 + 1,
            event,
        })
        .collect::<Vec<_>>();
    job_state_from_events(
        JobId::new(),
        RunId::new(),
        None,
        RunStatus::Cancelled,
        events,
        Vec::new(),
        Vec::new(),
    )
}

/// `llm_message` is how an aborted turn reaches a live client, so the
/// salvage marker has to survive the SSE frame itself — not just the
/// in-process event. The event name must stay `llm_message`, because that
/// is what a client written before this field dispatches on. The OpenAPI
/// projection of this event is an opaque object
/// (`#[schema(value_type = Object)]`), so the additive field changes no
/// documented schema; the typed `JobStateResponse` and transcript segment
/// fields are where the marker is part of the documented contract.
#[tokio::test]
async fn sse_frame_carries_the_aborted_marker_on_llm_message() {
    let frame = sse_event(JobStreamEvent {
        seq: 7,
        event: StreamEvent::LlmMessage {
            full: "cut short".to_string(),
            usage: rove_models::Usage::default(),
            tool_calls: Vec::new(),
            assistant_turn: None,
            aborted: true,
        },
    })
    .expect("an llm_message frame must serialize");
    // Rendered through the same `Sse` response the route returns, so the
    // assertions below are about the bytes a client actually reads.
    let stream = futures::stream::iter(vec![frame]).map(Ok::<Event, std::convert::Infallible>);
    let response = axum::response::IntoResponse::into_response(Sse::new(stream));
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();

    assert!(
        text.contains("event: llm_message"),
        "the frame name must not change: {text}"
    );
    assert!(
        text.contains(r#""type":"llm_message""#),
        "the payload keeps its variant tag: {text}"
    );
    assert!(
        text.contains(r#""aborted":true"#),
        "the salvage marker must reach the wire: {text}"
    );
    assert!(
        text.contains(r#""v":"#),
        "the frame keeps its protocol version first: {text}"
    );
}

async fn publish_terminal_event_after_barrier(
    record: &JobRecord,
    event: StreamEvent,
    finalized: &AtomicBool,
) {
    finalized.store(true, Ordering::SeqCst);
    append_job_event(record, event).await;
}

#[tokio::test]
async fn terminal_event_updates_live_status_after_finalization_barrier() {
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let (completion, _) = watch::channel(false);
    let workspace = Workspace::detect(std::env::current_dir().unwrap().as_path()).unwrap();
    let mut config = AppConfig::default();
    config.rebase_to_workspace(&workspace.root);
    let record = JobRecord {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        workspace,
        config,
        message: "test".to_string(),
        content_blocks: Vec::new(),
        resumed_from_run_id: None,
        resume_state: None,
        product_session_id: None,
        product_store: None,
        attachment_storage: None,
        product_model_config: None,
        run_model_snapshot: None,
        status: Mutex::new(RunStatus::Running),
        events: Mutex::new(Vec::new()),
        pending_approvals: Mutex::new(HashMap::new()),
        pending_inputs: Mutex::new(HashMap::new()),
        tx,
        handle: Mutex::new(None),
        control: Mutex::new(None),
        control_event_trace: Mutex::new(None),
        control_event_trace_lock: Mutex::new(()),
        control_lifecycle_lock: Mutex::new(()),
        pending_product_events: Mutex::new(Vec::new()),
        completion,
        cancel_token: CancellationToken::new(),
    };
    let finalized = AtomicBool::new(false);

    publish_terminal_event_after_barrier(
        &record,
        StreamEvent::RunCompleted {
            reason: TerminationReason::StepLimit,
            output: None,
        },
        &finalized,
    )
    .await;

    assert!(finalized.load(Ordering::SeqCst));
    assert_eq!(*record.status.lock().await, RunStatus::Done);
    let events = record.events.lock().await;
    assert!(matches!(
        events.last().map(|event| &event.event),
        Some(StreamEvent::RunCompleted { .. })
    ));
}

#[tokio::test]
async fn completion_guard_notifies_when_a_supervisor_panics() {
    let (completion, mut receiver) = watch::channel(false);
    let handle = tokio::spawn(async move {
        let _guard = JobCompletionGuard::new(completion);
        panic!("supervisor panic fixture");
    });

    assert!(handle.await.is_err());
    receiver.changed().await.unwrap();
    assert!(*receiver.borrow());
}

#[tokio::test]
async fn completion_waiter_waits_for_registration_gap_to_close() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (_state, record, _) = test_job_record(&temp_dir).await;
    assert!(record.handle.lock().await.is_none());

    let waiting_record = Arc::clone(&record);
    let waiter = tokio::spawn(async move {
        wait_for_job_completion(&waiting_record).await;
    });
    tokio::task::yield_now().await;
    assert!(
        !waiter.is_finished(),
        "a live record must not look complete merely because its supervisor is registering"
    );

    record.completion.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("completion waiter should observe the durable completion signal")
        .unwrap();
}

#[tokio::test]
async fn live_job_event_stream_includes_terminal_then_closes() {
    let (sender, receiver) = broadcast::channel(EVENT_BUFFER);
    let mut stream = live_job_event_stream(receiver, 0);
    sender
        .send(JobStreamEvent {
            seq: 1,
            event: StreamEvent::ModelStatus {
                status: "running".to_string(),
                message: "working".to_string(),
            },
        })
        .unwrap();
    sender
        .send(JobStreamEvent {
            seq: 2,
            event: StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("done".to_string()),
            },
        })
        .unwrap();

    assert_eq!(stream.next().await.unwrap().seq, 1);
    assert_eq!(stream.next().await.unwrap().seq, 2);
    assert!(stream.next().await.is_none());
    assert_eq!(sender.receiver_count(), 0);
}

#[tokio::test]
async fn persisted_terminal_waits_for_the_live_finalization_barrier() {
    let (sender, receiver) = broadcast::channel(EVENT_BUFFER);
    let terminal = JobStreamEvent {
        seq: 1,
        event: StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("persisted first".to_string()),
        },
    };
    let mut stream =
        replay_and_live_job_event_stream(vec![terminal.clone()], RunStatus::Running, receiver, 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), stream.next())
            .await
            .is_err(),
        "persisted terminal must remain behind the live finalization barrier"
    );
    sender.send(terminal.clone()).unwrap();

    let emitted = stream.next().await.expect("terminal event");
    assert_eq!(emitted.seq, terminal.seq);
    assert!(matches!(
        emitted.event,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            ..
        }
    ));
    assert!(
        tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("terminal replay must close promptly")
            .is_none()
    );
    assert_eq!(sender.receiver_count(), 0);
}

#[tokio::test]
async fn already_replayed_live_terminal_still_closes_the_stream() {
    let (sender, receiver) = broadcast::channel(EVENT_BUFFER);
    let mut stream = replay_and_live_job_event_stream(Vec::new(), RunStatus::Running, receiver, 1);
    sender
        .send(JobStreamEvent {
            seq: 1,
            event: StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("already replayed".to_string()),
            },
        })
        .unwrap();

    assert!(
        tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("an already replayed terminal must still close live delivery")
            .is_none()
    );
    assert_eq!(sender.receiver_count(), 0);
}

async fn test_job_record(temp_dir: &tempfile::TempDir) -> (ApiState, Arc<JobRecord>, StateIndex) {
    let mut workspace = Workspace::detect(temp_dir.path()).unwrap();
    let mut config = AppConfig::default();
    config.rebase_to_workspace(&workspace.root);
    workspace.state_dir = config.state_dir();
    workspace.ensure_state_dir().unwrap();
    let state = ApiState::new(workspace.clone(), config.clone());
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let (completion, _) = watch::channel(false);
    let record = Arc::new(JobRecord {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        workspace,
        config,
        message: "test interaction".to_string(),
        content_blocks: Vec::new(),
        resumed_from_run_id: None,
        resume_state: None,
        product_session_id: None,
        product_store: None,
        attachment_storage: None,
        product_model_config: None,
        run_model_snapshot: None,
        status: Mutex::new(RunStatus::Running),
        events: Mutex::new(Vec::new()),
        pending_approvals: Mutex::new(HashMap::new()),
        pending_inputs: Mutex::new(HashMap::new()),
        tx,
        handle: Mutex::new(None),
        control: Mutex::new(None),
        control_event_trace: Mutex::new(None),
        control_event_trace_lock: Mutex::new(()),
        control_lifecycle_lock: Mutex::new(()),
        pending_product_events: Mutex::new(Vec::new()),
        completion,
        cancel_token: CancellationToken::new(),
    });
    let state_store = state_store_for_record(&record);
    let run_dir = state_store.run_store.run_dir(&record.run_id);
    state_store
        .index
        .record_run_started(
            record.session_id,
            record.job_id,
            record.run_id,
            &run_dir,
            &run_dir.join("trace.jsonl"),
        )
        .unwrap();
    state
        .inner
        .jobs
        .write()
        .await
        .insert(record.job_id, Arc::clone(&record));
    (state, record, state_store.index)
}

#[tokio::test]
async fn replay_snapshot_keeps_terminal_status_and_event_consistent() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (state, record, _) = test_job_record(&temp_dir).await;
    append_job_event(
        &record,
        StreamEvent::ModelStatus {
            status: "running".to_string(),
            message: "working".to_string(),
        },
    )
    .await;
    let terminal = append_job_event(
        &record,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("done".to_string()),
        },
    )
    .await;

    let (events, status) = persisted_or_live_events(&state, &record, 0).await.unwrap();
    assert_eq!(status, RunStatus::Done);
    assert!(matches!(
        events.last().map(|event| &event.event),
        Some(StreamEvent::RunCompleted { .. })
    ));

    let (events_after_terminal, status) = persisted_or_live_events(&state, &record, terminal.seq)
        .await
        .unwrap();
    assert!(events_after_terminal.is_empty());
    assert_eq!(status, RunStatus::Done);
}

#[tokio::test]
async fn shutdown_drain_waits_for_a_superseded_same_job_supervisor() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (state, first, _) = test_job_record(&temp_dir).await;
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let (completion, _) = watch::channel(false);
    let second = Arc::new(JobRecord {
        session_id: first.session_id,
        job_id: first.job_id,
        run_id: RunId::new(),
        workspace: first.workspace.clone(),
        config: first.config.clone(),
        message: "same job continuation".to_string(),
        content_blocks: Vec::new(),
        resumed_from_run_id: Some(first.run_id),
        resume_state: None,
        product_session_id: None,
        product_store: None,
        attachment_storage: None,
        product_model_config: None,
        run_model_snapshot: None,
        status: Mutex::new(RunStatus::Running),
        events: Mutex::new(Vec::new()),
        pending_approvals: Mutex::new(HashMap::new()),
        pending_inputs: Mutex::new(HashMap::new()),
        tx,
        handle: Mutex::new(None),
        control: Mutex::new(None),
        control_event_trace: Mutex::new(None),
        control_event_trace_lock: Mutex::new(()),
        control_lifecycle_lock: Mutex::new(()),
        pending_product_events: Mutex::new(Vec::new()),
        completion,
        cancel_token: state.inner.shutdown_token.child_token(),
    });

    let (release_first, first_released) = oneshot::channel::<()>();
    let first_completion = first.completion.clone();
    let first_handle = state.inner.supervisors.spawn(async move {
        let _guard = JobCompletionGuard::new(first_completion);
        let _ = first_released.await;
    });
    *first.handle.lock().await = Some(first_handle);

    let (release_second, second_released) = oneshot::channel::<()>();
    let second_completion = second.completion.clone();
    let second_handle = state.inner.supervisors.spawn(async move {
        let _guard = JobCompletionGuard::new(second_completion);
        let _ = second_released.await;
    });
    *second.handle.lock().await = Some(second_handle);
    state
        .inner
        .jobs
        .write()
        .await
        .insert(second.job_id, Arc::clone(&second));

    assert_eq!(
        live_job(&state, first.job_id).await.unwrap().run_id,
        second.run_id
    );
    let drain_state = state.clone();
    let drain = tokio::spawn(async move {
        drain_job_supervisors(&drain_state).await;
    });
    tokio::task::yield_now().await;
    assert!(!drain.is_finished());

    release_second.send(()).unwrap();
    wait_for_job_completion(&second).await;
    tokio::task::yield_now().await;
    assert!(
        !drain.is_finished(),
        "the superseded supervisor must remain part of graceful drain"
    );

    release_first.send(()).unwrap();
    drain.await.unwrap();

    assert!(*first.completion.borrow());
    assert!(*second.completion.borrow());
    assert!(state.inner.job_starts.is_closed());
    assert!(state.inner.job_starts.is_empty());
    assert!(state.inner.supervisors.is_closed());
    assert!(state.inner.supervisors.is_empty());
    let first_handle = first.handle.lock().await.take().unwrap();
    first_handle.await.unwrap();
    assert!(second.handle.lock().await.is_none());
}

/// Drive the shutdown drain to completion under a bounded deadline, or
/// report the tracker state that was still holding it.
///
/// `drain_job_supervisors` waits for the tracked job starts and then for the
/// supervisors they hand jobs to, and a supervisor runs the whole job to a
/// durable terminal state. A budget that only covers startup therefore
/// measures the runner's load instead of the contract under test: the same
/// drain has taken longer than five seconds on a shared CI runner. The
/// deadline is generous, but it is still an assertion — exhausting it fails
/// with what was still outstanding, so a drain that never completes cannot
/// pass.
async fn drain_job_supervisors_within(state: &ApiState, budget: Duration) -> Result<(), String> {
    let drain = drain_job_supervisors(state);
    tokio::pin!(drain);
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        tokio::select! {
            _ = &mut drain => return Ok(()),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(format!(
                        "job_starts pending={} closed={}, supervisors pending={} closed={}",
                        state.inner.job_starts.len(),
                        state.inner.job_starts.is_closed(),
                        state.inner.supervisors.len(),
                        state.inner.supervisors.is_closed(),
                    ));
                }
            }
        }
    }
}

#[tokio::test]
async fn dropped_job_start_waiter_does_not_cancel_a_blocked_product_claim() {
    let server = tempfile::TempDir::new().unwrap();
    let workspace_root = server.path().join("product-workspace");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace = Workspace::detect(server.path()).unwrap();
    let mut config = AppConfig::default();
    config.state.state_dir = PathBuf::from("api-state");
    config.state.sqlite_busy_timeout_ms = 5_000;
    let user_paths = UserConfigPaths::from_root(server.path().join("user-config"));
    let mut user_document = rove_app_bootstrap::UserConfigDocument::default();
    user_document.provider.profiles.insert(
        "test-fake".to_string(),
        config.provider.profiles["default"].clone(),
    );
    user_document.model.default_profile = Some("test-fake".to_string());
    user_document.model.default_model = Some("fake".to_string());
    rove_app_bootstrap::UserConfigWriter::new(user_paths.clone())
        .update(None, &user_document)
        .unwrap();
    config.source_summary.user_config_path = user_paths.config_file;
    let state = ApiState::new(workspace, config);
    let store = state.product_store().unwrap();
    let product_workspace = store
        .create_workspace(CreateProductWorkspaceRequest {
            root: workspace_root,
            kind: ProductWorkspaceKind::Folder,
            display_name: Some("Tracked start".to_string()),
            pinned: false,
        })
        .await
        .unwrap();
    let product_session = store
        .create_session(CreateProductSessionRequest {
            workspace_id: product_workspace.id.clone(),
            title: Some("Disconnect during claim".to_string()),
        })
        .await
        .unwrap();
    let test_profile_id = ProductProviderProfileId::from_catalog_id("test-fake").unwrap();
    store
        .upsert_provider_catalog_identity(
            &test_profile_id,
            "Test Fake",
            ProductProviderType::Fake,
            &user_document.revision(),
        )
        .await
        .unwrap();
    store
        .update_session_model_config(
            &product_session.id,
            UpdateProductSessionModelConfigRequest {
                profile_id: Some(test_profile_id),
                model: "fake".to_string(),
                reasoning: ProductReasoningPreference::Default,
                max_steps: DEFAULT_PRODUCT_MAX_STEPS,
                expected_revision: None,
            },
        )
        .await
        .unwrap();

    let blocker = rusqlite::Connection::open(state.product_store_path()).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let response = start_tracked_job(
        state.clone(),
        CreateJobRequest {
            message: "finish after the response waiter disconnects".to_string(),
            model: None,
            max_steps: None,
            agent: None,
            approval: None,
            resume: None,
            workspace: None,
            provider: None,
            product_session_id: Some(product_session.id.clone()),
        },
    );
    assert_eq!(state.inner.job_starts.len(), 1);
    tokio::time::sleep(Duration::from_millis(25)).await;
    drop(response);
    assert_eq!(
        state.inner.job_starts.len(),
        1,
        "dropping the HTTP response waiter must not cancel the owned start task"
    );

    blocker.execute_batch("ROLLBACK").unwrap();
    drop(blocker);
    // The drain covers the rest of the blocked claim *and* the whole job the
    // freed start hands to a supervisor — provider stream, artifact
    // finalization — so the budget has to cover real work on a shared
    // runner, not only task startup unwinding. Sixty seconds is twelve times
    // the budget that flaked, and it is still an assertion: the helper fails
    // when it is exhausted, and the durable assertions below still require
    // the job to have actually run.
    const DRAIN_BUDGET: Duration = Duration::from_secs(60);
    assert_eq!(
        drain_job_supervisors_within(&state, DRAIN_BUDGET).await,
        Ok(()),
        "the tracked job start and its supervisor must drain after the database unlock",
    );

    let sessions = store
        .list_all_sessions(&product_workspace.id)
        .await
        .unwrap();
    let session = sessions
        .into_iter()
        .find(|session| session.id == product_session.id)
        .expect("product session");
    assert_eq!(session.status, ProductSessionStatus::Idle);
    assert!(session.runtime_binding.is_some());
    assert_eq!(
        store
            .list_run_bindings(&product_session.id)
            .await
            .unwrap()
            .len(),
        1
    );
    let claim = store.claim_session_turn(&product_session.id).await.unwrap();
    store
        .finish_session_turn(&claim.claim_id, claim.previous_status)
        .await
        .unwrap();
}

#[tokio::test]
async fn shutdown_drains_job_starts_before_supervisors() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp_dir.path()).unwrap();
    let state = ApiState::new(workspace, AppConfig::default());
    let supervisors = state.inner.supervisors.clone();
    let (start_entered_tx, start_entered_rx) = oneshot::channel();
    let (release_start_tx, release_start_rx) = oneshot::channel();
    let (supervisor_entered_tx, supervisor_entered_rx) = oneshot::channel();
    let (release_supervisor_tx, release_supervisor_rx) = oneshot::channel();
    drop(state.inner.job_starts.spawn(async move {
        let _ = start_entered_tx.send(());
        let _ = release_start_rx.await;
        drop(supervisors.spawn(async move {
            let _ = supervisor_entered_tx.send(());
            let _ = release_supervisor_rx.await;
        }));
    }));
    start_entered_rx.await.unwrap();

    let drain_state = state.clone();
    let drain = tokio::spawn(async move {
        drain_job_supervisors(&drain_state).await;
    });
    tokio::task::yield_now().await;
    assert!(state.inner.job_starts.is_closed());
    assert!(!state.inner.supervisors.is_closed());
    assert!(!drain.is_finished());

    release_start_tx.send(()).unwrap();
    supervisor_entered_rx.await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while !state.inner.supervisors.is_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("supervisor tracker should close after all job starts drain");
    assert!(!drain.is_finished());

    release_supervisor_tx.send(()).unwrap();
    drain.await.unwrap();
    assert!(state.inner.job_starts.is_empty());
    assert!(state.inner.supervisors.is_empty());
}

#[tokio::test]
async fn submit_input_conflicts_when_responder_was_dropped() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (state, record, index) = test_job_record(&temp_dir).await;
    let input_id = CallId::new();
    let request = UserInputRequest {
        prompt: "Which branch?".to_string(),
    };
    index
        .record_pending_input(input_id, record.job_id, record.run_id, &request.prompt)
        .unwrap();
    let (tx, rx) = oneshot::channel();
    drop(rx);
    record
        .pending_inputs
        .lock()
        .await
        .insert(input_id, PendingInput { request, tx });

    let error = submit_input(
        State(state),
        Path((record.job_id, input_id)),
        Ok(Json(SubmitInputRequest {
            answer: "main".to_string(),
        })),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(
        index.pending_input_status(input_id).unwrap().as_deref(),
        Some("cancelled")
    );
}

#[tokio::test]
async fn submit_approval_conflicts_when_responder_was_dropped() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (state, record, index) = test_job_record(&temp_dir).await;
    let call_id = CallId::new();
    let request = ToolApprovalRequest {
        call_id,
        name: "write_file".to_string(),
        args: serde_json::json!({"path": "result.txt"}),
        reason: "writes a file".to_string(),
    };
    index
        .record_pending_approval(
            call_id,
            record.job_id,
            record.run_id,
            &request.name,
            &request.args.to_string(),
            &request.reason,
        )
        .unwrap();
    let (tx, rx) = oneshot::channel();
    drop(rx);
    record
        .pending_approvals
        .lock()
        .await
        .insert(call_id, PendingApproval { request, tx });

    let error = submit_approval(
        State(state),
        Path((record.job_id, call_id)),
        Ok(Json(SubmitApprovalRequest {
            decision: ApprovalDecision::Approve,
        })),
    )
    .await
    .unwrap_err();

    assert_eq!(error.status, StatusCode::CONFLICT);
    assert_eq!(
        index.pending_approval_status(call_id).unwrap().as_deref(),
        Some("cancelled")
    );
}

#[tokio::test]
async fn api_input_registration_cancellation_marks_late_sqlite_insert_cancelled() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (state, record, index) = test_job_record(&temp_dir).await;
    let connection = rusqlite::Connection::open(index.path()).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();

    let provider = ApiInputProvider {
        record: Arc::clone(&record),
        index: index.clone(),
    };
    let input_id = CallId::new();
    let handle = tokio::spawn(async move {
        provider
            .begin_input(
                input_id,
                UserInputRequest {
                    prompt: "blocked registration".to_string(),
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !handle.is_finished(),
        "registration should be waiting on SQLite"
    );

    record.cancel_token.cancel();
    handle.abort();
    drop(connection);

    for _ in 0..100 {
        if index.pending_input_status(input_id).unwrap().as_deref() == Some("cancelled") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        index.pending_input_status(input_id).unwrap().as_deref(),
        Some("cancelled")
    );
    assert!(record.pending_inputs.lock().await.is_empty());
    drop(state);
}

#[tokio::test]
async fn api_approval_registration_cancellation_marks_late_sqlite_insert_cancelled() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (state, record, index) = test_job_record(&temp_dir).await;
    let connection = rusqlite::Connection::open(index.path()).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();

    let provider = ApiApprovalProvider {
        record: Arc::clone(&record),
        index: index.clone(),
    };
    let call_id = CallId::new();
    let handle = tokio::spawn(async move {
        provider
            .begin_approval(ToolApprovalRequest {
                call_id,
                name: "write_file".to_string(),
                args: serde_json::json!({"path": "blocked.txt"}),
                reason: "writes a file".to_string(),
            })
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !handle.is_finished(),
        "registration should be waiting on SQLite"
    );

    record.cancel_token.cancel();
    handle.abort();
    drop(connection);

    for _ in 0..100 {
        if index.pending_approval_status(call_id).unwrap().as_deref() == Some("cancelled") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        index.pending_approval_status(call_id).unwrap().as_deref(),
        Some("cancelled")
    );
    assert!(record.pending_approvals.lock().await.is_empty());
    drop(state);
}

#[tokio::test]
async fn api_interaction_registration_fails_closed_when_initial_sqlite_insert_fails() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let (state, record, index) = test_job_record(&temp_dir).await;
    let short_timeout_index =
        StateIndex::with_path(&record.workspace.state_dir, index.path().to_path_buf(), 20);

    let connection = rusqlite::Connection::open(index.path()).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let call_id = CallId::new();
    let approval = ApiApprovalProvider {
        record: Arc::clone(&record),
        index: short_timeout_index.clone(),
    }
    .begin_approval(ToolApprovalRequest {
        call_id,
        name: "write_file".to_string(),
        args: serde_json::json!({"path": "blocked.txt"}),
        reason: "writes a file".to_string(),
    })
    .await;
    assert!(matches!(
        approval,
        Err(ToolError::ExecutionFailed { ref reason }) if reason.contains("persist pending approval")
    ));
    assert!(record.pending_approvals.lock().await.is_empty());
    drop(connection);
    assert_eq!(index.pending_approval_status(call_id).unwrap(), None);

    let connection = rusqlite::Connection::open(index.path()).unwrap();
    connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let input_id = CallId::new();
    let input = ApiInputProvider {
        record: Arc::clone(&record),
        index: short_timeout_index,
    }
    .begin_input(
        input_id,
        UserInputRequest {
            prompt: "blocked input".to_string(),
        },
    )
    .await;
    assert!(matches!(
        input,
        Err(ToolError::ExecutionFailed { ref reason }) if reason.contains("persist pending input")
    ));
    assert!(record.pending_inputs.lock().await.is_empty());
    drop(connection);
    assert_eq!(index.pending_input_status(input_id).unwrap(), None);
    drop(state);
}

#[test]
fn api_state_registers_the_bearer_token_as_a_credential() {
    let temp_dir = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp_dir.path()).unwrap();
    // A token that matches no pattern the export backstop knows, so only
    // the assembly-time registration can make the runtime aware of it.
    let token = "api-bearer-credential-canary-4d17b2";
    let mut config = AppConfig::default();
    config.api.token_auth = Some(token.to_string());

    let _state = ApiState::new(workspace, config);

    let secrets = rove_runtime::secrets::registry();
    assert!(secrets.is_known_value(token));
    assert_eq!(
        secrets.redact_text(&format!("Authorization: Bearer {token}")),
        format!(
            "Authorization: Bearer {}",
            rove_runtime::secrets::KNOWN_SECRET_MARKER
        )
    );
}

#[tokio::test]
async fn the_error_envelope_never_carries_a_known_credential() {
    // Every `ApiError` leaves through this one conversion, including the
    // typed rejections that echo back the value a caller sent.
    let canary = "api-error-envelope-canary-9e30c5";
    assert!(rove_runtime::secrets::registry().register_value(canary));
    let response = axum::response::IntoResponse::into_response(ApiError::bad_request(format!(
        "provider rejected credential {canary}"
    )));
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(!text.contains(canary), "{text}");
    assert!(
        text.contains(rove_runtime::secrets::KNOWN_SECRET_MARKER),
        "{text}"
    );
    // The envelope keeps its shape, so a client's error handling is
    // unaffected by the redaction.
    let decoded: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(decoded["code"], "bad_request");
    assert_eq!(
        decoded["error"],
        format!(
            "provider rejected credential {}",
            rove_runtime::secrets::KNOWN_SECRET_MARKER
        )
    );
}

#[tokio::test]
async fn an_error_message_the_runtime_knows_nothing_about_is_left_alone() {
    let response = axum::response::IntoResponse::into_response(ApiError::bad_request(
        "plain bounded rejection",
    ));
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let decoded: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Redaction must not become a mangler: text the runtime has no
    // authority over crosses the boundary unchanged.
    assert_eq!(decoded["error"], "plain bounded rejection");
}
