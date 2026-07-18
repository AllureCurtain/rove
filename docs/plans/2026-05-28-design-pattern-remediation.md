# Design Pattern Remediation Implementation Plan

> **For implementers:** Execute this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the current runtime closer to the RAGENT-inspired model governance patterns by sharing routing health state, using stable provider identities, improving first-packet observability, tightening API resume semantics, and cleaning up tool construction boundaries.

**Architecture:** Keep the existing `ModelClient` decorator shape: provider adapters remain simple, and `RoutingModelClient` remains transparent to the engine. Move cross-run health decisions into shared runtime state, make provider identity explicit, add traceable probe outcomes, and centralize interface-owned runtime assembly. RAG embedding/rerank governance is planned as a second pass because the current RAG tool intentionally defaults to deterministic local retrieval.

**Tech Stack:** Rust 2024, tokio, axum, serde, tracing, cargo test, Vitest/TypeScript for any API contract changes reflected in Web types.

---

## Scope And Ordering

Implement these tasks in order:

1. Shared model health state and stable provider identity.
2. First-packet routing observability.
3. API resume conflict semantics.
4. Default tool registry and context-bound memory tools.
5. RAG embedding/rerank governance foundation.
6. Documentation/status sync and full verification.

The first task is the highest-leverage fix. Without shared health state and stable candidate IDs, observability and RAG routing cannot report reliable provider health.

## File Structure

- Create `src/models/health.rs` for reusable health store, circuit state, and health configuration.
- Modify `src/models/mod.rs` to export the new health module.
- Modify `src/models/traits.rs` to add a stable, provider-aware model target identity.
- Modify `src/models/openai.rs`, `src/models/anthropic.rs`, `src/models/ollama.rs`, and `src/models/fake.rs` so each provider exposes a non-ambiguous target ID.
- Modify `src/models/routing.rs` so `RoutingModelClient` consumes an injectable shared `Arc<ModelHealthStore>` and logs structured first-packet probe outcomes.
- Modify `src/models/factory.rs` so CLI can keep a private health store and API can inject a process-shared health store.
- Modify `src/interfaces/api/mod.rs` so `ApiState` owns shared routing health and rejects resume requests that would overwrite a live job.
- Modify `tests/model_factory.rs`, `src/models/routing.rs` unit tests, and `tests/api.rs` with regression coverage.
- Modify `src/tools/mod.rs` for a shared default registry builder.
- Modify `src/main.rs` and `src/interfaces/api/mod.rs` to use the shared registry builder.
- Modify `src/tools/memory.rs` and `tests/memory_tool.rs` to remove constructor arguments that are no longer used.
- Modify RAG files under `src/tools/rag/` only after the shared health module exists.
- Update `docs/runtime/implementation-status.md` and `docs/runtime/implementation-guide.md` after code changes.

## Task 1: Shared Model Health State And Provider Identity

**Intent:** Make circuit breaker state process-level for API jobs and prevent different providers with the same model name from sharing one health bucket accidentally.

**Current problem:**

- `RoutingModelClient::new` creates its own `ModelHealthStore`, so API jobs do not share circuit state.
- Provider adapters currently expose only `model_id()`, which is usually just the model name.
- The health map key should represent the actual provider target, not only the user-facing model name.

**Files:**

- Create: `src/models/health.rs`
- Modify: `src/models/mod.rs`
- Modify: `src/models/traits.rs`
- Modify: `src/models/openai.rs`
- Modify: `src/models/anthropic.rs`
- Modify: `src/models/ollama.rs`
- Modify: `src/models/fake.rs`
- Modify: `src/models/routing.rs`
- Modify: `src/models/factory.rs`
- Modify: `src/interfaces/api/mod.rs`
- Test: `src/models/routing.rs`
- Test: `tests/model_factory.rs`
- Test: `tests/api.rs`

- [ ] **Step 1: Move health types into a reusable module**

Move `HealthConfig`, `ModelHealthStore`, `HealthState`, and `CircuitStatus` out of `src/models/routing.rs` into `src/models/health.rs`.

