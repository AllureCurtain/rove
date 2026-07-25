use async_stream::stream;
use futures::{StreamExt, stream::BoxStream};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{Instant, sleep, timeout};

use crate::health::{HealthConfig, ModelHealthStore};
use crate::traits::{ModelClient, ModelEvent};
use crate::{Message, ModelError, ModelToolSchema};

/// Model client that tries a primary provider, then fallback providers if the
/// active provider fails before streaming any response chunks.
pub struct RoutingModelClient {
    clients: Vec<Box<dyn ModelClient>>,
    health: Arc<ModelHealthStore>,
    model_id: String,
    probe_timeout: Duration,
    retry_policy: RetryPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            backoff_base: Duration::from_millis(250),
            backoff_max: Duration::from_secs(5),
        }
    }
}

impl RetryPolicy {
    fn normalized(self) -> Self {
        Self {
            max_attempts: self.max_attempts.max(1),
            backoff_base: self.backoff_base,
            backoff_max: self.backoff_max.max(self.backoff_base),
        }
    }

    fn delay_for(self, error: &ModelError, failed_attempt_index: u32) -> Duration {
        match error {
            ModelError::RateLimited { retry_after_ms } => Duration::from_millis(*retry_after_ms),
            _ => {
                let factor = 1_u32
                    .checked_shl(failed_attempt_index.saturating_sub(1).min(31))
                    .unwrap_or(u32::MAX);
                self.backoff_base
                    .saturating_mul(factor)
                    .min(self.backoff_max)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeOutcome {
    Committed,
    ErrorBeforeCommit,
    NoContent,
    Timeout,
    SkippedOpenCircuit,
}

impl ProbeOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::ErrorBeforeCommit => "error_before_commit",
            Self::NoContent => "no_content",
            Self::Timeout => "timeout",
            Self::SkippedOpenCircuit => "skipped_open_circuit",
        }
    }
}

impl RoutingModelClient {
    const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(60);

    pub fn new(primary: Box<dyn ModelClient>, fallbacks: Vec<Box<dyn ModelClient>>) -> Self {
        Self::with_health_store(
            primary,
            fallbacks,
            Arc::new(ModelHealthStore::new(HealthConfig::default())),
        )
    }

