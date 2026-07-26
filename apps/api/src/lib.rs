use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::Query;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Json, Router};
use futures::{FutureExt, Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock, broadcast, oneshot, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_swagger_ui::SwaggerUi;

use rove_app_bootstrap::build_model_client_with_health;
use rove_app_bootstrap::{AppConfig, AppConfigOverrides};
use rove_app_bootstrap::{EngineOptions, build_engine};
use rove_core::ToolError;
use rove_models::ModelClient;
use rove_models::fake::FakeModelClient;
use rove_models::health::{HealthConfig, ModelHealthStore};
use rove_runtime::engine::Engine;
use rove_runtime::events::StreamEvent;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::index::{ResumeJobClaim, RunIndexRecord, StateIndex};
use rove_runtime::state::resume::resolve_resume_state;
use rove_runtime::state::store::{RunHandle, StateStore};
use rove_runtime::state::trace::TraceWriter;
use rove_runtime::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, Message, PendingToolApproval,
    PendingUserInput, Role, RunId, RunStatus, SessionId, TaskState, TerminationReason,
    ToolApprovalProvider, ToolApprovalRequest, UserInputProvider, UserInputRequest,
};
use rove_runtime::workspace::Workspace;

mod benchmark;
mod debug;
mod docs;
mod product;
mod provider;
mod security;
mod types;

use benchmark::BenchState;
pub use product::*;
pub use types::*;

use provider::{
    apply_provider_profile, normalize_provider_profile, provider_inventory, provider_key_env,
};

const EVENT_BUFFER: usize = 256;

#[derive(Clone)]
pub struct ApiState {
    inner: Arc<ApiStateInner>,
}

struct ApiStateInner {
    workspace: Workspace,
    config: AppConfig,
    product_store_path: PathBuf,
    product_store: Option<Arc<dyn ProductStore>>,
    product_transcript_reader: Option<Arc<dyn ProductTranscriptReader>>,
    shutdown_token: CancellationToken,
    supervisors: TaskTracker,
    jobs: RwLock<HashMap<JobId, Arc<JobRecord>>>,
    model_health: Arc<ModelHealthStore>,
    rate_limit: tokio::sync::Mutex<RateLimitState>,
    bench_runs: Arc<BenchState>,
}

#[derive(Debug, Default)]
struct RateLimitState {
    window_started_at: Option<Instant>,
    requests_in_window: u32,
}

#[derive(Clone)]
struct ApiProductRuntimeStateResolver {
    config: AppConfig,
}

impl ProductRuntimeStateResolver for ApiProductRuntimeStateResolver {
    fn state_store_for(
        &self,
        product_workspace: &ProductWorkspace,
    ) -> Result<StateStore, ProductStoreError> {
        let mut workspace = match product_workspace.kind {
            ProductWorkspaceKind::Folder => {
                Workspace::open_folder(&product_workspace.canonical_root)
            }
            ProductWorkspaceKind::Repo => Workspace::open_repo(&product_workspace.canonical_root),
        }
        .map_err(|err| {
            ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                format!("product workspace runtime state is unavailable: {err}"),
            )
        })?;
        let mut config = self.config.clone();
        config.rebase_to_workspace(&workspace.root);
        workspace.state_dir = config.state_dir();
        Ok(state_store_for_parts(&workspace, &config))
    }
}

struct JobRecord {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    workspace: Workspace,
    config: AppConfig,
    message: String,
    resumed_from_run_id: Option<RunId>,
    resume_state: Option<TaskState>,
    status: Mutex<RunStatus>,
    events: Mutex<Vec<JobStreamEvent>>,
    pending_approvals: Mutex<HashMap<CallId, PendingApproval>>,
    pending_inputs: Mutex<HashMap<CallId, PendingInput>>,
    tx: broadcast::Sender<JobStreamEvent>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    completion: watch::Sender<bool>,
    cancel_token: CancellationToken,
}

struct JobLaunch {
    record: Arc<JobRecord>,
    engine: Engine,
    run: RunHandle,
    product_turn: Option<ProductTurnSupervisor>,
}

#[derive(Clone)]
struct ProductTurnSupervisor {
    store: Arc<dyn ProductStore>,
    claim_id: ProductTurnClaimId,
}

struct JobCompletionGuard {
    completion: watch::Sender<bool>,
}

impl JobCompletionGuard {
    fn new(completion: watch::Sender<bool>) -> Self {
        Self { completion }
    }
}

impl Drop for JobCompletionGuard {
    fn drop(&mut self) {
        let _ = self.completion.send_replace(true);
    }
}

struct PendingApproval {
    request: ToolApprovalRequest,
    tx: oneshot::Sender<ApprovalDecision>,
}

struct PendingInput {
    request: UserInputRequest,
    tx: oneshot::Sender<String>,
}

#[derive(Debug, Deserialize)]
struct JobEventsQuery {
    after: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RunsQuery {
    limit: Option<usize>,
}

pub fn router(state: ApiState) -> Router {
    let (api_router, api) = OpenApiRouter::with_openapi(docs::ApiDoc::openapi())
        .routes(routes!(list_provider_models))
        .routes(routes!(test_provider))
        .routes(routes!(product::routes::list_product_workspaces))
        .routes(routes!(product::routes::create_product_workspace))
        .routes(routes!(product::routes::delete_product_workspace))
        .routes(routes!(product::routes::list_product_sessions))
        .routes(routes!(product::routes::create_product_session))
        .routes(routes!(product::routes::update_product_session))
        .routes(routes!(product::routes::delete_product_session))
        .routes(routes!(product::routes::get_product_session_transcript))
        .routes(routes!(product::routes::list_product_provider_profiles))
        .routes(routes!(product::routes::create_product_provider_profile))
        .routes(routes!(product::routes::update_product_provider_profile))
        .routes(routes!(product::routes::delete_product_provider_profile))
        .routes(routes!(product::routes::get_product_preferences))
        .routes(routes!(product::routes::update_product_preferences))
        .routes(routes!(product::routes::migrate_m1_browser_state))
        .routes(routes!(create_job))
        .routes(routes!(job_events))
        .routes(routes!(job_state))
        .routes(routes!(cancel_job))
        .routes(routes!(submit_approval))
        .routes(routes!(submit_input))
        .routes(routes!(list_runs))
        .routes(routes!(run_report))
        .routes(routes!(debug::list_memory))
        .routes(routes!(debug::get_memory_topic))
        .routes(routes!(debug::test_recall))
        .routes(routes!(benchmark::list_bench_suites))
        .routes(routes!(benchmark::start_bench_run))
        .routes(routes!(benchmark::list_bench_runs))
        .routes(routes!(benchmark::get_bench_run))
        .routes(routes!(benchmark::get_bench_task))
        .routes(routes!(benchmark::get_bench_evidence))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state,
            security::api_security,
        ))
        .split_for_parts();

    api_router.merge(SwaggerUi::new("/swagger-ui").url("/api/openapi.json", api))
}

pub async fn serve(addr: Option<SocketAddr>, cwd: PathBuf) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    tokio::spawn(async move {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!("failed to listen for Ctrl+C: {err}");
        }
        signal_shutdown.cancel();
    });
    serve_with_shutdown(addr, cwd, shutdown).await
}

pub async fn serve_with_shutdown(
    addr: Option<SocketAddr>,
    cwd: PathBuf,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let workspace = Workspace::detect(&cwd)?;
    let config = AppConfig::load(
        &workspace.root,
        AppConfigOverrides {
            api_bind_addr: addr.map(|addr| addr.to_string()),
            ..AppConfigOverrides::default()
        },
    )?;
    let configured_state_dir = config.state_dir();
    let workspace = Workspace {
        state_dir: configured_state_dir,
        ..workspace
    };
    workspace.ensure_state_dir()?;
    let addr: SocketAddr = config.api.bind_addr.parse()?;
    let state = ApiState::with_shutdown(workspace, config, shutdown.clone());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let result = serve_listener(listener, router(state.clone()), shutdown).await;
    state.inner.shutdown_token.cancel();
    drain_job_supervisors(&state).await;
    result
}

pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.cancelled().await;
            tracing::info!("API graceful shutdown initiated");
        })
        .await?;
    Ok(())
}

impl ApiState {
    pub fn new(workspace: Workspace, config: AppConfig) -> Self {
        Self::with_shutdown(workspace, config, CancellationToken::new())
    }