Use this public API shape:

```rust
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::time::Instant;

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

#[derive(Debug)]
pub struct ModelHealthStore {
    config: HealthConfig,
    states: Mutex<HashMap<String, HealthState>>,
}

impl ModelHealthStore {
    pub fn new(config: HealthConfig) -> Self {
        Self {
            config,
            states: Mutex::new(HashMap::new()),
        }
    }

    pub fn allow_call(&self, target_id: &str) -> bool {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(target_id.to_string()).or_default();
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

    pub fn mark_success(&self, target_id: &str) {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(target_id.to_string()).or_default();
        state.status = CircuitStatus::Closed;
        state.consecutive_failures = 0;
        state.opened_at = None;
        state.half_open_token = true;
    }

    pub fn mark_failure(&self, target_id: &str) {
        let mut states = self.states.lock().expect("model health mutex poisoned");
        let state = states.entry(target_id.to_string()).or_default();
        state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        if state.status == CircuitStatus::HalfOpen
            || state.consecutive_failures >= self.config.failure_threshold.max(1)
        {
            state.status = CircuitStatus::Open;
            state.opened_at = Some(Instant::now());
            state.half_open_token = false;
        }
    }

    #[cfg(test)]
    pub fn status_for_test(&self, target_id: &str) -> CircuitStatus {
        self.states
            .lock()
            .expect("model health mutex poisoned")
            .get(target_id)
            .map(|state| state.status)
            .unwrap_or_default()
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
pub enum CircuitStatus {
    #[default]
    Closed,
    Open,
    HalfOpen,
}
```

Export it in `src/models/mod.rs`:

```rust
pub mod health;
```

Run:

```powershell
cargo test --lib models::routing
```

Expected: it fails until imports in `routing.rs` are updated.

- [ ] **Step 2: Add stable provider-aware target identity**

In `src/models/traits.rs`, add a small newtype and a default trait method:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelClientId(String);

impl ModelClientId {
    pub fn new(provider: &str, endpoint: impl AsRef<str>, model: impl AsRef<str>) -> Self {
        let endpoint = endpoint.as_ref().trim_end_matches('/');
        let model = model.as_ref();
        Self(format!("{provider}:{endpoint}:{model}"))
    }

    pub fn opaque(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelClientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
```

Extend the trait:

```rust
pub trait ModelClient: Send + Sync {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>>;

    fn model_id(&self) -> &str;

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque(self.model_id().to_string())
    }
}
```

Provider overrides:

```rust
// OpenAI-compatible
fn client_id(&self) -> ModelClientId {
    ModelClientId::new("openai-compatible", &self.api_base, &self.model)
}

// Anthropic
fn client_id(&self) -> ModelClientId {
    ModelClientId::new("anthropic", &self.api_base, &self.model)
}

// Ollama
fn client_id(&self) -> ModelClientId {
    ModelClientId::new("ollama", &self.api_base, &self.model)
}

// Fake
fn client_id(&self) -> ModelClientId {
    ModelClientId::new("fake", "local", self.model_id())
}
```

Add focused tests beside provider adapter tests:

```rust
#[test]
fn client_id_includes_provider_endpoint_and_model() {
    let left = OpenAiClient::new(
        "https://primary.test/v1".to_string(),
        "key".to_string(),
        "same-model".to_string(),
    );
    let right = OpenAiClient::new(
        "https://fallback.test/v1".to_string(),
        "key".to_string(),
        "same-model".to_string(),
    );

    assert_ne!(left.client_id(), right.client_id());
    assert_eq!(left.model_id(), right.model_id());
}
```

Run:

```powershell
cargo test client_id_includes_provider_endpoint_and_model
```

Expected: PASS after provider overrides are implemented.

- [ ] **Step 3: Make `RoutingModelClient` consume injectable health**

Change `RoutingModelClient` to use `Arc<ModelHealthStore>` from `src/models/health.rs`.

Recommended shape:

```rust
use std::sync::Arc;
use crate::models::health::{HealthConfig, ModelHealthStore};

pub struct RoutingModelClient {
    clients: Vec<Box<dyn ModelClient>>,
    health: Arc<ModelHealthStore>,
    model_id: String,
    probe_timeout: Duration,
}

impl RoutingModelClient {
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
        }
    }

    pub fn with_health_config(mut self, health_config: HealthConfig) -> Self {
        self.health = Arc::new(ModelHealthStore::new(health_config));
        self
    }
}
```

Inside `stream`, use `client.client_id()` for all health decisions:

```rust
let client_id = client.client_id();
let client_id_text = client_id.as_str().to_string();
if !self.health.allow_call(&client_id_text) {
    continue;
}
```

Run:

```powershell
cargo test --lib models::routing
```

Expected: existing routing tests pass.

- [ ] **Step 4: Add a regression test for shared health across routed clients**

Add a unit test in `src/models/routing.rs`:

```rust
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
    let tools: Vec<ToolSchema> = Vec::new();

    let _ = first.stream(&messages, &tools).collect::<Vec<_>>().await;
    let _ = second.stream(&messages, &tools).collect::<Vec<_>>().await;

    assert_eq!(first_primary_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_primary_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fallback_calls.load(Ordering::SeqCst), 2);
}
```

If local test clients still use `model_id()` as their default `client_id()`, set their `id` values to the exact provider-target string as shown.

Run:

```powershell
cargo test --lib shared_health_store_skips_open_target_across_routing_clients -- --exact
```

Expected: PASS.

- [ ] **Step 5: Wire API to a process-shared health store**

Add an API state field:

```rust
use crate::models::health::{HealthConfig, ModelHealthStore};