    pub fn with_health_store(
        primary: Box<dyn ModelClient>,
        fallbacks: Vec<Box<dyn ModelClient>>,
        health: Arc<ModelHealthStore>,
    ) -> Self {
        let mut clients = Vec::with_capacity(1 + fallbacks.len());
        clients.push(primary);
        clients.extend(fallbacks);
        let model_id = format!(
            "routing({})",
            clients
                .iter()
                .map(|client| client.client_id().to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        Self {
            clients,
            health,
            model_id,
            probe_timeout: Self::DEFAULT_PROBE_TIMEOUT,
            retry_policy: RetryPolicy::default(),
        }
    }

    pub fn with_probe_timeout(mut self, probe_timeout: Duration) -> Self {
        self.probe_timeout = probe_timeout;
        self
    }

    pub fn with_health_config(mut self, health_config: HealthConfig) -> Self {
        self.health = Arc::new(ModelHealthStore::new(health_config));
        self
    }

    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy.normalized();
        self
    }
}

impl ModelClient for RoutingModelClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        Box::pin(stream! {
            let mut last_error = None;
            let probe_timeout = self.probe_timeout;
            let retry_policy = self.retry_policy.normalized();
            for client in &self.clients {
                let client_id = client.client_id();
                let client_id_text = client_id.as_str().to_string();
                if !self.health.allow_call(&client_id_text) {
                    tracing::debug!(
                        model_target = %client_id_text,
                        routing_model = %self.model_id,
                        outcome = ProbeOutcome::SkippedOpenCircuit.as_str(),
                        "model routing candidate skipped"
                    );
                    continue;
                }

                let mut committed = false;
                for attempt in 1..=retry_policy.max_attempts {
                    let attempt_started = Instant::now();
                    tracing::debug!(
                        model_target = %client_id_text,
                        routing_model = %self.model_id,
                        attempt,
                        max_attempts = retry_policy.max_attempts,
                        "model routing candidate probe started"
                    );
                    let mut events = client.stream(&messages, &tools);
                    let mut buffered = Vec::new();
                    let attempt_result = loop {
                        match timeout(probe_timeout, events.next()).await {
                            Ok(Some(Ok(event))) if is_commit_event(&event) => {
                                tracing::info!(
                                    model_target = %client_id_text,
                                    routing_model = %self.model_id,
                                    attempt,
                                    max_attempts = retry_policy.max_attempts,
                                    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
                                    outcome = ProbeOutcome::Committed.as_str(),
                                    "model routing candidate committed"
                                );
                                self.health.mark_success(&client_id_text);
                                for buffered_event in buffered {
                                    yield Ok(buffered_event);
                                }
                                yield Ok(event);
                                committed = true;
                                break Ok(events);
                            }
                            Ok(Some(Ok(ModelEvent::Done))) => {
                                let err = ModelError::StreamInterrupted(
                                    "stream ended before first content event".to_string(),
                                );
                                tracing::warn!(
                                    model_target = %client_id_text,
                                    routing_model = %self.model_id,
                                    attempt,
                                    max_attempts = retry_policy.max_attempts,
                                    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
                                    outcome = ProbeOutcome::NoContent.as_str(),
                                    "model routing stream ended before first content event"
                                );
                                break Err(err);
                            }
                            Ok(Some(Ok(event))) => {
                                buffered.push(event);
                                continue;
                            }
                            Ok(Some(Err(err))) => {
                                tracing::warn!(
                                    model_target = %client_id_text,
                                    routing_model = %self.model_id,
                                    attempt,
                                    max_attempts = retry_policy.max_attempts,
                                    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
                                    outcome = ProbeOutcome::ErrorBeforeCommit.as_str(),
                                    error = %err,
                                    "model routing candidate failed before commit"
                                );
                                break Err(err);
                            }
                            Ok(None) => {
                                let err = ModelError::StreamInterrupted(
                                    "stream ended before first content event".to_string(),
                                );
                                tracing::warn!(
                                    model_target = %client_id_text,
                                    routing_model = %self.model_id,
                                    attempt,
                                    max_attempts = retry_policy.max_attempts,
                                    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
                                    outcome = ProbeOutcome::NoContent.as_str(),
                                    "model routing stream ended before first content event"
                                );
                                break Err(err);
                            }
                            Err(_) => {
                                let err = ModelError::StreamInterrupted(format!(
                                    "first content event probe timed out after {}ms",
                                    probe_timeout.as_millis()
                                ));
                                tracing::warn!(
                                    model_target = %client_id_text,
                                    routing_model = %self.model_id,
                                    attempt,
                                    max_attempts = retry_policy.max_attempts,
                                    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
                                    timeout_ms = probe_timeout.as_millis() as u64,
                                    outcome = ProbeOutcome::Timeout.as_str(),
                                    "model routing first content probe timed out"
                                );
                                break Err(err);
                            }
                        }
                    };

                    match attempt_result {
                        Ok(mut events) => {
                            loop {
                                match events.next().await {
                                    Some(Ok(event)) => {
                                        yield Ok(event);
                                    }
                                    Some(Err(err)) => {
                                        if err.counts_as_health_failure() {
                                            self.health.mark_failure(&client_id_text);
                                        }
                                        yield Err(err);
                                        return;
                                    }
                                    None => return,
                                }
                            }
                        }
                        Err(err) => {
                            if err.counts_as_health_failure() {
                                self.health.mark_failure(&client_id_text);
                            }
                            let retryable = err.is_retryable();
                            let has_attempt_remaining = attempt < retry_policy.max_attempts;
                            last_error = Some(err.clone());
                            if retryable && has_attempt_remaining {
                                let delay = retry_policy.delay_for(&err, attempt);
                                tracing::warn!(
                                    model_target = %client_id_text,
                                    routing_model = %self.model_id,
                                    attempt,
                                    next_attempt = attempt + 1,
                                    max_attempts = retry_policy.max_attempts,
                                    delay_ms = delay.as_millis() as u64,
                                    error = %err,
                                    "model routing retry scheduled"
                                );
                                if !delay.is_zero() {
                                    sleep(delay).await;
                                }
                                continue;
                            }
                            tracing::warn!(
                                model_target = %client_id_text,
                                routing_model = %self.model_id,
                                attempt,
                                max_attempts = retry_policy.max_attempts,
                                retryable,
                                error = %err,
                                "model routing candidate exhausted before commit"
                            );
                            break;
                        }
                    }
                }
                if !committed {
                    continue;
                }
            }
            if let Some(err) = last_error {
                yield Err(err);
            }
        })
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

fn is_commit_event(event: &ModelEvent) -> bool {
    match event {
        ModelEvent::TextDelta { text } | ModelEvent::ThinkingDelta { text } => !text.is_empty(),
        ModelEvent::ToolUseStart { .. } | ModelEvent::ToolUseDone { .. } => true,
        ModelEvent::ToolUseDelta { .. } | ModelEvent::Usage { .. } | ModelEvent::Done => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use futures::{StreamExt, stream, stream::BoxStream};
    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::prelude::*;

    use crate::health::{HealthConfig, ModelHealthStore};
    use crate::routing::{RetryPolicy, RoutingModelClient};
    use crate::traits::{ModelClient, ModelEvent};
    use crate::{Message, ModelError, ModelToolSchema, Usage};

    struct FailingClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for FailingClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::once(async {
                Err(ModelError::RequestFailed("primary unavailable".to_string()))
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    struct StaticClient {
        id: &'static str,
        response: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for StaticClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self.response.to_string();
            Box::pin(stream::once(async move {
                Ok(ModelEvent::TextDelta { text: response })
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    struct PartialThenErrorClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for PartialThenErrorClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::iter([
                Ok(ModelEvent::TextDelta {
                    text: "partial".to_string(),
                }),
                Err(ModelError::StreamInterrupted(
                    "primary interrupted".to_string(),
                )),
            ]))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    struct SlowFirstChunkClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for SlowFirstChunkClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::once(async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok(ModelEvent::TextDelta {
                    text: "too late".to_string(),
                })
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    struct FailsThenSucceedsClient {
        id: &'static str,
        failures_remaining: AtomicUsize,
        calls: Arc<AtomicUsize>,
    }

    struct FixedErrorClient {
        id: &'static str,
        error: ModelError,
        calls: Arc<AtomicUsize>,
    }

    struct RateLimitThenSucceedsClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    struct UsageThenErrorClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for UsageThenErrorClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::iter([
                Ok(ModelEvent::Usage {
                    usage: Usage::default(),
                }),
                Err(ModelError::StreamInterrupted(
                    "primary interrupted after usage".to_string(),
                )),
            ]))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    struct ToolUseStartThenErrorClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for ToolUseStartThenErrorClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::iter([
                Ok(ModelEvent::ToolUseStart {
                    id: "native-call-1".to_string(),
                    name: "echo".to_string(),
                }),
                Err(ModelError::StreamInterrupted(
                    "primary interrupted after tool start".to_string(),
                )),
            ]))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    #[async_trait]
    impl ModelClient for FailsThenSucceedsClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self
                .failures_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                    current.checked_sub(1)
                })
                .is_ok()
            {
                return Box::pin(stream::once(async {
                    Err(ModelError::RequestFailed("primary unavailable".to_string()))
                }));
            }

            Box::pin(stream::once(async {
                Ok(ModelEvent::TextDelta {
                    text: "primary recovered".to_string(),
                })
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    #[async_trait]
    impl ModelClient for FixedErrorClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let error = self.error.clone();
            Box::pin(stream::once(async move { Err(error) }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    #[async_trait]
    impl ModelClient for RateLimitThenSucceedsClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                return Box::pin(stream::once(async {
                    Err(ModelError::RateLimited {
                        retry_after_ms: 2_000,
                    })
                }));
            }

            Box::pin(stream::once(async {
                Ok(ModelEvent::TextDelta {
                    text: "primary recovered".to_string(),
                })
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    fn assert_text_event(event: &Result<ModelEvent, ModelError>, expected: &str) {
        assert!(
            matches!(event, Ok(ModelEvent::TextDelta { text }) if text == expected),
            "expected text event {expected:?}, got {event:?}"
        );
    }

    #[derive(Clone, Debug, Default)]
    struct CapturedTraceEvent {
        fields: Vec<(String, String)>,
    }

    impl CapturedTraceEvent {
        fn field_eq(&self, name: &str, expected: &str) -> bool {
            self.fields
                .iter()
                .any(|(field, value)| field == name && value == expected)
        }

        fn message_contains(&self, expected: &str) -> bool {
            self.fields
                .iter()
                .any(|(field, value)| field == "message" && value.contains(expected))
        }
    }

    #[derive(Default)]
    struct TraceVisitor {
        event: CapturedTraceEvent,
    }

    impl TraceVisitor {
        fn record_value(&mut self, field: &Field, value: impl ToString) {
            self.event
                .fields
                .push((field.name().to_string(), value.to_string()));
        }
    }

    impl Visit for TraceVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.record_value(field, format!("{value:?}"));
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.record_value(field, value);
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.record_value(field, value);
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.record_value(field, value);
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.record_value(field, value);
        }
    }

    #[derive(Clone)]
    struct CaptureTraceLayer {
        events: Arc<Mutex<Vec<CapturedTraceEvent>>>,
    }

    impl<S> Layer<S> for CaptureTraceLayer
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = TraceVisitor::default();
            event.record(&mut visitor);
            self.events.lock().expect("trace lock").push(visitor.event);
        }
    }

    fn install_trace_capture() -> Arc<Mutex<Vec<CapturedTraceEvent>>> {
        static TRACE_EVENTS: OnceLock<Arc<Mutex<Vec<CapturedTraceEvent>>>> = OnceLock::new();
        static TRACE_SUBSCRIBER: OnceLock<()> = OnceLock::new();

        let events = TRACE_EVENTS
            .get_or_init(|| Arc::new(Mutex::new(Vec::new())))
            .clone();
        TRACE_SUBSCRIBER.get_or_init(|| {
            let subscriber = tracing_subscriber::registry().with(CaptureTraceLayer {
                events: events.clone(),
            });
            tracing::subscriber::set_global_default(subscriber)
                .expect("trace capture subscriber should install once");
        });
        events
    }

    #[tokio::test]
    async fn retries_retryable_request_failure_before_fallback() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FailsThenSucceedsClient {
                id: "primary",
                failures_remaining: AtomicUsize::new(1),
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 2,
            backoff_base: std::time::Duration::from_millis(0),
            backoff_max: std::time::Duration::from_millis(0),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(chunks.len(), 1);
        assert_text_event(&chunks[0], "primary recovered");
    }

    #[tokio::test]
    async fn retry_trace_records_attempts_and_final_outcome() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let events = install_trace_capture();
        events.lock().expect("trace lock").clear();
        let model = RoutingModelClient::new(
            Box::new(FailsThenSucceedsClient {
                id: "trace-primary",
                failures_remaining: AtomicUsize::new(1),
                calls: primary_calls.clone(),
            }),
            Vec::new(),
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 2,
            backoff_base: std::time::Duration::from_millis(0),
            backoff_max: std::time::Duration::from_millis(0),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_text_event(&chunks[0], "primary recovered");
        let events = events
            .lock()
            .expect("trace lock")
            .iter()
            .filter(|event| event.field_eq("model_target", "trace-primary"))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            events.iter().any(|event| event
                .message_contains("model routing candidate probe started")
                && event.field_eq("attempt", "1")
                && event.field_eq("max_attempts", "2")),
            "expected first attempt trace event, got {events:#?}"
        );
        assert!(
            events.iter().any(
                |event| event.message_contains("model routing retry scheduled")
                    && event.field_eq("attempt", "1")
                    && event.field_eq("next_attempt", "2")
                    && event.field_eq("max_attempts", "2")
            ),
            "expected retry scheduled trace event, got {events:#?}"
        );
        assert!(
            events.iter().any(
                |event| event.message_contains("model routing candidate committed")
                    && event.field_eq("attempt", "2")
                    && event.field_eq("max_attempts", "2")
                    && event.field_eq("outcome", "committed")
            ),
            "expected committed final outcome trace event, got {events:#?}"
        );
    }

    #[tokio::test]
    async fn does_not_retry_auth_context_or_configuration_errors() {
        let auth_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FixedErrorClient {
                id: "primary",
                error: ModelError::AuthFailed,
                calls: auth_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            backoff_base: std::time::Duration::from_millis(0),
            backoff_max: std::time::Duration::from_millis(0),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(auth_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        assert_text_event(&chunks[0], "fallback answer");

        let context_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FixedErrorClient {
                id: "primary",
                error: ModelError::ContextLengthExceeded { used: 10, max: 5 },
                calls: context_calls.clone(),
            }),
            Vec::new(),
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            backoff_base: std::time::Duration::from_millis(0),
            backoff_max: std::time::Duration::from_millis(0),
        });

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(context_calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            chunks[0],
            Err(ModelError::ContextLengthExceeded { used: 10, max: 5 })
        ));

        let configuration_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FixedErrorClient {
                id: "primary",
                error: ModelError::InvalidConfiguration("invalid endpoint".to_string()),
                calls: configuration_calls.clone(),
            }),
            Vec::new(),
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            backoff_base: std::time::Duration::from_millis(0),
            backoff_max: std::time::Duration::from_millis(0),
        });

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(configuration_calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            chunks[0],
            Err(ModelError::InvalidConfiguration(ref message))
                if message == "invalid endpoint"
        ));
    }

