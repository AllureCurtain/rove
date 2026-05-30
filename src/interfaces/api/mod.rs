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
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use crate::config::{AppConfig, AppConfigOverrides};
use crate::core::context::{ContextBudget, ContextManager};
use crate::core::engine::{Engine, EngineConfig};
use crate::core::events::StreamEvent;
use crate::core::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, RunId, RunStatus, SessionId, TaskState,
    TerminationReason, ToolApprovalProvider, ToolApprovalRequest, UserInputProvider,
    UserInputRequest,
};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::models::factory::build_model_client_with_health;
use crate::models::fake::FakeModelClient;
use crate::models::health::{HealthConfig, ModelHealthStore};
use crate::models::traits::ModelClient;
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::index::StateIndex;
use crate::state::resume::resolve_resume_state;
use crate::state::store::StateStore;
use crate::tools::runtime_tool_registry;

mod security;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApprovalResponse {
    pub call_id: CallId,
    pub name: String,
    pub args: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingInputResponse {
    pub input_id: CallId,
    pub prompt: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub message: String,
    pub model: Option<String>,
    pub max_steps: Option<u32>,
    pub approval: Option<ApprovalPolicy>,
    pub resume: Option<String>,
    pub workspace: Option<CreateJobWorkspace>,
}

#[derive(Debug, Deserialize)]
pub struct CreateJobWorkspace {
    pub kind: CreateJobWorkspaceKind,
    pub name: Option<String>,
    pub base: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateJobWorkspaceKind {
    Task,
}

#[derive(Debug, Deserialize)]
pub struct SubmitApprovalRequest {
    pub decision: ApprovalDecision,
}

#[derive(Debug, Deserialize)]
pub struct SubmitInputRequest {
    pub answer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStreamEvent {
    pub seq: u64,
    pub event: StreamEvent,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: JobId,
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from_run_id: Option<RunId>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobStateResponse {
    pub job_id: JobId,
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from_run_id: Option<RunId>,
    pub status: RunStatus,
    pub event_count: usize,
    pub events: Vec<JobStreamEvent>,
    pub pending_approvals: Vec<PendingApprovalResponse>,
    pub pending_inputs: Vec<PendingInputResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListRunsResponse {
    pub runs: Vec<RunSummaryResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunSummaryResponse {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub job_id: JobId,
    pub status: RunStatus,
    pub last_event_seq: u64,
    pub has_report: bool,
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
    Router::new()
        .route("/jobs", post(create_job))
        .route("/jobs/{job_id}/events", get(job_events))
        .route("/jobs/{job_id}/state", get(job_state))
        .route("/jobs/{job_id}/cancel", post(cancel_job))
        .route("/jobs/{job_id}/approvals/{call_id}", post(submit_approval))
        .route("/jobs/{job_id}/inputs/{input_id}", post(submit_input))
        .route("/runs", get(list_runs))
        .route("/runs/{run_id}/report", get(run_report))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(
            state,
            security::api_security,
        ))
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
            }),
        }
    }
}

async fn create_job(
    State(state): State<ApiState>,
    Json(req): Json<CreateJobRequest>,
) -> Result<Json<CreateJobResponse>, ApiError> {
    if req.message.trim().is_empty() {
        return Err(ApiError::bad_request("message must not be empty"));
    }

    let (workspace, config) = workspace_for_create_job(&state, req.workspace.as_ref())?;
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

async fn job_state(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Json<JobStateResponse>, ApiError> {
    if let Some(record) = live_job(&state, job_id).await {
        return Ok(Json(job_state_response(&record).await));
    }
    Ok(Json(persisted_job_state_response(&state, job_id).await?))
}

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
    if let Err(err) = state_store_for_record(&record)
        .index
        .mark_pending_approval_status_async(call_id, approval_status(req.decision).to_string())
        .await
    {
        tracing::warn!(job_id = %job_id, call_id = %call_id, "failed to mark pending approval resolved: {err}");
    }
    let _ = pending.tx.send(req.decision);
    Ok(Json(job_state_response(&record).await))
}

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
    if let Err(err) = state_store_for_record(&record)
        .index
        .mark_pending_input_status_async(input_id, "answered".to_string())
        .await
    {
        tracing::warn!(job_id = %job_id, input_id = %input_id, "failed to mark pending input answered: {err}");
    }
    let _ = pending.tx.send(req.answer);
    Ok(Json(job_state_response(&record).await))
}

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
    let memory_paths = config.memory_paths();
    let state_store = state_store_for_record(&record);
    let registry = runtime_tool_registry(
        &workspace,
        config.shell_policy(),
        config.resolve_path(&config.tool.mcp_config_path),
    )
    .await?;