struct ApiStateInner {
    workspace: Workspace,
    config: AppConfig,
    shutdown_token: CancellationToken,
    jobs: RwLock<HashMap<JobId, Arc<JobRecord>>>,
    model_health: Arc<ModelHealthStore>,
    rate_limit: tokio::sync::Mutex<RateLimitState>,
}
```

Initialize it from config:

```rust
let model_health = Arc::new(ModelHealthStore::new(HealthConfig {
    failure_threshold: config.routing.failure_threshold,
    open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
}));
```

Add factory overload:

```rust
pub fn build_model_client_with_health(
    config: &AppConfig,
    model_id: String,
    health: Arc<ModelHealthStore>,
) -> Box<dyn ModelClient> {
    build_routed_model_client(config, ProviderSpec::primary(config, model_id), Some(health))
}
```

Keep the existing CLI path:

```rust
pub fn build_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_routed_model_client(config, ProviderSpec::primary(config, model_id), None)
}
```

When fallback specs exist, use the shared store only on the API path and keep the configured private store on the CLI path:

```rust
let routed = match health {
    Some(health) => RoutingModelClient::with_health_store(primary, fallbacks, health),
    None => RoutingModelClient::new(primary, fallbacks).with_health_config(HealthConfig {
        failure_threshold: config.routing.failure_threshold,
        open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
    }),
};
```

The final API path should create the configured store once and pass it through without rebuilding it per job.

In `build_engine_for_record`, replace:

```rust
_ => build_model_client(config, model_id),
```

with:

```rust
_ => build_model_client_with_health(config, model_id, state.inner.model_health.clone()),
```

Run:

```powershell
cargo test --test api
cargo test --test model_factory
```

Expected: PASS.

## Task 2: First-Packet Probe Observability

**Intent:** Preserve the existing fallback behavior while making routing decisions diagnosable. RAGENT has a dedicated first-packet trace node; rove should emit structured tracing around the same decision point.

**Files:**

- Modify: `src/models/routing.rs`
- Test: `src/models/routing.rs`

- [ ] **Step 1: Add a small outcome enum in `routing.rs`**

Add near the routing implementation:

```rust
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
```

- [ ] **Step 2: Emit structured tracing for every candidate decision**

At the start of each candidate attempt:

```rust
let attempt_started = Instant::now();
tracing::debug!(
    model_target = %client_id_text,
    routing_model = %self.model_id,
    "model routing candidate probe started"
);
```

When a candidate is skipped due to open circuit:

```rust
tracing::debug!(
    model_target = %client_id_text,
    routing_model = %self.model_id,
    outcome = ProbeOutcome::SkippedOpenCircuit.as_str(),
    "model routing candidate skipped"
);
```

When the first commit event arrives:

```rust
tracing::info!(
    model_target = %client_id_text,
    routing_model = %self.model_id,
    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
    outcome = ProbeOutcome::Committed.as_str(),
    "model routing candidate committed"
);
```

When the probe times out:

```rust
tracing::warn!(
    model_target = %client_id_text,
    routing_model = %self.model_id,
    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
    timeout_ms = probe_timeout.as_millis() as u64,
    outcome = ProbeOutcome::Timeout.as_str(),
    "model routing first content probe timed out"
);
```

When the stream ends before content:

```rust
tracing::warn!(
    model_target = %client_id_text,
    routing_model = %self.model_id,
    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
    outcome = ProbeOutcome::NoContent.as_str(),
    "model routing stream ended before first content event"
);
```

When an error occurs before commit:

```rust
tracing::warn!(
    model_target = %client_id_text,
    routing_model = %self.model_id,
    first_event_latency_ms = attempt_started.elapsed().as_millis() as u64,
    outcome = ProbeOutcome::ErrorBeforeCommit.as_str(),
    error = %err,
    "model routing candidate failed before commit"
);
```

Do not add a user-facing `StreamEvent` in this pass. The first pass should write to tracing only so CLI/API/Web contracts remain stable.

- [ ] **Step 3: Keep routing behavior unchanged**

Run the existing behavior tests:

```powershell
cargo test --lib falls_back_when_primary_errors_before_streaming -- --exact
cargo test --lib does_not_fallback_after_primary_has_streamed_chunks -- --exact
cargo test --lib falls_back_when_primary_first_chunk_probe_times_out -- --exact
cargo test --lib half_open_probe_closes_circuit_after_success -- --exact
```

Expected: PASS. These tests prove the observability change did not alter fallback semantics.

- [ ] **Step 4: Add one log-shape unit test only if the repo already captures tracing in tests**

Search first:

```powershell
rg -n "tracing_test|traced_test|with_default|Subscriber|fmt\\(\\)\\.with" tests src
```

If there is no existing tracing-capture pattern, do not introduce a new test dependency. Treat the behavior tests above plus `cargo clippy` as verification for this task.

## Task 3: API Resume Conflict Semantics

**Intent:** Prevent a resumed job from overwriting a still-live job record in the API `jobs: HashMap<JobId, Arc<JobRecord>>`.

**Chosen behavior:** A `POST /jobs` request with `resume` is rejected with HTTP `409 Conflict` if the resumed `job_id` is currently live. This preserves the existing API addressing model, where `/jobs/{job_id}` refers to one live handle at a time.

**Files:**

- Modify: `src/interfaces/api/mod.rs`
- Test: `tests/api.rs`

- [ ] **Step 1: Add `ApiError::conflict`**

In `impl ApiError`:

```rust
fn conflict(message: impl Into<String>) -> Self {
    Self {
        status: StatusCode::CONFLICT,
        message: message.into(),
    }
}
```

- [ ] **Step 2: Guard resumed job creation before inserting into the live map**

After resolving `resume_state` and before building the `JobRecord`:

```rust
let session_id = resume_state
    .as_ref()
    .map(|task_state| task_state.session_id)
    .unwrap_or_else(SessionId::new);