    pub fn with_shutdown(
        workspace: Workspace,
        mut config: AppConfig,
        shutdown_token: CancellationToken,
    ) -> Self {
        if config.source_summary.workspace_root == std::path::Path::new(".") {
            config.source_summary.workspace_root = workspace.root.clone();
            config.source_summary.project_config_path = workspace.root.join(".rove/config.toml");
        }
        let state_store = state_store_for_parts(&workspace, &config);
        if let Err(err) = state_store.index.initialize() {
            tracing::warn!("failed to initialize API state index: {err}");
        }
        if let Err(err) = state_store.index.mark_running_jobs_interrupted() {
            tracing::warn!("failed to mark stale API jobs interrupted: {err}");
        }
        let product_store_path = config.state_dir().join("product.sqlite");
        let product_store = match product::store::open_product_store(
            product_store_path.clone(),
            config.state.sqlite_busy_timeout_ms,
        ) {
            Ok(store) => Some(store),
            Err(err) => {
                tracing::warn!("product store is unavailable: {err}");
                None
            }
        };
        let product_transcript_reader = product_store.as_ref().and_then(|store| {
            let runtime_state_resolver: Arc<dyn ProductRuntimeStateResolver> =
                Arc::new(ApiProductRuntimeStateResolver {
                    config: config.clone(),
                });
            match product::transcript::open_product_transcript_reader(
                Arc::clone(store),
                runtime_state_resolver,
            ) {
                Ok(reader) => Some(reader),
                Err(err) => {
                    tracing::warn!("product transcript reader is unavailable: {err}");
                    None
                }
            }
        });
        let model_health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: config.routing.failure_threshold,
            open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
        }));
        Self {
            inner: Arc::new(ApiStateInner {
                workspace,
                config,
                product_store_path,
                product_store,
                product_transcript_reader,
                shutdown_token,
                supervisors: TaskTracker::new(),
                jobs: RwLock::new(HashMap::new()),
                model_health,
                rate_limit: tokio::sync::Mutex::new(RateLimitState::default()),
                bench_runs: Arc::new(BenchState::default()),
            }),
        }
    }

    /// Stable API-global ProductStore location. Requests cannot override it.
    pub fn product_store_path(&self) -> &FsPath {
        &self.inner.product_store_path
    }

    pub(crate) fn product_store(&self) -> Result<Arc<dyn ProductStore>, ApiError> {
        self.inner
            .product_store
            .clone()
            .ok_or_else(|| ProductStoreError::unavailable().into())
    }

    pub(crate) fn product_transcript_reader(
        &self,
    ) -> Result<Arc<dyn ProductTranscriptReader>, ApiError> {
        self.inner.product_transcript_reader.clone().ok_or_else(|| {
            ApiError::not_implemented(
                ProductErrorCode::ProductStoreUnavailable.as_str(),
                "the C0 product transcript reader is not wired yet",
            )
        })
    }
}