    #[tokio::test]
    async fn does_not_retry_after_primary_has_streamed_chunks() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(PartialThenErrorClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            backoff_base: std::time::Duration::from_millis(0),
            backoff_max: std::time::Duration::from_millis(0),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(chunks.len(), 2);
        assert_text_event(&chunks[0], "partial");
        assert!(matches!(
            chunks[1],
            Err(ModelError::StreamInterrupted(ref message))
                if message == "primary interrupted"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limit_retry_respects_retry_after_before_next_attempt() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(RateLimitThenSucceedsClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            Vec::new(),
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 2,
            backoff_base: std::time::Duration::from_secs(30),
            backoff_max: std::time::Duration::from_secs(30),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let started = tokio::time::Instant::now();
        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert!(started.elapsed() >= std::time::Duration::from_secs(2));
        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_text_event(&chunks[0], "primary recovered");
    }

    #[tokio::test]
    async fn falls_back_when_primary_errors_before_streaming() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FailingClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        );
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chunks.len(), 1);
        assert_text_event(&chunks[0], "fallback answer");
    }

    #[tokio::test]
    async fn does_not_fallback_after_primary_has_streamed_chunks() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(PartialThenErrorClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        );
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(chunks.len(), 2);
        assert_text_event(&chunks[0], "partial");
        assert!(matches!(
            chunks[1],
            Err(ModelError::StreamInterrupted(ref message))
                if message == "primary interrupted"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn falls_back_when_primary_first_chunk_probe_times_out() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(SlowFirstChunkClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        )
        .with_probe_timeout(std::time::Duration::from_secs(1));
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chunks.len(), 1);
        assert_text_event(&chunks[0], "fallback answer");
    }