let job_id = resume_state
    .as_ref()
    .map(|task_state| task_state.job_id)
    .unwrap_or_else(JobId::new);

if resume_state.is_some() && live_job(&state, job_id).await.is_some() {
    return Err(ApiError::conflict(
        "cannot resume a job while its previous run is still active",
    ));
}
```

This must happen before:

```rust
state.inner.jobs.write().await.insert(job_id, record.clone());
```

- [ ] **Step 3: Add a regression test**

Add to `tests/api.rs`:

```rust
#[tokio::test]
async fn api_rejects_resume_when_job_is_still_live() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = test_workspace(tmp.path());
    let app = test_app(workspace.clone()).await;

    let created = create_job(
        app.clone(),
        serde_json::json!({
            "message": r#"{"tool":"request_input","args":{"prompt":"continue?"}}"#,
            "model": "fake-raw"
        }),
    )
    .await;

    let pending = wait_for_pending_input(app.clone(), created.job_id.to_string()).await;
    assert_eq!(pending.status, RunStatus::Running);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/jobs")
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "message": "resume while live",
                        "model": "fake",
                        "resume": created.run_id.to_string()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}
```

Adapt helper names to the existing test helpers in `tests/api.rs`; do not duplicate helper functions if they already exist.

- [ ] **Step 4: Verify API behavior**

Run:

```powershell
cargo test --test api api_rejects_resume_when_job_is_still_live -- --exact
cargo test --test api api_can_resume_latest_task_state -- --exact
cargo test --test api
```

Expected: PASS.

## Task 4: Default Tool Registry And Context-Bound Memory Tools

**Intent:** Remove duplicated interface assembly and remove misleading constructor arguments from memory tools. Runtime paths should come from `ToolContext.workspace`, not from stale constructor state.

**Files:**

- Modify: `src/tools/mod.rs`
- Modify: `src/tools/memory.rs`
- Modify: `src/main.rs`
- Modify: `src/interfaces/api/mod.rs`
- Modify: `tests/memory_tool.rs`
- Test: `tests/api.rs`
- Test: `tests/memory_tool.rs`

- [ ] **Step 1: Add a shared default registry builder**

In `src/tools/mod.rs`, add:

```rust
use crate::core::workspace::Workspace;
use crate::tools::echo::EchoTool;
use crate::tools::fs::{FsReadTool, FsWriteTool};
use crate::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use crate::tools::rag::RagRetrieveTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::request_input::RequestInputTool;
use crate::tools::shell::ShellTool;