#[utoipa::path(
    post,
    path = "/jobs",
    tag = docs::JOBS_TAG,
    security(("BearerAuth" = [])),
    request_body = CreateJobRequest,
    responses(
        (status = 200, description = "Job created", body = CreateJobResponse, content_type = "application/json"),
        (status = 400, description = "Invalid job request", body = ApiErrorResponse, content_type = "application/json"),
        (status = 409, description = "Resume or product-session conflict", body = ApiErrorResponse, content_type = "application/json"),
        (status = 503, description = "Product store unavailable", body = ApiErrorResponse, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = ApiErrorResponse, content_type = "application/json")
    )
)]
async fn create_job(
    State(state): State<ApiState>,
    Json(req): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    if req.message.trim().is_empty() {
        return Err(ApiError::bad_request("message must not be empty"));
    }

    if req.product_session_id.is_some() && req.resume.is_some() {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionResumeConflict.as_str(),
            "product-session jobs resolve resume from the server binding; omit resume",
        ));
    }

    let launch = prepare_job_launch(&state, &req).await?;
    let record = Arc::clone(&launch.record);
    let response = CreateJobResponse {
        job_id: record.job_id,
        run_id: record.run_id,
        resumed_from_run_id: record.resumed_from_run_id,
    };
    start_job_supervisor(state, launch).await;

    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/providers/models",
    tag = docs::PROVIDERS_TAG,
    security(("BearerAuth" = [])),
    request_body = ProviderModelsRequest,
    responses(
        (status = 200, description = "Provider model catalog", body = ProviderModelsResponse, content_type = "application/json"),
        (status = 400, description = "Invalid provider profile or missing key env", body = serde_json::Value, content_type = "application/json"),
        (status = 502, description = "Provider model inventory request failed", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn list_provider_models(
    State(_state): State<ApiState>,
    Json(req): Json<ProviderModelsRequest>,
) -> Result<Json<ProviderModelsResponse>, ApiError> {
    let profile = normalize_provider_profile(&req.provider)?;
    let key_env = provider_key_env(&profile);
    let inventory = provider_inventory(&profile, &key_env, req.models_endpoint.as_deref()).await?;
    Ok(Json(ProviderModelsResponse {
        provider: profile.name,
        provider_type: profile.provider_type,
        wire_protocol: profile.wire_protocol,
        api_base: profile.api_base,
        key_env,
        key_present: inventory.key_present,
        models_count: inventory.models.len(),
        models: inventory.models,
    }))
}

#[utoipa::path(
    post,
    path = "/providers/test",
    tag = docs::PROVIDERS_TAG,
    security(("BearerAuth" = [])),
    request_body = ProviderTestRequest,
    responses(
        (status = 200, description = "Provider inventory check result", body = ProviderTestResponse, content_type = "application/json"),
        (status = 400, description = "Invalid provider profile", body = serde_json::Value, content_type = "application/json"),
        (status = 502, description = "Provider inventory request failed", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn test_provider(
    State(_state): State<ApiState>,
    Json(req): Json<ProviderTestRequest>,
) -> Result<Json<ProviderTestResponse>, ApiError> {
    let profile = normalize_provider_profile(&req.provider)?;
    let key_env = provider_key_env(&profile);
    let inventory = provider_inventory(&profile, &key_env, req.models_endpoint.as_deref()).await?;
    let model_present = req
        .model
        .as_ref()
        .map(|model| inventory.models.iter().any(|id| id == model));
    Ok(Json(ProviderTestResponse {
        status: "pass".to_string(),
        provider: profile.name,
        provider_type: Some(profile.provider_type),
        wire_protocol: Some(profile.wire_protocol),
        api_base: profile.api_base,
        key_env,
        key_present: inventory.key_present,
        model: req.model,
        model_present,
        models_count: inventory.models.len(),
    }))
}

#[utoipa::path(
    get,
    path = "/jobs/{job_id}/events",
    tag = docs::JOB_EVENTS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID"),
        ("after" = Option<u64>, Query, description = "Replay only events with seq greater than this value")
    ),
    responses(
        (status = 200, description = "Server-Sent Events stream of JobStreamEvent payloads", body = JobStreamEvent, content_type = "text/event-stream"),
        (status = 400, description = "Invalid Last-Event-ID header", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Failed to load persisted events", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn job_events(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
    Query(query): Query<JobEventsQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let after = query
        .after
        .or(parse_last_event_id(&headers)?)
        .unwrap_or_default();

    let Some(record) = live_job(&state, job_id).await else {
        let replay_events = persisted_job_events(&state, job_id, after).await?;
        let stream = futures::stream::iter(replay_events)
            .filter_map(|event| futures::future::ready(sse_event(event).ok()))
            .map(Ok)
            .boxed();
        return Ok(Sse::new(stream).keep_alive(KeepAlive::default()));
    };

    let live_rx = record.tx.subscribe();
    let (existing, status) = persisted_or_live_events(&state, &record, after).await?;
    let stream = replay_and_live_job_event_stream(existing, status, live_rx, after)
        .filter_map(|event| futures::future::ready(sse_event(event).ok()))
        .map(Ok)
        .boxed();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn replay_and_live_job_event_stream(
    existing: Vec<JobStreamEvent>,
    status: RunStatus,
    receiver: broadcast::Receiver<JobStreamEvent>,
    after: u64,
) -> futures::stream::BoxStream<'static, JobStreamEvent> {
    let live_terminal_published = is_terminal(&status);
    let replay_events: Vec<_> = existing
        .into_iter()
        .filter(|event| event.seq > after)
        .filter(|event| {
            live_terminal_published || !matches!(&event.event, StreamEvent::RunCompleted { .. })
        })
        .collect();
    let replay_has_terminal = replay_events
        .iter()
        .any(|event| matches!(&event.event, StreamEvent::RunCompleted { .. }));
    let replay_high_water = replay_events.last().map(|event| event.seq).unwrap_or(after);
    let replay = futures::stream::iter(replay_events);
    let live = if replay_has_terminal || live_terminal_published {
        futures::stream::empty().boxed()
    } else {
        live_job_event_stream(receiver, replay_high_water)
    };
    replay.chain(live).boxed()
}

fn live_job_event_stream(
    receiver: broadcast::Receiver<JobStreamEvent>,
    after: u64,
) -> futures::stream::BoxStream<'static, JobStreamEvent> {
    futures::stream::unfold(
        (receiver, false),
        move |(mut receiver, completed)| async move {
            if completed {
                return None;
            }
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let completed = matches!(&event.event, StreamEvent::RunCompleted { .. });
                        if event.seq > after {
                            return Some((event, (receiver, completed)));
                        }
                        if completed {
                            return None;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    )
    .boxed()
}

#[utoipa::path(
    get,
    path = "/jobs/{job_id}/state",
    tag = docs::JOBS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID")
    ),
    responses(
        (status = 200, description = "Current or persisted job state", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Job not found", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Failed to load persisted job state", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn job_state(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Json<JobStateResponse>, ApiError> {
    if let Some(record) = live_job(&state, job_id).await {
        return Ok(Json(job_state_response(&record).await));
    }
    Ok(Json(persisted_job_state_response(&state, job_id).await?))
}

#[utoipa::path(
    post,
    path = "/jobs/{job_id}/cancel",
    tag = docs::JOBS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID")
    ),
    responses(
        (status = 200, description = "Job state after cancellation request", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Live job not found", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn cancel_job(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let record = find_job(&state, job_id).await?;
    let current_status = record.status.lock().await.clone();
    if is_terminal(&current_status) {
        wait_for_job_completion(&record).await;
        return Ok(Json(job_state_response(&record).await));
    }

    record.cancel_token.cancel();
    let state_store = state_store_for_record(&record);
    reject_pending_approvals(&record, &state_store.index).await;
    reject_pending_inputs(&record, &state_store.index).await;
    wait_for_job_completion(&record).await;

    Ok(Json(job_state_response(&record).await))
}

#[utoipa::path(
    post,
    path = "/jobs/{job_id}/approvals/{call_id}",
    tag = docs::APPROVALS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID"),
        ("call_id" = String, Path, description = "Tool call ULID")
    ),
    request_body = SubmitApprovalRequest,
    responses(
        (status = 200, description = "Job state after resolving approval", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Job or pending approval not found", body = serde_json::Value, content_type = "application/json"),
        (status = 409, description = "Approval responder is no longer live", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn submit_approval(
    State(state): State<ApiState>,
    Path((job_id, call_id)): Path<(JobId, CallId)>,
    Json(req): Json<SubmitApprovalRequest>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let record = find_job(&state, job_id).await?;
    let pending = record
        .pending_approvals
        .lock()
        .await
        .remove(&call_id)
        .ok_or_else(|| ApiError::not_found("pending approval not found"))?;
    let index = state_store_for_record(&record).index;
    index
        .mark_pending_approval_status_async(call_id, approval_status(req.decision).to_string())
        .await
        .map_err(|err| ApiError::internal(format!("failed to persist approval decision: {err}")))?;
    if pending.tx.send(req.decision).is_err() {
        if let Err(err) = index
            .mark_pending_approval_status_async(call_id, "cancelled".to_string())
            .await
        {
            tracing::warn!(job_id = %job_id, call_id = %call_id, "failed to mark stale approval cancelled: {err}");
        }
        return Err(ApiError::conflict(
            "approval is no longer awaiting a response",
        ));
    }
    Ok(Json(job_state_response(&record).await))
}

#[utoipa::path(
    post,
    path = "/jobs/{job_id}/inputs/{input_id}",
    tag = docs::APPROVALS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("job_id" = String, Path, description = "Job ULID"),
        ("input_id" = String, Path, description = "Pending input ULID")
    ),
    request_body = SubmitInputRequest,
    responses(
        (status = 200, description = "Job state after answering input request", body = JobStateResponse, content_type = "application/json"),
        (status = 404, description = "Job or pending input not found", body = serde_json::Value, content_type = "application/json"),
        (status = 409, description = "Input responder is no longer live", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn submit_input(
    State(state): State<ApiState>,
    Path((job_id, input_id)): Path<(JobId, CallId)>,
    Json(req): Json<SubmitInputRequest>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let record = find_job(&state, job_id).await?;
    let pending = record
        .pending_inputs
        .lock()
        .await
        .remove(&input_id)
        .ok_or_else(|| ApiError::not_found("pending input not found"))?;
    let index = state_store_for_record(&record).index;
    index
        .mark_pending_input_status_async(input_id, "answered".to_string())
        .await
        .map_err(|err| ApiError::internal(format!("failed to persist input answer: {err}")))?;
    if pending.tx.send(req.answer).is_err() {
        if let Err(err) = index
            .mark_pending_input_status_async(input_id, "cancelled".to_string())
            .await
        {
            tracing::warn!(job_id = %job_id, input_id = %input_id, "failed to mark stale input cancelled: {err}");
        }
        return Err(ApiError::conflict("input is no longer awaiting a response"));
    }
    Ok(Json(job_state_response(&record).await))
}

#[utoipa::path(
    get,
    path = "/runs",
    tag = docs::RUNS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("limit" = Option<usize>, Query, description = "Maximum number of run summaries to return")
    ),
    responses(
        (status = 200, description = "Recent run summaries", body = ListRunsResponse, content_type = "application/json"),
        (status = 500, description = "Failed to list runs", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn list_runs(
    State(state): State<ApiState>,
    Query(query): Query<RunsQuery>,
) -> Result<Json<ListRunsResponse>, ApiError> {
    let state_store = state_store_for_api(&state);
    let records = state_store
        .index
        .list_run_records_async(query.limit.unwrap_or(50))
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(ListRunsResponse {
        runs: records
            .into_iter()
            .map(|record| RunSummaryResponse {
                run_id: record.run_id,
                session_id: record.session_id,
                job_id: record.job_id,
                status: run_status_from_index(&record.status),
                last_event_seq: record.last_event_seq,
                has_report: record.report_path.is_some(),
            })
            .collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/runs/{run_id}/report",
    tag = docs::RUNS_TAG,
    security(("BearerAuth" = [])),
    params(
        ("run_id" = String, Path, description = "Run ULID")
    ),
    responses(
        (status = 200, description = "Persisted run report", body = serde_json::Value, content_type = "application/json"),
        (status = 404, description = "Run report not found", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Failed to load run report", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn run_report(
    State(state): State<ApiState>,
    Path(run_id): Path<RunId>,
) -> Result<Json<rove_runtime::state::report::RunReport>, ApiError> {
    let state_store = state_store_for_api(&state);
    let report = state_store
        .load_report(run_id)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => ApiError::not_found("run report not found"),
            _ => ApiError::internal(err),
        })?;
    Ok(Json(report))
}

async fn prepare_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
) -> Result<JobLaunch, ApiError> {
    match req.product_session_id.as_ref() {
        Some(product_session_id) => {
            prepare_product_job_launch(state, req, product_session_id).await
        }
        None => prepare_generic_job_launch(state, req).await,
    }
}

async fn prepare_generic_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
) -> Result<JobLaunch, ApiError> {
    let (workspace, config) = workspace_and_config_for_create_job(state, req)?;
    let state_store = state_store_for_parts(&workspace, &config);
    let resume_state = resolve_resume_state(&state_store, req.resume.as_deref())
        .await
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    if req.resume.is_some() && resume_state.is_none() {
        return Err(ApiError::bad_request(
            "nothing to resume in this workspace; hard resume requires durable task_state under the requested workspace root",
        ));
    }

    let mut resume_claim = claim_runtime_resume(&state_store, resume_state.as_ref(), false).await?;
    let session_id = resume_state
        .as_ref()
        .map(|task_state| task_state.session_id)
        .unwrap_or_else(SessionId::new);
    let job_id = resume_state
        .as_ref()
        .map(|task_state| task_state.job_id)
        .unwrap_or_else(JobId::new);
    let record = new_job_record(
        state,
        workspace,
        config,
        req,
        session_id,
        job_id,
        resume_state,
    );
    let engine = match assemble_job_engine(state, &record.message, req, Arc::clone(&record)).await {
        Ok(engine) => engine,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            tracing::warn!(job_id = %record.job_id, "failed to assemble job engine: {error}");
            return Err(ApiError::internal("failed to assemble job engine"));
        }
    };
    let run = match state_store.start_run(record.session_id, record.job_id, record.run_id) {
        Ok(run) => run,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            return Err(ApiError::internal(error));
        }
    };

    Ok(JobLaunch {
        record,
        engine,
        run,
        product_turn: None,
    })
}

async fn prepare_product_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
    product_session_id: &ProductSessionId,
) -> Result<JobLaunch, ApiError> {
    let store = state.product_store()?;
    let claim = store.claim_session_turn(product_session_id).await?;
    let claim_id = claim.claim_id.clone();
    let previous_product_status = claim.previous_status;

    let (workspace, config) =
        match workspace_and_config_for_product_job(state, req, &claim.context.workspace) {
            Ok(value) => value,
            Err(error) => {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    previous_product_status,
                    "workspace validation",
                )
                .await;
                return Err(error);
            }
        };
    let state_store = state_store_for_parts(&workspace, &config);
    let (resume_state, mut resume_claim) = match claim.previous_binding.as_ref() {
        Some(previous) => match load_and_claim_product_resume(&state_store, previous).await {
            Ok((resume_state, resume_claim)) => (Some(resume_state), Some(resume_claim)),
            Err(error) => {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    ProductSessionStatus::NeedsAttention,
                    "exact runtime resume validation",
                )
                .await;
                return Err(error);
            }
        },
        None => (None, None),
    };
    let session_id = resume_state
        .as_ref()
        .map(|task_state| task_state.session_id)
        .unwrap_or_else(SessionId::new);
    let job_id = resume_state
        .as_ref()
        .map(|task_state| task_state.job_id)
        .unwrap_or_else(JobId::new);
    let record = new_job_record(
        state,
        workspace,
        config,
        req,
        session_id,
        job_id,
        resume_state,
    );
    let engine = match assemble_job_engine(state, &record.message, req, Arc::clone(&record)).await {
        Ok(engine) => engine,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            finish_failed_product_start(
                &store,
                &claim_id,
                previous_product_status,
                "engine assembly",
            )
            .await;
            tracing::warn!(job_id = %record.job_id, "failed to assemble product job engine: {error}");
            return Err(ApiError::internal("failed to assemble job engine"));
        }
    };
    let run = match state_store.start_run(record.session_id, record.job_id, record.run_id) {
        Ok(run) => run,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            finish_failed_product_start(
                &store,
                &claim_id,
                ProductSessionStatus::NeedsAttention,
                "runtime run start",
            )
            .await;
            tracing::warn!(product_session_id = %product_session_id, "failed to start product runtime run: {error}");
            return Err(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                "the product session runtime store could not start a run",
            )
            .into());
        }
    };
    let _ = resume_claim.take();

    if let Err(error) = store
        .commit_run_binding(CommitProductRunBinding {
            claim_id: claim_id.clone(),
            product_session_id: product_session_id.clone(),
            runtime_session_id: record.session_id,
            runtime_job_id: record.job_id,
            runtime_run_id: record.run_id,
            resumed_from_run_id: record.resumed_from_run_id,
        })
        .await
    {
        finalize_prestarted_run(
            &record,
            &engine,
            run,
            "product run binding was not committed",
        )
        .await;
        finish_failed_product_start(
            &store,
            &claim_id,
            ProductSessionStatus::NeedsAttention,
            "runtime binding commit",
        )
        .await;
        return Err(error.into());
    }

    Ok(JobLaunch {
        record,
        engine,
        run,
        product_turn: Some(ProductTurnSupervisor { store, claim_id }),
    })
}

fn new_job_record(
    state: &ApiState,
    workspace: Workspace,
    config: AppConfig,
    req: &CreateJobRequest,
    session_id: SessionId,
    job_id: JobId,
    resume_state: Option<TaskState>,
) -> Arc<JobRecord> {
    let run_id = RunId::new();
    let resumed_from_run_id = resume_state.as_ref().map(|task_state| task_state.run_id);
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let (completion, _) = watch::channel(false);
    Arc::new(JobRecord {
        session_id,
        job_id,
        run_id,
        workspace,
        config,
        message: req.message.clone(),
        resumed_from_run_id,
        resume_state,
        status: Mutex::new(RunStatus::Running),
        events: Mutex::new(Vec::new()),
        pending_approvals: Mutex::new(HashMap::new()),
        pending_inputs: Mutex::new(HashMap::new()),
        tx,
        handle: Mutex::new(None),
        completion,
        cancel_token: state.inner.shutdown_token.child_token(),
    })
}

async fn claim_runtime_resume(
    state_store: &StateStore,
    resume_state: Option<&TaskState>,
    product: bool,
) -> Result<Option<ResumeJobClaim>, ApiError> {
    let Some(resume_state) = resume_state else {
        return Ok(None);
    };
    let claim = state_store
        .index
        .claim_job_for_resume_async(resume_state.job_id, resume_state.run_id)
        .await
        .map_err(ApiError::internal)?;
    match claim {
        Some(claim) => Ok(Some(claim)),
        None if product => Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionResumeConflict.as_str(),
            "the product session runtime run is active, stale, or no longer the job's latest terminal run",
        )),
        None => Err(ApiError::conflict(
            "cannot resume a job unless the requested run is its latest terminal run",
        )),
    }
}

async fn release_runtime_resume_claim(state_store: &StateStore, claim: Option<ResumeJobClaim>) {
    let Some(claim) = claim else {
        return;
    };
    match state_store
        .index
        .release_job_resume_claim_async(claim)
        .await
    {
        Ok(true) => {}
        Ok(false) => tracing::warn!("runtime resume claim was no longer releasable"),
        Err(error) => tracing::warn!("failed to release runtime resume claim: {error}"),
    }
}

async fn load_and_claim_product_resume(
    state_store: &StateStore,
    previous: &ProductRuntimeBinding,
) -> Result<(TaskState, ResumeJobClaim), ApiError> {
    let job = state_store
        .index
        .job_record_async(previous.latest_job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                "the product session runtime job is missing",
            ))
        })?;
    if job.session_id != previous.runtime_session_id || job.run_id != Some(previous.latest_run_id) {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateCorrupt,
            "the product session runtime job identity does not match its binding",
        )
        .into());
    }
    let run = load_runtime_run_record(&state_store.index, previous.latest_run_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::from(ProductStoreError::new(
                ProductErrorCode::ProductSessionRuntimeStateMissing,
                "the product session runtime run is missing",
            ))
        })?;
    if run.session_id != previous.runtime_session_id
        || run.job_id != previous.latest_job_id
        || run.run_id != previous.latest_run_id
    {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateCorrupt,
            "the product session runtime run identity does not match its binding",
        )
        .into());
    }
    // Loading task state may lazily repair its index projection. Validate the
    // existing indexed identities first so product resume fails closed on drift.
    let resume_state = state_store
        .load_task_state(previous.latest_run_id)
        .await
        .map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                ProductErrorCode::ProductSessionRuntimeStateMissing
            } else {
                ProductErrorCode::ProductSessionRuntimeStateCorrupt
            };
            ApiError::from(ProductStoreError::new(
                code,
                "the product session's exact runtime task state is unavailable or invalid",
            ))
        })?;
    if resume_state.session_id != previous.runtime_session_id
        || resume_state.job_id != previous.latest_job_id
        || resume_state.run_id != previous.latest_run_id
    {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateCorrupt,
            "the product session runtime task-state identity does not match its binding",
        )
        .into());
    }
    let resume_state = project_product_follow_up_state(resume_state)?;
    let Some(claim) = claim_runtime_resume(state_store, Some(&resume_state), true).await? else {
        return Err(ApiError::internal(
            "product resume validation did not acquire a runtime claim",
        ));
    };
    Ok((resume_state, claim))
}

