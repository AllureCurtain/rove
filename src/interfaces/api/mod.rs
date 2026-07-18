use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::Query;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::{Json, Router};
use futures::{Stream, StreamExt};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock, broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;
use utoipa::OpenApi;
use utoipa_axum::{router::OpenApiRouter, routes};
use utoipa_swagger_ui::SwaggerUi;

use crate::config::{AppConfig, AppConfigOverrides};
use crate::core::engine::Engine;
use crate::core::events::StreamEvent;
use crate::core::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, PendingToolApproval, PendingUserInput, RunId,
    RunStatus, SessionId, TaskState, TerminationReason, ToolApprovalProvider, ToolApprovalRequest,
    UserInputProvider, UserInputRequest,
};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::interfaces::runtime::{EngineAssemblyOptions, build_interface_engine};
use crate::models::factory::build_model_client_with_health;
use crate::models::fake::FakeModelClient;
use crate::models::health::{HealthConfig, ModelHealthStore};
use crate::models::traits::ModelClient;
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::index::StateIndex;
use crate::state::resume::resolve_resume_state;
use crate::state::store::StateStore;

mod benchmark;
mod debug;
mod docs;
mod provider;
mod security;
mod types;

use benchmark::BenchState;
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
    shutdown_token: CancellationToken,
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
    cancel_token: CancellationToken,
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
        .routes(routes!(test_provider))
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
    serve_listener(listener, router(state), shutdown).await
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
        let model_health = Arc::new(ModelHealthStore::new(HealthConfig {
            failure_threshold: config.routing.failure_threshold,
            open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
        }));
        Self {
            inner: Arc::new(ApiStateInner {
                workspace,
                config,
                shutdown_token,
                jobs: RwLock::new(HashMap::new()),
                model_health,
                rate_limit: tokio::sync::Mutex::new(RateLimitState::default()),
                bench_runs: Arc::new(BenchState::default()),
            }),
        }
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
        (status = 400, description = "Invalid job request", body = serde_json::Value, content_type = "application/json"),
        (status = 409, description = "Requested resume target is still active", body = serde_json::Value, content_type = "application/json"),
        (status = 500, description = "Internal runtime error", body = serde_json::Value, content_type = "application/json")
    )
)]
async fn create_job(
    State(state): State<ApiState>,
    Json(req): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    if req.message.trim().is_empty() {
        return Err(ApiError::bad_request("message must not be empty"));
    }

    let (workspace, config) = workspace_and_config_for_create_job(&state, &req)?;
    let state_store = state_store_for_parts(&workspace, &config);
    let resume_state = resolve_resume_state(&state_store, req.resume.as_deref())
        .await
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let session_id = resume_state
        .as_ref()
        .map(|task_state| task_state.session_id)
        .unwrap_or_else(SessionId::new);
    let job_id = resume_state
        .as_ref()
        .map(|task_state| task_state.job_id)
        .unwrap_or_else(JobId::new);
    if resume_state.is_some()
        && let Some(record) = live_job(&state, job_id).await
    {
        let status = record.status.lock().await.clone();
        if !is_terminal(&status) {
            return Err(ApiError::conflict(
                "cannot resume a job while its previous run is still active",
            ));
        }
    }
    let run_id = RunId::new();
    let resumed_from_run_id = resume_state.as_ref().map(|task_state| task_state.run_id);
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let record = Arc::new(JobRecord {
        session_id,
        job_id,
        run_id,
        workspace,
        config,
        message: req.message.clone(),
        resumed_from_run_id,
        resume_state,
        status: Mutex::new(RunStatus::Init),
        events: Mutex::new(Vec::new()),
        pending_approvals: Mutex::new(HashMap::new()),
        pending_inputs: Mutex::new(HashMap::new()),
        tx,
        handle: Mutex::new(None),
        cancel_token: state.inner.shutdown_token.child_token(),
    });

    state
        .inner
        .jobs
        .write()
        .await
        .insert(job_id, record.clone());

    let state_for_task = state.clone();
    let record_for_task = record.clone();
    let handle = tokio::spawn(async move {
        run_job(state_for_task, record_for_task, req).await;
    });
    *record.handle.lock().await = Some(handle);

    Ok(Json(CreateJobResponse {
        job_id,
        run_id,
        resumed_from_run_id,
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
    let existing = persisted_or_live_events(&state, &record, after).await?;
    let status = record.status.lock().await.clone();
    let replay_events: Vec<_> = existing
        .into_iter()
        .filter(|event| event.seq > after)
        .collect();
    let replay_high_water = replay_events.last().map(|event| event.seq).unwrap_or(after);
    let replay = futures::stream::iter(replay_events);
    let live = if is_terminal(&status) {
        futures::stream::empty().boxed()
    } else {
        BroadcastStream::new(live_rx)
            .filter_map(move |event| {
                futures::future::ready(match event {
                    Ok(event) if event.seq > replay_high_water => Some(event),
                    _ => None,
                })
            })
            .boxed()
    };
    let stream = replay
        .chain(live)
        .filter_map(|event| futures::future::ready(sse_event(event).ok()))
        .map(Ok)
        .boxed();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
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
        return Ok(Json(job_state_response(&record).await));
    }

    record.cancel_token.cancel();
    let state_store = state_store_for_record(&record);
    reject_pending_approvals(&record, &state_store.index).await;
    reject_pending_inputs(&record, &state_store.index).await;

    if let Some(handle) = record.handle.lock().await.take() {
        let _ = handle.await;
    }

    let status = record.status.lock().await.clone();
    if !is_terminal(&status) {
        finalize_cancelled_job(&state, &record).await;
    }

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
) -> Result<Json<crate::state::report::RunReport>, ApiError> {
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

async fn run_job(state: ApiState, record: Arc<JobRecord>, req: CreateJobRequest) {
    *record.status.lock().await = RunStatus::Running;
    let result = run_job_inner(&state, &record, &req).await;
    if let Err(err) = result {
        tracing::warn!(job_id = %record.job_id, "job failed: {err}");
        *record.status.lock().await = RunStatus::Error;
    }
}

async fn run_job_inner(
    state: &ApiState,
    record: &Arc<JobRecord>,
    req: &CreateJobRequest,
) -> anyhow::Result<()> {
    let engine = build_engine(state, &record.message, req, record.clone()).await?;
    let state_store = state_store_for_record(record);
    let run = state_store.start_run(record.session_id, record.job_id, record.run_id)?;
    let mut recorder = RunArtifactRecorder::new(
        record.session_id,
        record.job_id,
        record.run_id,
        record.message.clone(),
        record.resume_state.as_ref(),
        Some(engine.runtime_identity()),
    );
    let model_id = engine.model_id().to_string();
    let workspace = engine.workspace().clone();
    let request = run.request(record.message.clone(), record.resume_state.clone());
    let mut stream = std::pin::pin!(engine.run_with_cancel(
        request,
        Some(run.trace_writer),
        record.cancel_token.clone(),
    ));
    let mut completed = false;
    let mut terminal_status = RunStatus::Error;
    let mut terminal_event = None;
    while let Some(event) = stream.next().await {
        recorder.record_event(&event, &state_store).await;
        if let StreamEvent::RunCompleted { reason, .. } = &event {
            completed = true;
            terminal_status = status_for_reason(reason);
            terminal_event = Some(event);
            continue;
        }
        append_job_event(record, event).await;
    }
    recorder
        .finalize(&state_store, &workspace, &model_id, &run.run_dir)
        .await;
    if matches!(terminal_status, RunStatus::Cancelled | RunStatus::Error) {
        reject_pending_approvals(record, &state_store.index).await;
        reject_pending_inputs(record, &state_store.index).await;
    }
    if let Some(event) = terminal_event {
        append_job_event(record, event).await;
    } else if !completed {
        *record.status.lock().await = RunStatus::Error;
    }
    Ok(())
}

async fn finalize_cancelled_job(_state: &ApiState, record: &Arc<JobRecord>) {
    let cancel_event = StreamEvent::RunCompleted {
        reason: TerminationReason::Cancelled,
        output: None,
    };

    let already_completed = record
        .events
        .lock()
        .await
        .iter()
        .any(|event| matches!(event.event, StreamEvent::RunCompleted { .. }));
    let mut events_for_recorder: Vec<_> = record
        .events
        .lock()
        .await
        .iter()
        .map(|event| event.event.clone())
        .collect();
    if !already_completed {
        events_for_recorder.push(cancel_event.clone());
    }

    let state_store = state_store_for_record(record);
    if let Ok(trace_writer) = state_store.run_store.create_trace(&record.run_id) {
        let _ = trace_writer.append(&cancel_event);
    }

    let mut recorder = RunArtifactRecorder::new(
        record.session_id,
        record.job_id,
        record.run_id,
        record.message.clone(),
        None,
        None,
    );
    for event in &events_for_recorder {
        recorder.record_event(event, &state_store).await;
    }
    let run_dir = state_store.run_store.run_dir(&record.run_id);
    recorder
        .finalize(
            &state_store,
            &record.workspace,
            record.config.provider.model.as_str(),
            &run_dir,
        )
        .await;
    if !already_completed {
        append_job_event(record, cancel_event).await;
    } else {
        *record.status.lock().await = RunStatus::Cancelled;
    }
}

async fn build_engine(
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

    build_interface_engine(EngineAssemblyOptions {
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

fn workspace_for_create_job(
    state: &ApiState,
    requested: Option<&CreateJobWorkspace>,
) -> Result<(Workspace, AppConfig), ApiError> {
    let Some(requested) = requested else {
        return Ok((state.inner.workspace.clone(), state.inner.config.clone()));
    };

    match requested.kind {
        CreateJobWorkspaceKind::Task => {
            let name = requested
                .name
                .as_deref()
                .ok_or_else(|| ApiError::bad_request("task workspace name is required"))?;
            let base = requested
                .base
                .clone()
                .unwrap_or_else(|| state.inner.config.state_dir().join("tasks"));
            let mut workspace = Workspace::task(&base, name)
                .map_err(|err| ApiError::bad_request(err.to_string()))?;
            let mut config = state.inner.config.clone();
            config.rebase_to_workspace(&workspace.root);
            workspace.state_dir = config.state_dir();
            workspace
                .ensure_state_dir()
                .map_err(|err| ApiError::internal(err.to_string()))?;
            Ok((workspace, config))
        }
    }
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
) -> Result<Vec<JobStreamEvent>, ApiError> {
    let state_store = state_store_for_record(record);
    let mut merged = persisted_events_for_run(&state_store, record.run_id, after)
        .await?
        .into_iter()
        .map(|event| (event.seq, event))
        .collect::<BTreeMap<_, _>>();
    for event in record.events.lock().await.iter().cloned() {
        if event.seq > after {
            merged.insert(event.seq, event);
        }
    }
    Ok(merged.into_values().collect())
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
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }

    fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": self.message,
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

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
            name: "fs_write".to_string(),
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
                    name: "fs_write".to_string(),
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
            name: "fs_write".to_string(),
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