pub fn default_tool_registry(workspace: &Workspace) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(ReadMemoryTopicTool::new()));
    registry.register(Box::new(SaveMemoryTool::new()));
    registry.register(Box::new(UpdateMemoryIndexTool::new()));
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
    registry.register(Box::new(RequestInputTool));
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));
    registry
}
```

This keeps root-bound tools root-bound and memory tools context-bound.

- [ ] **Step 2: Remove unused memory constructor arguments**

Change the memory tool constructors:

```rust
impl SaveMemoryTool {
    pub fn new() -> Self {
        Self
    }
}

impl UpdateMemoryIndexTool {
    pub fn new() -> Self {
        Self
    }
}

impl ReadMemoryTopicTool {
    pub fn new() -> Self {
        Self
    }
}
```

Keep this helper unchanged:

```rust
fn memory_dir(ctx: &ToolContext<'_>) -> PathBuf {
    ctx.workspace.state_dir.join("memory")
}
```

- [ ] **Step 3: Replace duplicated registry assembly in CLI and API**

In `src/main.rs`, replace manual registration with:

```rust
let mut registry = rove::tools::default_tool_registry(&workspace);
let mcp_config_path = config.resolve_path(&config.tool.mcp_config_path);
let mcp_tool_count = register_mcp_tools_from_file(&mut registry, mcp_config_path).await?;
```

In `src/interfaces/api/mod.rs`, replace manual registration with:

```rust
let registry = crate::tools::default_tool_registry(&workspace);
```

If import paths conflict inside the crate, use:

```rust
use crate::tools::default_tool_registry;
```

and then:

```rust
let registry = default_tool_registry(&workspace);
```

- [ ] **Step 4: Update tests that instantiate memory tools directly**

Replace:

```rust
registry.register(Box::new(SaveMemoryTool::new(workspace.root.clone())));
registry.register(Box::new(UpdateMemoryIndexTool::new(workspace.root.clone())));
registry.register(Box::new(ReadMemoryTopicTool::new(workspace.root.clone())));
```

with:

```rust
registry.register(Box::new(SaveMemoryTool::new()));
registry.register(Box::new(UpdateMemoryIndexTool::new()));
registry.register(Box::new(ReadMemoryTopicTool::new()));
```

Run:

```powershell
rg -n "SaveMemoryTool::new\\(|UpdateMemoryIndexTool::new\\(|ReadMemoryTopicTool::new\\(" src tests
```

Expected: every match uses zero arguments.

- [ ] **Step 5: Verify tool registration parity**

Run:

```powershell
cargo test --test memory_tool
cargo test --test api api_registers_save_memory_tool_for_jobs -- --exact
cargo test --test api api_registers_memory_index_and_topic_read_tools_for_jobs -- --exact
cargo test --test cli_config
```

Expected: PASS.

## Task 5: RAG Embedding/Rerank Governance Foundation

**Intent:** Prepare RAG provider calls to use the same routing and health patterns when production embedding or rerank providers are enabled. The current default deterministic RAG path remains unchanged.

**Files:**

- Modify: `src/tools/rag/embed.rs`
- Modify: `src/tools/rag/retrieve/postprocess.rs`
- Modify: `src/tools/rag/retrieve/pipeline.rs`
- Modify: `src/tools/rag/mod.rs`
- Test: RAG unit tests under `src/tools/rag/`
- Test: `tests/rag.rs` and `tests/rag_default.rs` as applicable

- [ ] **Step 1: Add identity to `Embedder`**

In `src/tools/rag/embed.rs`, extend the trait:

```rust
use crate::models::traits::ModelClientId;

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>>;

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque("embedding:unknown")
    }
}
```

Implement concrete IDs:

```rust
impl Embedder for DeterministicEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(deterministic_embedding(text))
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("embedding-deterministic", "local", "deterministic-64")
    }
}