fn project_product_follow_up_state(mut state: TaskState) -> Result<TaskState, ApiError> {
    // A product follow-up is a new user turn in the same durable conversation,
    // not a replay of the previous turn's terminal execution decision.
    state.step = 0;
    state.plan = None;
    state.step_ledger = Default::default();
    state.history = close_product_follow_up_tool_rounds(state.history)?;
    if let Some(checkpoint) = state.checkpoint.as_mut() {
        checkpoint.last_step = 0;
        checkpoint.plan = None;
        checkpoint.step_ledger = Default::default();
        checkpoint.last_event_seq = None;
        checkpoint.preserved_tail =
            close_product_follow_up_tool_rounds(std::mem::take(&mut checkpoint.preserved_tail))?;
    }
    Ok(state)
}

const UNKNOWN_PRODUCT_TOOL_RESULT: &str = "The previous turn ended before a durable tool result was recorded. The tool effect is unknown; verify the current state before retrying this tool call.";

fn close_product_follow_up_tool_rounds(messages: Vec<Message>) -> Result<Vec<Message>, ApiError> {
    let mut closed = Vec::with_capacity(messages.len());
    let mut pending_tool_call_ids = Vec::new();
    let mut completed_tool_call_ids = HashSet::new();

    for message in messages {
        if message.role == Role::Tool {
            let Some(tool_call_id) = message.tool_call_id.as_ref() else {
                append_missing_product_tool_results(
                    &mut closed,
                    &pending_tool_call_ids,
                    &completed_tool_call_ids,
                );
                pending_tool_call_ids.clear();
                completed_tool_call_ids.clear();
                closed.push(message);
                continue;
            };
            if tool_call_id.trim().is_empty() {
                return Err(invalid_product_tool_history(
                    "the product session runtime history contains an empty tool result call id",
                ));
            }
            if pending_tool_call_ids.contains(tool_call_id)
                && completed_tool_call_ids.insert(tool_call_id.clone())
            {
                closed.push(message);
            }
            continue;
        }

        append_missing_product_tool_results(
            &mut closed,
            &pending_tool_call_ids,
            &completed_tool_call_ids,
        );
        pending_tool_call_ids.clear();
        completed_tool_call_ids.clear();

        if message.role == Role::Assistant && !message.tool_calls.is_empty() {
            let mut round_tool_call_ids = HashSet::new();
            for tool_call in &message.tool_calls {
                if tool_call.id.trim().is_empty() {
                    return Err(invalid_product_tool_history(
                        "the product session runtime history contains an empty assistant tool call id",
                    ));
                }
                if !round_tool_call_ids.insert(tool_call.id.clone()) {
                    return Err(invalid_product_tool_history(
                        "the product session runtime history contains duplicate assistant tool call ids",
                    ));
                }
                pending_tool_call_ids.push(tool_call.id.clone());
            }
        }
        closed.push(message);
    }

    append_missing_product_tool_results(
        &mut closed,
        &pending_tool_call_ids,
        &completed_tool_call_ids,
    );
    Ok(closed)
}

