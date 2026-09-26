//! Offline contract tests for the run loop's model-call retry budget and
//! silent-turn recovery.
//!
//! Every case runs against the deterministic Fake provider, so the suite needs
//! no provider key and no network. Two deliberate choices keep it fast and
//! exact:
//!
//! - the backoff base and ceiling are one and four milliseconds, so a test
//!   never waits for a production-scale delay;
//! - jitter is disabled, so `delay_ms` is a contract fact a test can assert
//!   instead of a range it has to tolerate.
//!
//! The production defaults are covered separately by the runtime policy unit
//! tests and by the configured-policy projection tests in the bootstrap crate.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use rove_core::ToolRegistry;
use rove_models::{
    FakeModelClient, FakeTurn, Message, ModelClient, ModelError, ModelEvent, ModelToolSchema,
    ProviderCapabilities,
};
use rove_runtime::context::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig, ProviderRetryPolicy, SilentTurnRecoveryPolicy};
use rove_runtime::events::StreamEvent;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{JobId, Role, RunId, RunRequest, SessionId, TerminationReason};
use rove_runtime::workspace::Workspace;
use tokio_util::sync::CancellationToken;

/// The fixed nudge text the runtime injects before a recovery turn. Pinned as a
/// literal so the test proves the wire contract rather than the constant.
const NUDGE: &str = "Your previous turn produced no visible response. Continue: either finish the task or summarize the progress you have so far.";

/// One-millisecond base delay, four-millisecond ceiling, no jitter.
fn fast_policy() -> ProviderRetryPolicy {
    ProviderRetryPolicy {
        backoff_base_ms: 1,
        backoff_max_ms: 4,
        jitter_ratio: 0.0,
        ..ProviderRetryPolicy::default()
    }
}

fn engine_with_turns(turns: Vec<FakeTurn>, policy: ProviderRetryPolicy) -> Engine {
    engine_with_turns_and_plan(turns, policy, false)
}

fn engine_with_turns_and_plan(
    turns: Vec<FakeTurn>,
    policy: ProviderRetryPolicy,
    plan_enabled: bool,
) -> Engine {
    let model = Box::new(FakeModelClient::with_turns(
        "unscripted fallback response".to_string(),
        turns,
    ));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(rove_runtime::tools::echo::EchoTool));
    Engine::new(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig::new(8, plan_enabled).with_provider_retry(policy),
    )
}

/// A scripted client whose final prompt and call count stay observable after the
/// Engine has taken ownership of it.
#[derive(Clone)]
struct ObservableFake {
    inner: Arc<FakeModelClient>,
}

impl ObservableFake {
    fn with_turns(turns: Vec<FakeTurn>) -> Self {
        Self {
            inner: Arc::new(FakeModelClient::with_turns(
                "unscripted fallback response".to_string(),
                turns,
            )),
        }
    }

    fn call_count(&self) -> usize {
        self.inner.call_count()
    }

    fn last_messages(&self) -> Vec<Message> {
        self.inner.last_messages().expect("the model was called")
    }

    fn nudges_in_last_prompt(&self) -> usize {
        self.last_messages()
            .iter()
            .filter(|message| message.content == NUDGE)
            .count()
    }
}

#[async_trait]
impl ModelClient for ObservableFake {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        self.inner.stream(messages, tools)
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }

    fn history_protocol(&self) -> String {
        self.inner.history_protocol()
    }

    fn compatibility_text_tool_calls(&self) -> bool {
        self.inner.compatibility_text_tool_calls()
    }

    fn requires_terminal_event(&self) -> bool {
        self.inner.requires_terminal_event()
    }

    fn client_id(&self) -> rove_models::ModelClientId {
        self.inner.client_id()
    }
}

/// Engine in front of an observable scripted client, with silent-turn recovery
/// explicitly configured.
fn engine_with_observation(
    turns: Vec<FakeTurn>,
    silent_turn_recovery: SilentTurnRecoveryPolicy,
) -> (Engine, ObservableFake) {
    let model = ObservableFake::with_turns(turns);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(rove_runtime::tools::echo::EchoTool));
    let engine = Engine::new(
        Box::new(model.clone()),
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig::new(8, false)
            .with_provider_retry(fast_policy())
            .with_silent_turn_recovery(silent_turn_recovery),
    );
    (engine, model)
}