impl Embedder for OpenAiEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        // existing implementation
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("embedding-openai-compatible", &self.api_base, &self.model)
    }
}
```

- [ ] **Step 2: Add `RoutingEmbedder` without changing default behavior**

In `src/tools/rag/embed.rs`, add:

```rust
use std::sync::Arc;
use crate::models::health::ModelHealthStore;

pub struct RoutingEmbedder {
    candidates: Vec<Box<dyn Embedder>>,
    health: Arc<ModelHealthStore>,
}

impl RoutingEmbedder {
    pub fn new(candidates: Vec<Box<dyn Embedder>>, health: Arc<ModelHealthStore>) -> Self {
        Self { candidates, health }
    }
}

#[async_trait]
impl Embedder for RoutingEmbedder {
    async fn embed(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut last_error = None;
        for candidate in &self.candidates {
            let candidate_id = candidate.client_id().to_string();
            if !self.health.allow_call(&candidate_id) {
                continue;
            }
            match candidate.embed(text).await {
                Ok(vector) => {
                    self.health.mark_success(&candidate_id);
                    return Ok(vector);
                }
                Err(err) => {
                    self.health.mark_failure(&candidate_id);
                    last_error = Some(err);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("all embedding candidates failed")))
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque("embedding-routing")
    }
}
```

This is synchronous-call routing, so it does not use first-packet probe semantics.

- [ ] **Step 3: Add focused `RoutingEmbedder` tests**

Add tests in `src/tools/rag/embed.rs`:

```rust
#[cfg(test)]
mod routing_tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use crate::models::health::{HealthConfig, ModelHealthStore};

    struct FailingEmbedder {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Embedder for FailingEmbedder {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            anyhow::bail!("embedding unavailable")
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque(self.id)
        }
    }

    struct StaticEmbedder {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Embedder for StaticEmbedder {
        async fn embed(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![1.0; EMBEDDING_DIMS])
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque(self.id)
        }
    }