fn invalid_product_tool_history(message: &'static str) -> ApiError {
    ProductStoreError::new(ProductErrorCode::ProductSessionRuntimeStateCorrupt, message).into()
}

fn append_missing_product_tool_results(
    messages: &mut Vec<Message>,
    pending_tool_call_ids: &[String],
    completed_tool_call_ids: &HashSet<String>,
) {
    for tool_call_id in pending_tool_call_ids {
        if !completed_tool_call_ids.contains(tool_call_id) {
            messages.push(Message::tool(
                UNKNOWN_PRODUCT_TOOL_RESULT,
                Some(tool_call_id.clone()),
            ));
        }
    }
}

async fn finish_failed_product_start(
    store: &Arc<dyn ProductStore>,
    claim_id: &ProductTurnClaimId,
    status: ProductSessionStatus,
    phase: &'static str,
) {
    if let Err(error) = store.finish_session_turn(claim_id, status).await {
        tracing::warn!(
            phase = phase,
            "failed to release product turn after start failure: {error}"
        );
    }
}

async fn finalize_prestarted_run(
    record: &JobRecord,
    engine: &Engine,
    run: RunHandle,
    message: &str,
) {
    let state_store = state_store_for_record(record);
    let terminal = StreamEvent::RunCompleted {
        reason: TerminationReason::Error,
        output: Some(message.to_string()),
    };
    let trace_persisted = append_trace_event(&run.trace_writer, record, &terminal);
    let mut recorder = RunArtifactRecorder::new(
        record.session_id,
        record.job_id,
        record.run_id,
        record.message.clone(),
        record.resume_state.as_ref(),
        Some(engine.runtime_identity()),
    );
    recorder.record_event(&terminal, &state_store).await;
    recorder
        .finalize(
            &state_store,
            engine.workspace(),
            engine.model_id(),
            &run.run_dir,
        )
        .await;
    if !trace_persisted || !runtime_terminal_is_durable(&state_store, record, &terminal).await {
        tracing::warn!(
            job_id = %record.job_id,
            run_id = %record.run_id,
            "prestarted runtime run did not reach a fully durable terminal state"
        );
    }
}

async fn start_job_supervisor(state: ApiState, launch: JobLaunch) {
    let JobLaunch {
        record,
        engine,
        run,
        product_turn,
    } = launch;
    let record_for_task = Arc::clone(&record);
    let recovery_record = Arc::clone(&record);
    let recovery_product_turn = product_turn.clone();
    let completion = record.completion.clone();
    let handle = state.inner.supervisors.spawn(async move {
        let _completion_guard = JobCompletionGuard::new(completion);
        let outcome = AssertUnwindSafe(run_job_supervisor(
            record_for_task,
            engine,
            run,
            product_turn,
        ))
        .catch_unwind()
        .await;
        if outcome.is_err() {
            tracing::warn!(job_id = %recovery_record.job_id, "job supervisor panicked");
            let recovery = AssertUnwindSafe(recover_job_supervisor_panic(
                &recovery_record,
                recovery_product_turn,
            ))
            .catch_unwind()
            .await;
            if recovery.is_err() {
                tracing::warn!(job_id = %recovery_record.job_id, "job supervisor recovery panicked");
                let terminal_published = {
                    let status = recovery_record.status.lock().await;
                    is_terminal(&status)
                };
                if !terminal_published {
                    append_job_event(
                        &recovery_record,
                        supervisor_failure_event("job supervisor failed"),
                    )
                    .await;
                }
            }
        }
    });
    *record.handle.lock().await = Some(handle);
    state.inner.jobs.write().await.insert(record.job_id, record);
}

async fn run_job_supervisor(
    record: Arc<JobRecord>,
    engine: Engine,
    run: RunHandle,
    product_turn: Option<ProductTurnSupervisor>,
) {
    let state_store = state_store_for_record(&record);
    let mut recorder = RunArtifactRecorder::new(
        record.session_id,
        record.job_id,
        record.run_id,
        record.message.clone(),
        record.resume_state.as_ref(),
        Some(engine.runtime_identity()),
    );
    let stream_outcome = AssertUnwindSafe(consume_job_stream(
        &record,
        &engine,
        &run,
        &state_store,
        &mut recorder,
    ))
    .catch_unwind()
    .await;
    let (terminal, needs_attention, stream_trace_complete) = match stream_outcome {
        Ok(Some((terminal, trace_complete))) => (terminal, false, trace_complete),
        Ok(None) => (
            supervisor_failure_event("runtime stream ended without exactly one terminal event"),
            true,
            false,
        ),
        Err(_) => {
            tracing::warn!(job_id = %record.job_id, "job runtime panicked");
            (supervisor_failure_event("job runtime failed"), true, false)
        }
    };
    let trace_persisted = append_trace_event(&run.trace_writer, &record, &terminal);
    recorder.record_event(&terminal, &state_store).await;
    recorder
        .finalize(
            &state_store,
            engine.workspace(),
            engine.model_id(),
            &run.run_dir,
        )
        .await;

    let terminal_status = terminal_run_status(&terminal);
    if matches!(terminal_status, RunStatus::Cancelled | RunStatus::Error) {
        reject_pending_approvals(&record, &state_store.index).await;
        reject_pending_inputs(&record, &state_store.index).await;
    }
    let runtime_durable = stream_trace_complete
        && trace_persisted
        && runtime_terminal_is_durable(&state_store, &record, &terminal).await;
    if !runtime_durable {
        tracing::warn!(
            job_id = %record.job_id,
            run_id = %record.run_id,
            "runtime terminal artifacts are incomplete"
        );
    }
    finish_product_turn(product_turn, &terminal, needs_attention || !runtime_durable).await;
    append_job_event(&record, terminal).await;
}

async fn consume_job_stream(
    record: &JobRecord,
    engine: &Engine,
    run: &RunHandle,
    state_store: &StateStore,
    recorder: &mut RunArtifactRecorder,
) -> Option<(StreamEvent, bool)> {
    let request = run.request(record.message.clone(), record.resume_state.clone());
    let mut stream =
        std::pin::pin!(engine.run_with_cancel(request, None, record.cancel_token.clone(),));
    let mut terminal = None;
    let mut protocol_invalid = false;
    let mut trace_complete = true;
    while let Some(event) = stream.next().await {
        if matches!(&event, StreamEvent::RunCompleted { .. }) {
            if terminal.replace(event).is_some() {
                protocol_invalid = true;
            }
            continue;
        }
        if terminal.is_some() {
            protocol_invalid = true;
            continue;
        }
        trace_complete &= append_trace_event(&run.trace_writer, record, &event);
        recorder.record_event(&event, state_store).await;
        append_job_event(record, event).await;
    }
    if protocol_invalid {
        None
    } else {
        terminal.map(|terminal| (terminal, trace_complete))
    }
}

async fn finish_product_turn(
    product_turn: Option<ProductTurnSupervisor>,
    terminal: &StreamEvent,
    needs_attention: bool,
) {
    let Some(product_turn) = product_turn else {
        return;
    };
    let status = if needs_attention {
        ProductSessionStatus::NeedsAttention
    } else {
        match terminal_run_status(terminal) {
            RunStatus::Error | RunStatus::Interrupted => ProductSessionStatus::Error,
            RunStatus::Init | RunStatus::Running => ProductSessionStatus::NeedsAttention,
            RunStatus::Done | RunStatus::Cancelled => ProductSessionStatus::Idle,
        }
    };
    if let Err(error) = product_turn
        .store
        .finish_session_turn(&product_turn.claim_id, status)
        .await
    {
        tracing::warn!("failed to finish product session turn: {error}");
        if status != ProductSessionStatus::NeedsAttention
            && let Err(retry_error) = product_turn
                .store
                .finish_session_turn(&product_turn.claim_id, ProductSessionStatus::NeedsAttention)
                .await
        {
            tracing::warn!("failed to conservatively release product session turn: {retry_error}");
        }
    }
}

async fn recover_job_supervisor_panic(
    record: &JobRecord,
    product_turn: Option<ProductTurnSupervisor>,
) {
    let state_store = state_store_for_record(record);
    let terminal = match load_persisted_terminal_event(&state_store.index, record.run_id).await {
        Ok(Some(event)) => event,
        Ok(None) => supervisor_failure_event("job supervisor failed"),
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to inspect terminal event during supervisor recovery: {error}");
            supervisor_failure_event("job supervisor failed")
        }
    };
    reject_pending_approvals(record, &state_store.index).await;
    reject_pending_inputs(record, &state_store.index).await;
    let finish_outcome = AssertUnwindSafe(finish_product_turn(product_turn, &terminal, true))
        .catch_unwind()
        .await;
    if finish_outcome.is_err() {
        tracing::warn!(job_id = %record.job_id, "product turn recovery panicked");
    }
    let terminal_published = {
        let status = record.status.lock().await;
        is_terminal(&status)
    };
    if !terminal_published {
        append_job_event(record, terminal).await;
    }
}

