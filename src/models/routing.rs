use async_stream::stream;
use futures::{StreamExt, stream::BoxStream};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::{Instant, timeout};

use crate::core::types::{Message, ToolSchema};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, StreamChunk};

/// Model client that tries a primary provider, then fallback providers if the
/// active provider fails before streaming any response chunks.
pub struct RoutingModelClient {
    clients: Vec<Box<dyn ModelClient>>,
    health: Arc<ModelHealthStore>,
    model_id: String,
    probe_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct HealthConfig {
    pub failure_threshold: u32,
    pub open_cooldown: Duration,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            open_cooldown: Duration::from_secs(30),
        }
    }
}

struct ModelHealthStore {
    config: HealthConfig,
    states: Mutex<HashMap<String, HealthState>>,
}

impl ModelHealthStore {
    fn new(config: HealthConfig) -> Self {
        Self {
            config,
            states: Mutex::new(HashMap::new()),
        }
    }

    fn allow_call(&self, model_id: &str) -> bool {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(model_id.to_string()).or_default();
        match state.status {
            CircuitStatus::Closed => true,
            CircuitStatus::Open => {
                if state
                    .opened_at
                    .is_some_and(|opened_at| opened_at.elapsed() >= self.config.open_cooldown)
                {
                    state.status = CircuitStatus::HalfOpen;
                    state.half_open_token = true;
                }
                if state.status == CircuitStatus::HalfOpen && state.half_open_token {
                    state.half_open_token = false;
                    true
                } else {
                    false
                }
            }
            CircuitStatus::HalfOpen => {
                if state.half_open_token {
                    state.half_open_token = false;
                    true
                } else {
                    false
                }
            }
        }
    }

    fn mark_success(&self, model_id: &str) {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(model_id.to_string()).or_default();
        state.status = CircuitStatus::Closed;
        state.consecutive_failures = 0;
        state.opened_at = None;
        state.half_open_token = true;
    }

    fn mark_failure(&self, model_id: &str) {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(model_id.to_string()).or_default();
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.status == CircuitStatus::HalfOpen
            || state.consecutive_failures >= self.config.failure_threshold.max(1)
        {
            state.status = CircuitStatus::Open;
            state.opened_at = Some(Instant::now());
            state.half_open_token = false;
        }
    }
}

#[derive(Debug, Default)]
struct HealthState {
    status: CircuitStatus,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    half_open_token: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum CircuitStatus {
    #[default]
    Closed,
    Open,
    HalfOpen,
}

impl RoutingModelClient {
    const DEFAULT_PROBE_TIMEOUT: Duration = Duration::from_secs(60);

    pub fn new(primary: Box<dyn ModelClient>, fallbacks: Vec<Box<dyn ModelClient>>) -> Self {
        let mut clients = Vec::with_capacity(1 + fallbacks.len());
        clients.push(primary);
        clients.extend(fallbacks);
        let model_id = format!(
            "routing({})",
            clients
                .iter()
                .map(|client| client.model_id())
                .collect::<Vec<_>>()
                .join(",")
        );
        Self {
            clients,
            health: Arc::new(ModelHealthStore::new(HealthConfig::default())),
            model_id,
            probe_timeout: Self::DEFAULT_PROBE_TIMEOUT,
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
}

impl ModelClient for RoutingModelClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        Box::pin(stream! {
            let mut last_error = None;
            let probe_timeout = self.probe_timeout;
            for client in &self.clients {
                let client_id = client.model_id().to_string();
                if !self.health.allow_call(&client_id) {
                    continue;
                }

                let mut chunks = client.stream(&messages, &tools);
                match timeout(probe_timeout, chunks.next()).await {
                    Ok(Some(Ok(chunk))) => {
                        self.health.mark_success(&client_id);
                        yield Ok(chunk);
                    }
                    Ok(Some(Err(err))) => {
                        if err.counts_as_health_failure() {
                            self.health.mark_failure(&client_id);
                        }
                        last_error = Some(err);
                        continue;
                    }
                    Ok(None) => {
                        let err = ModelError::StreamInterrupted(
                            "stream ended before first chunk".to_string(),
                        );
                        self.health.mark_failure(&client_id);
                        last_error = Some(err);
                        continue;
                    }
                    Err(_) => {
                        let err = ModelError::StreamInterrupted(format!(
                            "first chunk probe timed out after {}ms",
                            probe_timeout.as_millis()
                        ));
                        self.health.mark_failure(&client_id);
                        last_error = Some(err);
                        continue;
                    }
                }

                loop {
                    match chunks.next().await {
                        Some(Ok(chunk)) => {
                            yield Ok(chunk);
                        }
                        Some(Err(err)) => {
                            if err.counts_as_health_failure() {
                                self.health.mark_failure(&client_id);
                            }
                            yield Err(err);
                            return;
                        }
                        None => return,
                    }
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

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use futures::{StreamExt, stream, stream::BoxStream};

    use crate::core::types::{Message, ToolSchema, Usage};
    use crate::errors::ModelError;
    use crate::models::routing::{HealthConfig, RoutingModelClient};
    use crate::models::traits::{ModelClient, StreamChunk};

    struct FailingClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for FailingClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
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
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self.response.to_string();
            Box::pin(stream::once(async move {
                Ok(StreamChunk {
                    delta: response,
                    usage: Some(Usage::default()),
                })
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
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::iter([
                Ok(StreamChunk {
                    delta: "partial".to_string(),
                    usage: None,
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
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::once(async {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                Ok(StreamChunk {
                    delta: "too late".to_string(),
                    usage: None,
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

    #[async_trait]
    impl ModelClient for FailsThenSucceedsClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
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
                Ok(StreamChunk {
                    delta: "primary recovered".to_string(),
                    usage: Some(Usage::default()),
                })
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
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
        let tools: Vec<ToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "fallback answer");
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
        let tools: Vec<ToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "partial");
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
        let tools: Vec<ToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "fallback answer");
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
        let tools: Vec<ToolSchema> = Vec::new();

        let first = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let second = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let third = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(first[0].as_ref().unwrap().delta, "fallback answer");
        assert_eq!(second[0].as_ref().unwrap().delta, "fallback answer");
        assert_eq!(third[0].as_ref().unwrap().delta, "fallback answer");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 3);
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
        let tools: Vec<ToolSchema> = Vec::new();

        let first = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let second = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        tokio::time::advance(std::time::Duration::from_secs(30)).await;
        let half_open = model.stream(&messages, &tools).collect::<Vec<_>>().await;
        let closed_again = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(first[0].as_ref().unwrap().delta, "fallback answer");
        assert_eq!(second[0].as_ref().unwrap().delta, "fallback answer");
        assert_eq!(half_open[0].as_ref().unwrap().delta, "primary recovered");
        assert_eq!(closed_again[0].as_ref().unwrap().delta, "primary recovered");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 4);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 2);
    }
}
