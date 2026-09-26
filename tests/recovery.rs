//! Offline contract tests for the run loop's model-call retry budget.
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

use std::time::Duration;

use futures::StreamExt;
use rove_core::ToolRegistry;
use rove_models::{FakeModelClient, FakeTurn, ModelError};
use rove_runtime::context::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig, ProviderRetryPolicy};
use rove_runtime::events::StreamEvent;
use rove_runtime::types::{JobId, RunId, RunRequest, SessionId, TerminationReason};

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

async fn collect(engine: &Engine, message: &str) -> Vec<StreamEvent> {
    let stream = engine.run(
        RunRequest {
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message: message.to_string(),
            resume_state: None,
        },
        None,
    );
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
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