async fn collect(engine: &Engine, message: &str) -> Vec<StreamEvent> {
    collect_with_cancel(engine, message, CancellationToken::new()).await
}

async fn collect_with_cancel(
    engine: &Engine,
    message: &str,
    cancel: CancellationToken,
) -> Vec<StreamEvent> {
    let stream = engine.run_with_cancel(
        RunRequest {
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message: message.to_string(),
            resume_state: None,
        },
        None,
        cancel,
    );
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

fn recovery_notices(events: &[StreamEvent]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ModelStatus { status, message } if status == "recovering_silent_turn" => {
                Some((status.clone(), message.clone()))
            }
            _ => None,
        })
        .collect()
}

fn silent_turn_degradations(events: &[StreamEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ExecutionDegraded { record } if record.code == "silent_turn_recovery" => {
                Some(record.code.clone())
            }
            _ => None,
        })
        .collect()
}

fn final_output(events: &[StreamEvent]) -> Option<String> {
    events.iter().find_map(|event| match event {
        StreamEvent::RunCompleted { reason, output } if *reason == TerminationReason::Final => {
            Some(output.clone().unwrap_or_default())
        }
        _ => None,
    })
}

fn llm_messages(events: &[StreamEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::LlmMessage { full, .. } => Some(full.clone()),
            _ => None,
        })
        .collect()
}