    let approval_policy = req.approval.unwrap_or(ApprovalPolicy::Ask);
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::with_token_budget(
            config.load_system_prompt(),
            ContextBudget {
                soft_limit_tokens: config.runtime.context_soft_limit_tokens,
                hard_limit_tokens: config.runtime.context_hard_limit_tokens,
                reserved_tokens: config.runtime.context_reserved_tokens,
            },
        ),
        EngineConfig {
            max_steps: req.max_steps.unwrap_or(config.runtime.max_steps),
            plan_enabled: true,
        },
        workspace,
        approval_policy,
    )
    .with_planner_prompt(config.load_planner_prompt())
    .with_memory_paths(memory_paths)
    .with_model_compaction(
        config.runtime.model_compaction_enabled,
        config.runtime.compaction_failure_threshold,
    )
    .with_input_provider(Arc::new(ApiInputProvider {
        record: record.clone(),
        index: state_store.index.clone(),
    }));
    if approval_policy == ApprovalPolicy::Ask {
        Ok(engine.with_approval_provider(Arc::new(ApiApprovalProvider {
            record,
            index: state_store.index.clone(),
        })))
    } else {
        Ok(engine)
    }
}

fn state_store_for_api(state: &ApiState) -> StateStore {
    state_store_for_parts(&state.inner.workspace, &state.inner.config)
}

fn state_store_for_record(record: &JobRecord) -> StateStore {
    state_store_for_parts(&record.workspace, &record.config)
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
    async fn decide(&self, request: ToolApprovalRequest) -> ApprovalDecision {
        let (tx, rx) = oneshot::channel();
        if let Err(err) = self
            .index
            .record_pending_approval_async(
                request.call_id,
                self.record.job_id,
                self.record.run_id,
                request.name.clone(),
                request.args.to_string(),
                request.reason.clone(),
            )
            .await
        {
            tracing::warn!(
                job_id = %self.record.job_id,
                call_id = %request.call_id,
                "failed to persist pending approval: {err}"
            );
        }
        self.record
            .pending_approvals
            .lock()
            .await
            .insert(request.call_id, PendingApproval { request, tx });
        rx.await.unwrap_or(ApprovalDecision::Reject)
    }
}

struct ApiInputProvider {
    record: Arc<JobRecord>,
    index: StateIndex,
}

#[async_trait]
impl UserInputProvider for ApiInputProvider {
    async fn request_input(&self, request: UserInputRequest) -> Result<String, ToolError> {
        let input_id = CallId::new();
        let (tx, rx) = oneshot::channel();
        let prompt = request.prompt.clone();
        if let Err(err) = self
            .index
            .record_pending_input_async(
                input_id,
                self.record.job_id,
                self.record.run_id,
                prompt.clone(),
            )
            .await
        {
            tracing::warn!(
                job_id = %self.record.job_id,
                input_id = %input_id,
                "failed to persist pending input: {err}"
            );
        }
        self.record
            .pending_inputs
            .lock()
            .await
            .insert(input_id, PendingInput { request, tx });
        append_job_event(&self.record, StreamEvent::InputNeeded { input_id, prompt }).await;
        rx.await.map_err(|_| ToolError::ExecutionFailed {
            reason: "input request cancelled".to_string(),
        })
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
}