    #[tokio::test(start_paused = true)]
    async fn skips_open_circuit_before_cooldown() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FailingClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        )
        .with_health_config(HealthConfig {
            failure_threshold: 2,
            open_cooldown: std::time::Duration::from_secs(30),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let first = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let second = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let third = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_text_event(&first[0], "fallback answer");
        assert_text_event(&second[0], "fallback answer");
        assert_text_event(&third[0], "fallback answer");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn shared_health_store_skips_open_target_across_routing_clients() {
        let shared_health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: 1,
            open_cooldown: std::time::Duration::from_secs(30),
        }));
        let first_primary_calls = Arc::new(AtomicUsize::new(0));
        let second_primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));

        let first = RoutingModelClient::with_health_store(
            Box::new(FailingClient {
                id: "provider-a:same-model",
                calls: first_primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
            shared_health.clone(),
        );

        let second = RoutingModelClient::with_health_store(
            Box::new(FailingClient {
                id: "provider-a:same-model",
                calls: second_primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
            shared_health,
        );

        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let _ = first.stream(&messages, &tools).collect::<Vec<_>>().await;
        let _ = second.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(first_primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second_primary_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn half_open_probe_closes_circuit_after_success() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FailsThenSucceedsClient {
                id: "primary",
                failures_remaining: AtomicUsize::new(2),
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        )
        .with_health_config(HealthConfig {
            failure_threshold: 2,
            open_cooldown: std::time::Duration::from_secs(30),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let first = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let second = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        tokio::time::advance(std::time::Duration::from_secs(30)).await;
        let half_open = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let closed_again = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_text_event(&first[0], "fallback answer");
        assert_text_event(&second[0], "fallback answer");
        assert_text_event(&half_open[0], "primary recovered");
        assert_text_event(&closed_again[0], "primary recovered");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 4);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn usage_only_first_event_does_not_commit_routing_provider() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(UsageThenErrorClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        );
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chunks.len(), 1);
        assert_text_event(&chunks[0], "fallback answer");
    }

    #[tokio::test]
    async fn tool_use_start_commits_routing_provider() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(ToolUseStartThenErrorClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            backoff_base: std::time::Duration::from_millis(0),
            backoff_max: std::time::Duration::from_millis(0),
        });
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ModelToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            &chunks[0],
            Ok(ModelEvent::ToolUseStart { id, name })
                if id == "native-call-1" && name == "echo"
        ));
        assert!(matches!(
            &chunks[1],
            Err(ModelError::StreamInterrupted(message))
                if message == "primary interrupted after tool start"
        ));
    }
}