fn retries(events: &[StreamEvent]) -> Vec<(u32, u32, u64, String, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ProviderRetry {
                attempt,
                max_attempts,
                delay_ms,
                reason,
                phase,
            } => Some((
                *attempt,
                *max_attempts,
                *delay_ms,
                reason.clone(),
                phase.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn terminated_with(events: &[StreamEvent], reason: TerminationReason) -> bool {
    events
        .iter()
        .any(|event| matches!(event, StreamEvent::RunCompleted { reason: actual, .. } if *actual == reason))
}

fn saw_text(events: &[StreamEvent], needle: &str) -> bool {
    events.iter().any(|event| match event {
        StreamEvent::LlmChunk { delta } => delta.contains(needle),
        StreamEvent::LlmMessage { full, .. } => full.contains(needle),
        _ => false,
    })
}

#[tokio::test]
async fn provider_retry_after_is_honoured_and_the_turn_recovers() {
    let policy = ProviderRetryPolicy {
        rate_limit_max_attempts: 3,
        // Wide enough that the provider's own instruction, not the ceiling,
        // decides the wait; the ceiling clamp has its own policy unit test.
        backoff_max_ms: 50,
        ..fast_policy()
    };
    let engine = engine_with_turns(
        vec![
            FakeTurn::Fail(ModelError::RateLimited { retry_after_ms: 7 }),
            FakeTurn::Text("recovered after throttling".to_string()),
        ],
        policy,
    );

    let events = collect(&engine, "answer the request").await;

    assert_eq!(
        retries(&events),
        vec![(
            2,
            3,
            7,
            "rate_limited".to_string(),
            "model_call".to_string()
        )],
        "the provider's retry_after must be honoured exactly and reported once"
    );
    assert!(
        saw_text(&events, "recovered after throttling"),
        "the retried attempt must produce the run's answer"
    );
    assert!(
        terminated_with(&events, TerminationReason::Final),
        "a recovered turn must complete the run"
    );
}

#[tokio::test]
async fn transient_failures_spend_the_budget_and_then_report_the_error() {
    let policy = ProviderRetryPolicy {
        transient_max_attempts: 4,
        ..fast_policy()
    };
    let engine = engine_with_turns(
        vec![
            FakeTurn::Fail(ModelError::RequestFailed("first".to_string())),
            FakeTurn::Fail(ModelError::RequestFailed("second".to_string())),
            FakeTurn::Fail(ModelError::RequestFailed("third".to_string())),
            FakeTurn::Fail(ModelError::RequestFailed("fourth".to_string())),
            FakeTurn::Text("must never be reached".to_string()),
        ],
        policy,
    );

    let events = collect(&engine, "answer the request").await;

    let retries = retries(&events);
    assert_eq!(
        retries.len(),
        3,
        "four total attempts mean three retries, got {retries:?}"
    );
    assert_eq!(
        retries
            .iter()
            .map(|(attempt, max, delay, reason, phase)| (
                *attempt,
                *max,
                *delay,
                reason.as_str(),
                phase.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (2, 4, 1, "transient:request_failed", "model_call"),
            (3, 4, 2, "transient:request_failed", "model_call"),
            (4, 4, 4, "transient:request_failed", "model_call"),
        ],
        "backoff must double from the base and the reason must stay whitelisted"
    );
    assert!(
        !saw_text(&events, "must never be reached"),
        "an exhausted budget must not call the provider again"
    );
    assert!(
        terminated_with(&events, TerminationReason::Error),
        "an exhausted budget must surface a typed failure"
    );
}

#[tokio::test]
async fn rate_limit_and_transient_failures_use_independent_budgets() {
    let policy = ProviderRetryPolicy {
        rate_limit_max_attempts: 2,
        transient_max_attempts: 3,
        ..fast_policy()
    };
    let engine = engine_with_turns(
        vec![
            FakeTurn::Fail(ModelError::RateLimited { retry_after_ms: 3 }),
            FakeTurn::Fail(ModelError::RequestFailed("connection reset".to_string())),
            FakeTurn::Fail(ModelError::StreamInterrupted("stream closed".to_string())),
            FakeTurn::Text("recovered across both classes".to_string()),
        ],
        policy,
    );

    let events = collect(&engine, "answer the request").await;

    assert_eq!(
        retries(&events)
            .iter()
            .map(|(attempt, max, delay, reason, _)| (*attempt, *max, *delay, reason.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (2, 2, 3, "rate_limited"),
            (2, 3, 1, "transient:request_failed"),
            (3, 3, 2, "transient:stream_interrupted"),
        ],
        "each class must spend its own budget and report its own reason"
    );
    assert!(
        saw_text(&events, "recovered across both classes"),
        "independent budgets must not starve the recovery attempt"
    );
}

#[tokio::test]
async fn a_turn_that_already_produced_output_is_never_retried() {
    let policy = ProviderRetryPolicy {
        transient_max_attempts: 4,
        ..fast_policy()
    };
    let engine = engine_with_turns(
        vec![
            FakeTurn::TextThenFail {
                text: "half an answer".to_string(),
                error: ModelError::StreamInterrupted("socket closed".to_string()),
            },
            FakeTurn::Text("a retry would duplicate the visible text".to_string()),
        ],
        policy,
    );

    let events = collect(&engine, "answer the request").await;

    assert!(
        retries(&events).is_empty(),
        "a failure after streamed text must not spend a retry"
    );
    assert!(
        saw_text(&events, "half an answer"),
        "the partial answer must stay visible"
    );
    assert!(
        !saw_text(&events, "a retry would duplicate the visible text"),
        "the provider must not be called again after output"
    );
    assert!(
        terminated_with(&events, TerminationReason::Error),
        "the turn must fail once a retry is no longer safe"
    );
}

#[tokio::test]
async fn cancelling_during_backoff_ends_the_turn_without_another_call() {
    let policy = ProviderRetryPolicy {
        backoff_base_ms: 5_000,
        backoff_max_ms: 5_000,
        jitter_ratio: 0.0,
        ..ProviderRetryPolicy::default()
    };
    let engine = engine_with_turns(
        vec![
            FakeTurn::Fail(ModelError::RequestFailed("connection reset".to_string())),
            FakeTurn::Text("must never be reached after cancellation".to_string()),
        ],
        policy,
    );

    let mut stream = engine.run(
        RunRequest {
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message: "answer the request".to_string(),
            resume_state: None,
        },
        None,
    );

    let mut events = Vec::new();
    let mut cancelled = false;
    let started = std::time::Instant::now();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
        let is_retry = matches!(event, StreamEvent::ProviderRetry { .. });
        events.push(event);
        if is_retry {
            // Cancel while the runtime is sleeping before attempt two.
            stream.cancel();
            cancelled = true;
            break;
        }
    }
    assert!(
        cancelled,
        "the retry notice must be observable before the wait"
    );
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
        events.push(event);
    }

    assert!(
        started.elapsed() < Duration::from_secs(4),
        "cancellation must abort the backoff instead of waiting it out, took {:?}",
        started.elapsed()
    );
    assert!(
        terminated_with(&events, TerminationReason::Cancelled),
        "cancelling during backoff must terminate the run as cancelled"
    );
    assert!(
        !saw_text(&events, "must never be reached after cancellation"),
        "no further attempt may be sent after cancellation"
    );
}

#[tokio::test]
async fn disabling_the_budget_restores_immediate_failure() {
    let engine = engine_with_turns(
        vec![
            FakeTurn::Fail(ModelError::RequestFailed("connection reset".to_string())),
            FakeTurn::Text("must never be reached while the budget is disabled".to_string()),
        ],
        ProviderRetryPolicy::disabled(),
    );

    let events = collect(&engine, "answer the request").await;

    assert!(
        retries(&events).is_empty(),
        "a disabled budget must not emit retry notices"
    );
    assert!(
        !saw_text(
            &events,
            "must never be reached while the budget is disabled"
        ),
        "a disabled budget must keep the provider call count at one"
    );
    assert!(
        terminated_with(&events, TerminationReason::Error),
        "a disabled budget must keep the immediate failure contract"
    );
}

#[tokio::test]
async fn retry_notices_never_carry_provider_payloads() {
    let policy = ProviderRetryPolicy {
        transient_max_attempts: 2,
        ..fast_policy()
    };
    let engine = engine_with_turns(
        vec![
            FakeTurn::Fail(ModelError::RequestFailed(
                "Bearer sk-live-do-not-leak against https://internal.example".to_string(),
            )),
            FakeTurn::Text("ok".to_string()),
        ],
        policy,
    );

    let events = collect(&engine, "answer the request").await;

    let notices: Vec<String> = events
        .iter()
        .filter(|event| matches!(event, StreamEvent::ProviderRetry { .. }))
        .map(|event| serde_json::to_string(event).expect("event should serialize"))
        .collect();
    assert_eq!(notices.len(), 1, "expected exactly one retry notice");
    for notice in &notices {
        assert!(
            !notice.contains("sk-live-do-not-leak") && !notice.contains("internal.example"),
            "retry notices must not carry provider payloads: {notice}"
        );
        assert!(
            notice.contains("\"reason\":\"transient:request_failed\""),
            "retry notices must carry the whitelisted reason: {notice}"
        );
    }
}

#[tokio::test]
async fn a_non_retryable_failure_is_terminal_on_its_first_attempt() {
    // Auth, context-length, and invalid-configuration failures have no retry
    // class. The budget must not treat them as a class whose allowance is spent
    // on the first call, and no notice may be emitted for them.
    let policy = ProviderRetryPolicy {
        rate_limit_max_attempts: 4,
        transient_max_attempts: 4,
        ..fast_policy()
    };
    let engine = engine_with_turns(
        vec![
            FakeTurn::Fail(ModelError::AuthFailed),
            FakeTurn::Fail(ModelError::ContextLengthExceeded { used: 9, max: 8 }),
            FakeTurn::Text("must never be reached".to_string()),
        ],
        policy,
    );

    let events = collect(&engine, "answer the request").await;

    assert!(
        retries(&events).is_empty(),
        "a failure with no retry class must not schedule a retry: {:?}",
        retries(&events)
    );
    assert!(
        !saw_text(&events, "must never be reached"),
        "the turn must end on the first non-retryable failure"
    );
    assert!(
        terminated_with(&events, TerminationReason::Error),
        "a non-retryable failure must keep the immediate failure contract"
    );
}

#[tokio::test]
async fn planned_step_turns_spend_the_same_budget() {
    let policy = ProviderRetryPolicy {
        transient_max_attempts: 2,
        ..fast_policy()
    };
    let engine = engine_with_turns_and_plan(
        vec![
            FakeTurn::Text(
                r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#.to_string(),
            ),
            FakeTurn::Fail(ModelError::RequestFailed("step call failed".to_string())),
            FakeTurn::Text("step answer".to_string()),
        ],
        policy,
        true,
    );

    let events = collect(&engine, "fix the docs").await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::PlanCreated { .. })),
        "planned mode must still create a plan"
    );
    assert_eq!(
        retries(&events),
        vec![(
            2,
            2,
            1,
            "transient:request_failed".to_string(),
            "model_call".to_string()
        )],
        "a planned step's model turn must spend the same budget"
    );
    assert!(
        saw_text(&events, "step answer"),
        "the retried planned step must use the retry's response"
    );
}