async fn load_persisted_terminal_event(
    index: &StateIndex,
    run_id: RunId,
) -> std::io::Result<Option<StreamEvent>> {
    let Some(run) = load_runtime_run_record(index, run_id).await? else {
        return Ok(None);
    };
    if run.last_event_seq == 0 {
        return Ok(None);
    }
    let snapshot = index
        .run_event_snapshot_async(run_id, run.last_event_seq.saturating_sub(1), 1)
        .await?;
    let Some(event) = snapshot.and_then(|snapshot| snapshot.events.into_iter().next()) else {
        return Ok(None);
    };
    let event = serde_json::from_str::<StreamEvent>(&event.event_json)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    if matches!(&event, StreamEvent::RunCompleted { .. }) {
        Ok(Some(event))
    } else {
        Ok(None)
    }
}

async fn runtime_terminal_is_durable(
    state_store: &StateStore,
    record: &JobRecord,
    terminal: &StreamEvent,
) -> bool {
    let run = match load_runtime_run_record(&state_store.index, record.run_id).await {
        Ok(Some(run)) => run,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to inspect finalized runtime run: {error}");
            return false;
        }
    };
    if run.session_id != record.session_id
        || run.job_id != record.job_id
        || run.run_id != record.run_id
        || run.status != run_status_label(&terminal_run_status(terminal))
        || run.task_state_path.is_none()
        || run.report_path.is_none()
        || run.last_event_seq == 0
    {
        return false;
    }
    let task_state = match state_store.load_task_state(record.run_id).await {
        Ok(task_state) => task_state,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to reload finalized task state: {error}");
            return false;
        }
    };
    if task_state.session_id != record.session_id
        || task_state.job_id != record.job_id
        || task_state.run_id != record.run_id
        || task_state
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.last_event_seq)
            != Some(run.last_event_seq)
    {
        return false;
    }
    let report = match state_store.load_report(record.run_id).await {
        Ok(report) => report,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to reload finalized report: {error}");
            return false;
        }
    };
    if report.session_id != record.session_id
        || report.job_id != record.job_id
        || report.run_id != record.run_id
    {
        return false;
    }
    let persisted = match load_persisted_terminal_event(&state_store.index, record.run_id).await {
        Ok(Some(event)) => event,
        Ok(None) => return false,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to reload finalized terminal event: {error}");
            return false;
        }
    };
    terminal_events_match(&persisted, terminal)
        && matches!(
            terminal,
            StreamEvent::RunCompleted { reason, .. } if &report.termination_reason == reason
        )
}

fn terminal_events_match(left: &StreamEvent, right: &StreamEvent) -> bool {
    match (left, right) {
        (
            StreamEvent::RunCompleted {
                reason: left_reason,
                output: left_output,
            },
            StreamEvent::RunCompleted {
                reason: right_reason,
                output: right_output,
            },
        ) => left_reason == right_reason && left_output == right_output,
        _ => false,
    }
}

async fn load_runtime_run_record(
    index: &StateIndex,
    run_id: RunId,
) -> std::io::Result<Option<RunIndexRecord>> {
    let index = index.clone();
    tokio::task::spawn_blocking(move || index.run_record(run_id))
        .await
        .map_err(std::io::Error::other)?
}

fn supervisor_failure_event(message: &str) -> StreamEvent {
    StreamEvent::RunCompleted {
        reason: TerminationReason::Error,
        output: Some(message.to_string()),
    }
}

fn terminal_run_status(event: &StreamEvent) -> RunStatus {
    match event {
        StreamEvent::RunCompleted { reason, .. } => status_for_reason(reason),
        _ => RunStatus::Error,
    }
}

fn run_status_label(status: &RunStatus) -> &'static str {
    match status {
        RunStatus::Init => "init",
        RunStatus::Running => "running",
        RunStatus::Done => "done",
        RunStatus::Error => "error",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Interrupted => "interrupted",
    }
}

fn append_trace_event(trace_writer: &TraceWriter, record: &JobRecord, event: &StreamEvent) -> bool {
    if let Err(error) = trace_writer.append(event) {
        tracing::warn!(job_id = %record.job_id, run_id = %record.run_id, "failed to append runtime trace event: {error}");
        return false;
    }
    true
}

async fn assemble_job_engine(
    state: &ApiState,
    message: &str,
    req: &CreateJobRequest,
    record: Arc<JobRecord>,
) -> anyhow::Result<Engine> {
    let config = &record.config;
    let model_id = req
        .model
        .clone()
        .unwrap_or_else(|| config.provider.model.clone());
    let model: Box<dyn ModelClient> = match model_id.as_str() {
        "fake" => Box::new(FakeModelClient::new(format!("fake response: {message}"))),
        "fake-raw" => Box::new(FakeModelClient::new(message.to_string())),
        _ => build_model_client_with_health(config, model_id, state.inner.model_health.clone()),
    };

    let workspace = record.workspace.clone();
    let state_store = state_store_for_record(&record);
    let approval_policy = req.approval.unwrap_or(ApprovalPolicy::Ask);
    let input_provider: Arc<dyn UserInputProvider> = Arc::new(ApiInputProvider {
        record: record.clone(),
        index: state_store.index.clone(),
    });
    let approval_provider = (approval_policy == ApprovalPolicy::Ask).then(|| {
        Arc::new(ApiApprovalProvider {
            record: record.clone(),
            index: state_store.index.clone(),
        }) as Arc<dyn ToolApprovalProvider>
    });

    build_engine(EngineOptions {
        model,
        workspace: &workspace,
        config,
        max_steps: req.max_steps.unwrap_or(config.runtime.max_steps),
        approval_policy,
        input_provider: Some(input_provider),
        approval_provider,
    })
    .await
}

fn state_store_for_api(state: &ApiState) -> StateStore {
    state_store_for_parts(&state.inner.workspace, &state.inner.config)
}

fn state_store_for_record(record: &JobRecord) -> StateStore {
    state_store_for_parts(&record.workspace, &record.config)
}

fn workspace_and_config_for_create_job(
    state: &ApiState,
    req: &CreateJobRequest,
) -> Result<(Workspace, AppConfig), ApiError> {
    let (workspace, mut config) = workspace_for_create_job(state, req.workspace.as_ref())?;
    if let Some(profile) = &req.provider {
        apply_provider_profile(&mut config, profile, req.model.as_deref())?;
    }
    Ok((workspace, config))
}

fn workspace_and_config_for_product_job(
    state: &ApiState,
    req: &CreateJobRequest,
    product_workspace: &ProductWorkspace,
) -> Result<(Workspace, AppConfig), ApiError> {
    let workspace = open_product_workspace(product_workspace)?;
    if let Some(requested) = req.workspace.as_ref() {
        validate_product_workspace_hint(requested, product_workspace, &workspace)?;
    }
    let (workspace, mut config) = rebased_workspace_config(state, workspace)?;
    if let Some(profile) = &req.provider {
        apply_provider_profile(&mut config, profile, req.model.as_deref())?;
    }
    Ok((workspace, config))
}

fn open_product_workspace(product_workspace: &ProductWorkspace) -> Result<Workspace, ApiError> {
    let workspace = match product_workspace.kind {
        ProductWorkspaceKind::Folder => Workspace::open_folder(&product_workspace.canonical_root),
        ProductWorkspaceKind::Repo => Workspace::open_repo(&product_workspace.canonical_root),
    }
    .map_err(|error| {
        tracing::warn!(product_workspace_id = %product_workspace.id, "failed to open catalog workspace: {error}");
        ApiError::from(ProductStoreError::new(
            ProductErrorCode::ProductSessionRuntimeStateMissing,
            "the product session workspace is unavailable",
        ))
    })?;
    if workspace.root != product_workspace.canonical_root {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductBindingCorrupt,
            "the product workspace canonical root no longer matches its catalog binding",
        )
        .into());
    }
    Ok(workspace)
}

fn validate_product_workspace_hint(
    requested: &CreateJobWorkspace,
    product_workspace: &ProductWorkspace,
    server_workspace: &Workspace,
) -> Result<(), ApiError> {
    let kind_matches = matches!(
        (requested.kind, product_workspace.kind),
        (CreateJobWorkspaceKind::Folder, ProductWorkspaceKind::Folder)
            | (CreateJobWorkspaceKind::Repo, ProductWorkspaceKind::Repo)
    );
    if !kind_matches
        || requested.name.is_some()
        || requested.base.is_some()
        || requested.root.is_none()
    {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not match the product session workspace",
        ));
    }
    let root = requested.root.as_ref().ok_or_else(|| {
        ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not include the product session workspace root",
        )
    })?;
    let hinted_workspace = match requested.kind {
        CreateJobWorkspaceKind::Folder => Workspace::open_folder(root),
        CreateJobWorkspaceKind::Repo => Workspace::open_repo(root),
        CreateJobWorkspaceKind::Task => unreachable!("task cannot match a product workspace"),
    }
    .map_err(|_| {
        ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not resolve to the product session workspace",
        )
    })?;
    if hinted_workspace.root != server_workspace.root {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductSessionWorkspaceMismatch.as_str(),
            "the client workspace hint does not match the product session workspace",
        ));
    }
    Ok(())
}