    #[tokio::test]
    async fn routing_embedder_falls_back_after_failure() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: 1,
            open_cooldown: std::time::Duration::from_secs(30),
        }));
        let embedder = RoutingEmbedder::new(
            vec![
                Box::new(FailingEmbedder {
                    id: "embedding-primary",
                    calls: primary_calls.clone(),
                }),
                Box::new(StaticEmbedder {
                    id: "embedding-fallback",
                    calls: fallback_calls.clone(),
                }),
            ],
            health,
        );

        let vector = embedder.embed("project context").await.unwrap();

        assert_eq!(vector.len(), EMBEDDING_DIMS);
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }
}
```

Run with the RAG feature:

```powershell
cargo test --features rag routing_embedder_falls_back_after_failure -- --exact
```

Expected: PASS.

- [ ] **Step 4: Keep default retrieval deterministic**

Do not change `RagRetrieveTool` default construction in this task. It should still use:

```rust
let embedder = DeterministicEmbedder;
```

The new `RoutingEmbedder` is available for production embedding integration without changing local-first behavior.

Run:

```powershell
cargo test --test rag_default
cargo test --features rag --test rag
```

Expected: PASS.

- [ ] **Step 5: Record rerank governance as explicit interface work**

Current rerank is `NoopRerankPostProcessor`. Do not add a remote rerank provider in the same pass. Instead, add a short comment above `NoopRerankPostProcessor`:

```rust
/// Local deterministic rerank fallback. Remote rerank providers should be
/// introduced behind a routed rerank client that reuses ModelHealthStore.
pub struct NoopRerankPostProcessor;
```

This keeps the current retrieval pipeline stable and prevents a half-integrated remote rerank path.

## Task 6: Documentation And Verification

**Intent:** Keep implementation docs aligned with the new design boundaries.

**Files:**

- Modify: `docs/runtime/implementation-status.md`
- Modify: `docs/runtime/implementation-guide.md`
- Modify: this plan file if execution discovers a necessary deviation

- [ ] **Step 1: Update current status**

In `docs/runtime/implementation-status.md`, update these rows after implementation:

```markdown
| Routing and fallback | Fallback models and native fallback providers are supported. Fallback happens before committed visible output/tool-use, uses provider-aware target IDs, and shares API health state across jobs. Structured tracing records first-packet probe outcomes. | More detailed retry/backoff policy could be added. |
| RAG | Feature-gated staged RAG pipeline with LanceDB, manifest fallback, deterministic embeddings, retrieval channels, postprocessing, eval reports, RAG prompt formatting, lightweight code-aware chunking, and a routing embedder foundation for production providers. | Full production embedding/provider config and remote rerank provider integration remain a separate product scope. |
```

- [ ] **Step 2: Update implementation guide model section**

In `docs/runtime/implementation-guide.md`, update the model routing section to mention:

- `src/models/health.rs` owns `ModelHealthStore`.
- API state injects shared health into routed model clients.
- Provider target identity is `provider + endpoint + model`.
- Fallback still happens only before committed visible output/tool-use.
- First-packet outcomes are emitted through `tracing`.

- [ ] **Step 3: Run full verification**

Run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --features rag --test rag
cd web-ui
npm test
npm run typecheck
npm run build
```

Expected: every command exits successfully.

- [ ] **Step 4: Inspect git diff**

Run:

```powershell
git status --short --branch
git diff --stat
```

Expected:

- Source changes are limited to model health/routing, API job creation, tool registry assembly, memory constructors, RAG embedding foundation, tests, and docs.
- No generated build artifacts are staged.

## Final Acceptance Criteria

- API jobs share routing health state across runs within the same process.
- Health buckets cannot collide for different providers that use the same model name.
- Routing fallback behavior is unchanged: fallback before commit, no fallback after visible output/tool-use.
- First-packet probe outcomes are visible in structured tracing.
- API resume cannot overwrite a live job record.
- CLI and API use the same default tool registry builder.
- Memory tools no longer accept unused root paths and continue to write under `ToolContext.workspace.state_dir`.
- RAG keeps deterministic local defaults and has a routed embedder foundation for production provider integration.
- Full Rust, RAG, Web test/type/build verification passes.