// ---------------------------------------------------------------------------
// R2a silent-turn recovery
// ---------------------------------------------------------------------------

/// A silent turn is one model call that produced no text and no tool call. The
/// scripted client reproduces it exactly: an empty assistant turn still reaches
/// the run loop as a normal `Final`, which is what makes it silent.
#[tokio::test]
async fn a_silent_turn_is_recovered_once_with_the_fixed_nudge() {
    let (engine, model) = engine_with_observation(
        vec![
            FakeTurn::Text(String::new()),
            FakeTurn::Text("recovered answer".to_string()),
        ],
        SilentTurnRecoveryPolicy::default(),
    );

    let events = collect(&engine, "answer the request").await;

    assert_eq!(
        recovery_notices(&events),
        vec![("recovering_silent_turn".to_string(), NUDGE.to_string())],
        "exactly one recovery turn must be announced, carrying the fixed nudge"
    );
    assert_eq!(
        silent_turn_degradations(&events),
        vec!["silent_turn_recovery".to_string()],
        "a spent recovery turn must be an explicit, canonical degradation fact"
    );
    assert_eq!(
        model.call_count(),
        2,
        "recovery is one extra model turn, not a loop"
    );
    assert_eq!(
        model.nudges_in_last_prompt(),
        1,
        "the recovery turn's conversation must contain the nudge exactly once"
    );
    assert_eq!(
        llm_messages(&events),
        vec![String::new(), "recovered answer".to_string()],
        "the silent turn and the recovery answer are the run's two model messages"
    );
    assert_eq!(
        final_output(&events).as_deref(),
        Some("recovered answer"),
        "the recovery answer becomes the run's final answer"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::LlmChunk { .. }))
            .count(),
        1,
        "only the recovery turn streams text to the user"
    );
}