fn workspace_for_create_job(
    state: &ApiState,
    requested: Option<&CreateJobWorkspace>,
) -> Result<(Workspace, AppConfig), ApiError> {
    let Some(requested) = requested else {
        return Ok((state.inner.workspace.clone(), state.inner.config.clone()));
    };

    match requested.kind {
        CreateJobWorkspaceKind::Task => {
            if requested.root.is_some() {
                return Err(ApiError::bad_request(
                    "task workspace uses name/base, not root",
                ));
            }
            let name = requested
                .name
                .as_deref()
                .ok_or_else(|| ApiError::bad_request("task workspace name is required"))?;
            let base = requested
                .base
                .clone()
                .unwrap_or_else(|| state.inner.config.state_dir().join("tasks"));
            let workspace = Workspace::task(&base, name)
                .map_err(|err| ApiError::bad_request(err.to_string()))?;
            rebased_workspace_config(state, workspace)
        }
        CreateJobWorkspaceKind::Folder | CreateJobWorkspaceKind::Repo => {
            if requested.name.is_some() || requested.base.is_some() {
                return Err(ApiError::bad_request(
                    "folder/repo workspace uses root, not name/base",
                ));
            }
            let root = requested
                .root
                .as_ref()
                .ok_or_else(|| ApiError::bad_request("folder/repo workspace root is required"))?;
            let workspace = match requested.kind {
                CreateJobWorkspaceKind::Folder => Workspace::open_folder(root),
                CreateJobWorkspaceKind::Repo => Workspace::open_repo(root),
                CreateJobWorkspaceKind::Task => unreachable!("task handled above"),
            }
            .map_err(|err| ApiError::bad_request(err.to_string()))?;
            rebased_workspace_config(state, workspace)
        }
    }
}

fn rebased_workspace_config(
    state: &ApiState,
    mut workspace: Workspace,
) -> Result<(Workspace, AppConfig), ApiError> {
    let mut config = state.inner.config.clone();
    config.rebase_to_workspace(&workspace.root);
    workspace.state_dir = config.state_dir();
    workspace
        .ensure_state_dir()
        .map_err(|err| ApiError::internal(err.to_string()))?;
    Ok((workspace, config))
}

fn state_store_for_parts(workspace: &Workspace, config: &AppConfig) -> StateStore {
    let sqlite_path = if config.state.sqlite_path.is_absolute() {
        config.state.sqlite_path.clone()
    } else {
        workspace.root.join(&config.state.sqlite_path)
    };
    StateStore::with_index_path(
        &workspace.state_dir,
        sqlite_path,
        config.state.sqlite_busy_timeout_ms,
    )
}

async fn live_job(state: &ApiState, job_id: JobId) -> Option<Arc<JobRecord>> {
    state.inner.jobs.read().await.get(&job_id).cloned()
}

async fn wait_for_job_completion(record: &JobRecord) {
    if record.handle.lock().await.is_none() {
        tracing::warn!(job_id = %record.job_id, "live job has no supervisor handle");
        return;
    }
    let mut completion = record.completion.subscribe();
    while !*completion.borrow_and_update() {
        if completion.changed().await.is_err() {
            tracing::warn!(job_id = %record.job_id, "job completion signal closed unexpectedly");
            return;
        }
    }
}

async fn drain_job_supervisors(state: &ApiState) {
    state.inner.supervisors.close();
    state.inner.supervisors.wait().await;

    let records: Vec<_> = state.inner.jobs.read().await.values().cloned().collect();
    for record in records {
        let handle = record.handle.lock().await.take();
        let Some(handle) = handle else {
            continue;
        };
        if let Err(error) = handle.await {
            tracing::warn!(job_id = %record.job_id, "job supervisor join failed during shutdown: {error}");
        }
    }
}

async fn find_job(state: &ApiState, job_id: JobId) -> Result<Arc<JobRecord>, ApiError> {
    live_job(state, job_id)
        .await
        .ok_or_else(|| ApiError::not_found("job not found"))
}

struct ApiApprovalProvider {
    record: Arc<JobRecord>,
    index: StateIndex,
}

#[async_trait]
impl ToolApprovalProvider for ApiApprovalProvider {
    async fn begin_approval(
        &self,
        request: ToolApprovalRequest,
    ) -> Result<PendingToolApproval, ToolError> {
        let (tx, rx) = oneshot::channel();
        let record = Arc::clone(&self.record);
        let index = self.index.clone();
        let job_id = record.job_id;
        let call_id = request.call_id;
        let registration = tokio::spawn(async move {
            if let Err(err) = index
                .record_pending_approval_async(
                    call_id,
                    record.job_id,
                    record.run_id,
                    request.name.clone(),
                    request.args.to_string(),
                    request.reason.clone(),
                )
                .await
            {
                tracing::warn!(
                    job_id = %record.job_id,
                    call_id = %call_id,
                    "failed to persist pending approval: {err}"
                );
                return Err(format!("failed to persist pending approval: {err}"));
            }

            let mut pending = record.pending_approvals.lock().await;
            if record.cancel_token.is_cancelled() {
                drop(pending);
                if let Err(err) = index
                    .mark_pending_approval_status_async(call_id, "cancelled".to_string())
                    .await
                {
                    tracing::warn!(job_id = %record.job_id, call_id = %call_id, "failed to mark cancelled approval registration: {err}");
                }
                return Err("approval request cancelled during registration".to_string());
            }
            pending.insert(call_id, PendingApproval { request, tx });
            Ok(())
        });

        match registration.await {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(ToolError::ExecutionFailed { reason }),
            Err(err) => {
                tracing::warn!(job_id = %job_id, call_id = %call_id, "approval registration task failed: {err}");
                return Err(ToolError::ExecutionFailed {
                    reason: format!("approval registration task failed for job {job_id}: {err}"),
                });
            }
        }
        Ok(PendingToolApproval::new(async move {
            rx.await.unwrap_or(ApprovalDecision::Reject)
        }))
    }
}

struct ApiInputProvider {
    record: Arc<JobRecord>,
    index: StateIndex,
}

#[async_trait]
impl UserInputProvider for ApiInputProvider {
    async fn begin_input(
        &self,
        input_id: CallId,
        request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        let (tx, rx) = oneshot::channel();
        let prompt = request.prompt.clone();
        let record = Arc::clone(&self.record);
        let index = self.index.clone();
        let job_id = record.job_id;
        let registration = tokio::spawn(async move {
            if let Err(err) = index
                .record_pending_input_async(input_id, record.job_id, record.run_id, prompt)
                .await
            {
                tracing::warn!(
                    job_id = %record.job_id,
                    input_id = %input_id,
                    "failed to persist pending input: {err}"
                );
                return Err(format!("failed to persist pending input: {err}"));
            }

            let mut pending = record.pending_inputs.lock().await;
            if record.cancel_token.is_cancelled() {
                drop(pending);
                if let Err(err) = index
                    .mark_pending_input_status_async(input_id, "cancelled".to_string())
                    .await
                {
                    tracing::warn!(job_id = %record.job_id, input_id = %input_id, "failed to mark cancelled input registration: {err}");
                }
                return Err("input request cancelled during registration".to_string());
            }
            pending.insert(input_id, PendingInput { request, tx });
            Ok(())
        });

        match registration.await {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(ToolError::ExecutionFailed { reason }),
            Err(err) => {
                return Err(ToolError::ExecutionFailed {
                    reason: format!("input registration task failed for job {job_id}: {err}"),
                });
            }
        }
        Ok(PendingUserInput::new(async move {
            rx.await.map_err(|_| ToolError::ExecutionFailed {
                reason: "input request cancelled".to_string(),
            })
        }))
    }
}

async fn job_state_response(record: &JobRecord) -> JobStateResponse {
    let events = record.events.lock().await.clone();
    JobStateResponse {
        job_id: record.job_id,
        run_id: record.run_id,
        resumed_from_run_id: record.resumed_from_run_id,
        status: record.status.lock().await.clone(),
        event_count: events.len(),
        events,
        pending_approvals: pending_approvals_response(record).await,
        pending_inputs: pending_inputs_response(record).await,
    }
}

async fn persisted_job_state_response(
    state: &ApiState,
    job_id: JobId,
) -> Result<JobStateResponse, ApiError> {
    let state_store = state_store_for_api(state);
    let job = state_store
        .index
        .job_record_async(job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("job not found"))?;
    let run_id = job
        .run_id
        .ok_or_else(|| ApiError::not_found("job run not found"))?;
    let events = persisted_events_for_run(&state_store, run_id, 0).await?;
    Ok(JobStateResponse {
        job_id: job.job_id,
        run_id,
        resumed_from_run_id: None,
        status: run_status_from_index(&job.status),
        event_count: events.len(),
        events,
        pending_approvals: Vec::new(),
        pending_inputs: Vec::new(),
    })
}