#[tokio::test]
async fn two_silent_turns_terminate_after_one_recovery_attempt() {
    let (engine, model) = engine_with_observation(
        vec![
            FakeTurn::Text(String::new()),
            FakeTurn::Text(String::new()),
            FakeTurn::Text("must never be reached".to_string()),
        ],
        SilentTurnRecoveryPolicy::default(),
    );

    let events = collect(&engine, "answer the request").await;

    assert_eq!(recovery_notices(&events).len(), 1);
    assert_eq!(silent_turn_degradations(&events).len(), 1);
    assert_eq!(
        model.call_count(),
        2,
        "a second silent turn must terminate instead of spending a third turn"
    );
    assert_eq!(
        final_output(&events).as_deref(),
        Some(""),
        "a run that stays silent terminates exactly as it did before recovery"
    );
    assert!(!saw_text(&events, "must never be reached"));
}

#[tokio::test]
async fn zero_attempts_reproduces_the_pre_recovery_stream_exactly() {
    let (engine, model) = engine_with_observation(
        vec![
            FakeTurn::Text(String::new()),
            FakeTurn::Text("must never be reached".to_string()),
        ],
        SilentTurnRecoveryPolicy { max_attempts: 0 },
    );

    let events = collect(&engine, "answer the request").await;

    assert_eq!(
        model.call_count(),
        1,
        "disabling silent-turn recovery must not spend a second model call"
    );
    assert!(recovery_notices(&events).is_empty());
    assert!(silent_turn_degradations(&events).is_empty());
    assert_eq!(model.nudges_in_last_prompt(), 0);
    assert_eq!(llm_messages(&events), vec![String::new()]);
    assert_eq!(final_output(&events).as_deref(), Some(""));

    // The whole stream is the pre-recovery stream: the same event names in the
    // same order, and nothing mentioning recovery anywhere in its payloads.
    let names: Vec<&'static str> = events.iter().map(StreamEvent::event_name).collect();
    assert_eq!(
        names,
        vec![
            "run_started",
            "agent_profile_activated",
            "execution_strategy_selected",
            "prompt_built",
            "model_status",
            "llm_message",
            "execution_budget_updated",
            "finalization_started",
            "finalization_completed",
            "run_completed",
        ],
        "disabling silent-turn recovery must not add, drop, or reorder an event"
    );
    for event in &events {
        let json = serde_json::to_string(event).expect("events serialize");
        assert!(
            !json.contains("silent_turn") && !json.contains("recovering_silent_turn"),
            "no recovery fact may appear when recovery is disabled: {json}"
        );
    }
}