async fn persisted_job_events(
    state: &ApiState,
    job_id: JobId,
    after: u64,
) -> Result<Vec<JobStreamEvent>, ApiError> {
    let state_store = state_store_for_api(state);
    let job = state_store
        .index
        .job_record_async(job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("job not found"))?;
    let run_id = job
        .run_id
        .ok_or_else(|| ApiError::not_found("job run not found"))?;
    persisted_events_for_run(&state_store, run_id, after).await
}

async fn persisted_or_live_events(
    _state: &ApiState,
    record: &JobRecord,
    after: u64,
) -> Result<(Vec<JobStreamEvent>, RunStatus), ApiError> {
    let state_store = state_store_for_record(record);
    let mut merged = persisted_events_for_run(&state_store, record.run_id, after)
        .await?
        .into_iter()
        .map(|event| (event.seq, event))
        .collect::<BTreeMap<_, _>>();
    // Terminal publication takes these locks in the same order. Holding the
    // event lock through the status snapshot makes the replay/live handoff
    // atomic with respect to a newly published terminal event.
    let events = record.events.lock().await;
    let status = record.status.lock().await.clone();
    for event in events.iter().cloned() {
        if event.seq > after {
            merged.insert(event.seq, event);
        }
    }
    Ok((merged.into_values().collect(), status))
}

async fn persisted_events_for_run(
    state_store: &StateStore,
    run_id: RunId,
    after: u64,
) -> Result<Vec<JobStreamEvent>, ApiError> {
    let records = state_store
        .index
        .event_records_async(run_id)
        .await
        .map_err(ApiError::internal)?;
    records
        .into_iter()
        .filter(|record| record.seq > after)
        .map(|record| {
            let event = serde_json::from_str::<StreamEvent>(&record.event_json)
                .map_err(ApiError::internal)?;
            Ok(JobStreamEvent {
                seq: record.seq,
                event,
            })
        })
        .collect()
}

async fn append_job_event(record: &JobRecord, event: StreamEvent) -> JobStreamEvent {
    let stored = {
        let mut events = record.events.lock().await;
        if let StreamEvent::RunCompleted { reason, .. } = &event {
            *record.status.lock().await = status_for_reason(reason);
        }
        let seq = events.last().map(|event| event.seq + 1).unwrap_or(1);
        let stored = JobStreamEvent { seq, event };
        events.push(stored.clone());
        stored
    };
    let _ = record.tx.send(stored.clone());
    stored
}

async fn pending_approvals_response(record: &JobRecord) -> Vec<PendingApprovalResponse> {
    record
        .pending_approvals
        .lock()
        .await
        .values()
        .map(|pending| PendingApprovalResponse {
            call_id: pending.request.call_id,
            name: pending.request.name.clone(),
            args: pending.request.args.clone(),
            reason: pending.request.reason.clone(),
        })
        .collect()
}

async fn pending_inputs_response(record: &JobRecord) -> Vec<PendingInputResponse> {
    record
        .pending_inputs
        .lock()
        .await
        .iter()
        .map(|(input_id, pending)| PendingInputResponse {
            input_id: *input_id,
            prompt: pending.request.prompt.clone(),
        })
        .collect()
}

async fn reject_pending_approvals(record: &JobRecord, index: &StateIndex) {
    let pending = std::mem::take(&mut *record.pending_approvals.lock().await);
    for (call_id, approval) in pending {
        if let Err(err) = index
            .mark_pending_approval_status_async(call_id, "cancelled".to_string())
            .await
        {
            tracing::warn!(job_id = %record.job_id, call_id = %call_id, "failed to mark pending approval cancelled: {err}");
        }
        let _ = approval.tx.send(ApprovalDecision::Reject);
    }
}

async fn reject_pending_inputs(record: &JobRecord, index: &StateIndex) {
    let pending = std::mem::take(&mut *record.pending_inputs.lock().await);
    for (input_id, _) in pending {
        if let Err(err) = index
            .mark_pending_input_status_async(input_id, "cancelled".to_string())
            .await
        {
            tracing::warn!(job_id = %record.job_id, input_id = %input_id, "failed to mark pending input cancelled: {err}");
        }
    }
}

fn approval_status(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Approve => "approved",
        ApprovalDecision::Reject => "rejected",
    }
}

fn sse_event(event: JobStreamEvent) -> Result<Event, serde_json::Error> {
    let name = event.event.event_name();
    Ok(Event::default()
        .id(event.seq.to_string())
        .event(name)
        .data(serde_json::to_string(&event.event)?))
}

fn parse_last_event_id(headers: &HeaderMap) -> Result<Option<u64>, ApiError> {
    let Some(value) = headers.get("last-event-id") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request("Last-Event-ID must be a valid integer"))?;
    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| ApiError::bad_request("Last-Event-ID must be a valid integer"))
}

fn is_terminal(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Done | RunStatus::Error | RunStatus::Cancelled | RunStatus::Interrupted
    )
}

fn status_for_reason(reason: &TerminationReason) -> RunStatus {
    match reason {
        TerminationReason::Final
        | TerminationReason::StepLimit
        | TerminationReason::TokenLimit
        | TerminationReason::TimeLimit => RunStatus::Done,
        TerminationReason::Error => RunStatus::Error,
        TerminationReason::Cancelled => RunStatus::Cancelled,
    }
}

fn run_status_from_index(status: &str) -> RunStatus {
    match status {
        "init" => RunStatus::Init,
        "running" => RunStatus::Running,
        "done" => RunStatus::Done,
        "error" => RunStatus::Error,
        "cancelled" => RunStatus::Cancelled,
        "interrupted" => RunStatus::Interrupted,
        _ => RunStatus::Error,
    }
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message: message.into(),
        }
    }

    pub(crate) fn bad_request_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "conflict",
            message: message.into(),
        }
    }

    fn conflict_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code: "bad_gateway",
            message: message.into(),
        }
    }

    pub(crate) fn not_implemented(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_IMPLEMENTED,
            code,
            message: message.into(),
        }
    }

    fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error",
            message: err.to_string(),
        }
    }
}

impl From<ProductStoreError> for ApiError {
    fn from(error: ProductStoreError) -> Self {
        let status = match error.code {
            ProductErrorCode::ProductNotFound => StatusCode::NOT_FOUND,
            ProductErrorCode::ProductInvalidInput => StatusCode::BAD_REQUEST,
            ProductErrorCode::ProductStoreUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ProductErrorCode::ProductStorageFailure => StatusCode::INTERNAL_SERVER_ERROR,
            ProductErrorCode::ProductSessionActive
            | ProductErrorCode::ProductSessionWorkspaceMismatch
            | ProductErrorCode::ProductSessionResumeConflict
            | ProductErrorCode::ProductSessionRuntimeStateMissing
            | ProductErrorCode::ProductSessionRuntimeStateCorrupt
            | ProductErrorCode::ProductBindingCorrupt
            | ProductErrorCode::MigrationIdempotencyConflict => StatusCode::CONFLICT,
        };
        let message = if error.code == ProductErrorCode::ProductStorageFailure {
            tracing::warn!("product store operation failed: {error}");
            "product store operation failed".to_string()
        } else {
            error.message
        };
        Self {
            status,
            code: error.code.as_str(),
            message,
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ApiErrorResponse {
                code: self.code.to_string(),
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
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
            resumed_from_run_id: None,
            resume_state: None,
            status: Mutex::new(RunStatus::Running),
            events: Mutex::new(Vec::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            pending_inputs: Mutex::new(HashMap::new()),
            tx,
            handle: Mutex::new(None),
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
        let mut stream = replay_and_live_job_event_stream(
            vec![terminal.clone()],
            RunStatus::Running,
            receiver,
            0,
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), stream.next())
                .await
                .is_err(),
            "persisted terminal must remain behind the live finalization barrier"
        );
        sender.send(terminal.clone()).unwrap();

        assert_eq!(stream.next().await, Some(terminal));
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
        let mut stream =
            replay_and_live_job_event_stream(Vec::new(), RunStatus::Running, receiver, 1);
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

    async fn test_job_record(
        temp_dir: &tempfile::TempDir,
    ) -> (ApiState, Arc<JobRecord>, StateIndex) {
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
            resumed_from_run_id: None,
            resume_state: None,
            status: Mutex::new(RunStatus::Running),
            events: Mutex::new(Vec::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            pending_inputs: Mutex::new(HashMap::new()),
            tx,
            handle: Mutex::new(None),
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

        let (events_after_terminal, status) =
            persisted_or_live_events(&state, &record, terminal.seq)
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
            resumed_from_run_id: Some(first.run_id),
            resume_state: None,
            status: Mutex::new(RunStatus::Running),
            events: Mutex::new(Vec::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            pending_inputs: Mutex::new(HashMap::new()),
            tx,
            handle: Mutex::new(None),
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
        assert!(state.inner.supervisors.is_closed());
        assert!(state.inner.supervisors.is_empty());
        let first_handle = first.handle.lock().await.take().unwrap();
        first_handle.await.unwrap();
        assert!(second.handle.lock().await.is_none());
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
            Json(SubmitInputRequest {
                answer: "main".to_string(),
            }),
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
            Json(SubmitApprovalRequest {
                decision: ApprovalDecision::Approve,
            }),
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
}