#[tokio::test]
async fn a_recovery_turn_moves_through_the_normal_tool_path() {
    // A silent first turn is recovered, and the recovery turn decides to use a
    // tool. The tool still goes through the ordinary dispatch path and its
    // result still reaches the next model turn; recovery grants no shortcut.
    let (engine, model) = engine_with_observation(
        vec![
            FakeTurn::Text(String::new()),
            FakeTurn::ToolUse {
                id: "call-1".to_string(),
                name: "echo".to_string(),
                args: serde_json::json!({"message": "through the normal path"}),
            },
            FakeTurn::Text("done after the tool".to_string()),
        ],
        SilentTurnRecoveryPolicy::default(),
    );

    let events = collect(&engine, "answer the request").await;

    assert_eq!(recovery_notices(&events).len(), 1);
    assert!(
        events.iter().any(|event| matches!(
            event,
            StreamEvent::ToolCallStarted { name, .. } if name == "echo"
        )),
        "the recovery turn's tool call must be dispatched: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolCallCompleted { .. })),
        "the recovery turn's tool call must produce a normal result"
    );
    assert_eq!(
        model.call_count(),
        3,
        "silent turn, recovery turn with a tool call, and the turn that sees the result"
    );
    let last_prompt = model.last_messages();
    assert!(
        last_prompt
            .iter()
            .any(|message| message.content.contains("through the normal path")),
        "the tool result must reach the model through the ordinary history"
    );
    assert_eq!(
        last_prompt
            .iter()
            .filter(|message| message.content == NUDGE)
            .count(),
        1,
        "the nudge must not be re-injected on later turns"
    );
    assert_eq!(
        final_output(&events).as_deref(),
        Some("done after the tool")
    );
}

#[tokio::test]
async fn cancellation_during_the_recovery_turn_cancels_normally() {
    let (engine, model) = engine_with_observation(
        vec![
            FakeTurn::Text(String::new()),
            FakeTurn::Text("must not be delivered".to_string()),
        ],
        SilentTurnRecoveryPolicy::default(),
    );
    let cancel = CancellationToken::new();
    let stream = engine.run_with_cancel(
        RunRequest {
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message: "answer the request".to_string(),
            resume_state: None,
        },
        None,
        cancel.clone(),
    );
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        let recovering = matches!(&event, StreamEvent::ModelStatus { status, .. } if status == "recovering_silent_turn");
        events.push(event);
        if recovering {
            // Cancel as the recovery turn begins: the recovery turn is an
            // ordinary turn, so it must observe cancellation like any other.
            cancel.cancel();
        }
    }

    assert_eq!(recovery_notices(&events).len(), 1);
    assert!(
        terminated_with(&events, TerminationReason::Cancelled),
        "cancelling the recovery turn must end the run as cancelled: {events:?}"
    );
    assert!(
        !saw_text(&events, "must not be delivered"),
        "no text may be delivered after cancellation"
    );
    assert_eq!(
        model.call_count(),
        1,
        "the cancelled recovery turn must not reach the model"
    );
}

#[tokio::test]
async fn a_tool_using_run_that_ends_empty_is_not_recovered() {
    // Detection rule 2: a run that started a tool call already made inspectable
    // progress, so an empty final answer after tool work is not a silent turn.
    let (engine, model) = engine_with_observation(
        vec![
            FakeTurn::ToolUse {
                id: "call-1".to_string(),
                name: "echo".to_string(),
                args: serde_json::json!({"message": "progress"}),
            },
            FakeTurn::Text(String::new()),
            FakeTurn::Text("must never be reached".to_string()),
        ],
        SilentTurnRecoveryPolicy::default(),
    );

    let events = collect(&engine, "answer the request").await;

    assert!(recovery_notices(&events).is_empty());
    assert!(silent_turn_degradations(&events).is_empty());
    assert_eq!(
        model.call_count(),
        2,
        "a tool-using run must terminate without spending a recovery turn"
    );
    assert_eq!(model.nudges_in_last_prompt(), 0);
    assert_eq!(final_output(&events).as_deref(), Some(""));
}

#[tokio::test]
async fn a_silent_run_with_no_user_message_is_not_recovered() {
    // Detection rule 3: recovery exists to answer a real user, so a run whose
    // input is empty must not start an extra turn.
    let (engine, model) = engine_with_observation(
        vec![
            FakeTurn::Text(String::new()),
            FakeTurn::Text("must never be reached".to_string()),
        ],
        SilentTurnRecoveryPolicy::default(),
    );

    let events = collect(&engine, "   ").await;

    assert!(recovery_notices(&events).is_empty());
    assert_eq!(model.call_count(), 1);
    assert_eq!(final_output(&events).as_deref(), Some(""));
}

/// The durable half of R2a: the events reach `trace.jsonl` and the SQLite event
/// index, the recovery answer lands in resumable history, and the degradation is
/// materialized in the lifecycle state that resume reads.
#[tokio::test]
async fn a_recovered_run_is_durable_and_resumable() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let model = ObservableFake::with_turns(vec![
        FakeTurn::Text(String::new()),
        FakeTurn::Text("recovered answer".to_string()),
    ]);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(rove_runtime::tools::echo::EchoTool));
    let engine = Engine::new(
        Box::new(model.clone()),
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig::new(8, false),
    );

    let run = state_store
        .start_run(SessionId::new(), JobId::new(), RunId::new())
        .unwrap();
    let mut recorder = RunArtifactRecorder::new(
        run.session_id,
        run.job_id,
        run.run_id,
        "answer the request".to_string(),
        None,
        None,
    );
    let stream = engine.run(
        run.request("answer the request".to_string(), None),
        Some(run.trace_writer.clone()),
    );
    futures::pin_mut!(stream);
    while let Some(event) = stream.next().await {
        recorder.record_event(&event, &state_store).await;
    }

    let trace = std::fs::read_to_string(run.run_dir.join("trace.jsonl")).unwrap();
    let degradations: Vec<(String, String)> =
        rove_runtime::state::trace_reader::read_trace_content(&trace)
            .entries
            .into_iter()
            .filter_map(|entry| match entry.entry {
                rove_runtime::foundation::TraceEntry::Ui(StreamEvent::ExecutionDegraded {
                    record,
                }) => Some((record.code, record.safe_summary)),
                _ => None,
            })
            .collect();
    assert_eq!(
        degradations.len(),
        1,
        "the recovery degradation must be durable in trace.jsonl"
    );
    assert_eq!(degradations[0].0, "silent_turn_recovery");
    assert!(
        !degradations[0].1.contains("answer the request"),
        "the safe summary must not carry the user's message"
    );
    assert!(
        trace.contains("recovering_silent_turn"),
        "the recovery status must be durable in trace.jsonl"
    );
    let statuses = trace.matches(NUDGE).count();
    assert_eq!(
        statuses, 1,
        "the nudge must be persisted once, as the status message"
    );

    let indexed: Vec<String> = state_store
        .index
        .event_records(run.run_id)
        .unwrap()
        .into_iter()
        .map(|record| record.event_name)
        .collect();
    assert_eq!(
        indexed
            .iter()
            .filter(|name| *name == "execution_degraded")
            .count(),
        1,
        "the degradation must reach the SQLite event index: {indexed:?}"
    );
    assert_eq!(
        indexed
            .iter()
            .filter(|name| *name == "model_status")
            .count(),
        3,
        "thinking, the recovery notice, and thinking again: {indexed:?}"
    );

    let task_state = state_store.load_task_state(run.run_id).await.unwrap();
    assert_eq!(
        task_state.execution_lifecycle.degradations.len(),
        1,
        "the degradation must be materialized in the lifecycle state"
    );
    assert_eq!(
        task_state.execution_lifecycle.degradations[0].code,
        "silent_turn_recovery"
    );
    assert!(
        task_state
            .history
            .iter()
            .any(|message| message.role == Role::Assistant
                && message.content == "recovered answer"),
        "the recovery answer must be part of the resumable history: {:?}",
        task_state.history
    );

    // Resume sees the recovery answer and does not re-run the silent turn.
    let resumed_model =
        ObservableFake::with_turns(vec![FakeTurn::Text("resumed after recovery".to_string())]);
    let mut resumed_registry = ToolRegistry::new();
    resumed_registry.register(Box::new(rove_runtime::tools::echo::EchoTool));
    let resumed_engine = Engine::new(
        Box::new(resumed_model.clone()),
        resumed_registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig::new(8, false),
    );
    let resumed = resumed_engine.run(
        RunRequest {
            session_id: task_state.session_id,
            job_id: task_state.job_id,
            run_id: RunId::new(),
            user_message: "keep going".to_string(),
            resume_state: Some(task_state),
        },
        None,
    );
    futures::pin_mut!(resumed);
    while resumed.next().await.is_some() {}

    assert_eq!(
        resumed_model.call_count(),
        1,
        "a resumed run must not replay the recovered silent turn"
    );
    assert!(
        resumed_model
            .last_messages()
            .iter()
            .any(|message| message.content == "recovered answer"),
        "the recovery answer must be visible to the resumed run"
    );
}
