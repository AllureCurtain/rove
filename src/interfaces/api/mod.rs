use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, broadcast, oneshot};
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::core::context::ContextManager;
use crate::core::engine::{Engine, EngineConfig};
use crate::core::events::StreamEvent;
use crate::core::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, RunId, RunStatus, SessionId,
    TerminationReason, ToolApprovalProvider, ToolApprovalRequest, UserInputProvider,
    UserInputRequest,
};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::models::factory::build_openai_model_client;
use crate::models::fake::FakeModelClient;
use crate::models::traits::ModelClient;
use crate::state::artifacts::RunArtifactRecorder;
use crate::state::store::StateStore;
use crate::tools::echo::EchoTool;
use crate::tools::fs::{FsReadTool, FsWriteTool};
use crate::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use crate::tools::rag::RagRetrieveTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::request_input::RequestInputTool;
use crate::tools::shell::ShellTool;

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
}

struct JobRecord {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    message: String,
    status: Mutex<RunStatus>,
    events: Mutex<Vec<StreamEvent>>,
    pending_approvals: Mutex<HashMap<CallId, PendingApproval>>,
    pending_inputs: Mutex<HashMap<CallId, PendingInput>>,
    tx: broadcast::Sender<StreamEvent>,
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
}

#[derive(Debug, Deserialize)]
pub struct SubmitApprovalRequest {
    pub decision: ApprovalDecision,
}

#[derive(Debug, Deserialize)]
pub struct SubmitInputRequest {
    pub answer: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: JobId,
    pub run_id: RunId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobStateResponse {
    pub job_id: JobId,
    pub run_id: RunId,
    pub status: RunStatus,
    pub event_count: usize,
    pub pending_approvals: Vec<PendingApprovalResponse>,
    pub pending_inputs: Vec<PendingInputResponse>,
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/jobs", post(create_job))
        .route("/jobs/{job_id}/events", get(job_events))
        .route("/jobs/{job_id}/state", get(job_state))
        .route("/jobs/{job_id}/cancel", post(cancel_job))
        .route("/jobs/{job_id}/approvals/{call_id}", post(submit_approval))
        .route("/jobs/{job_id}/inputs/{input_id}", post(submit_input))
        .with_state(state)
}

pub async fn serve(addr: SocketAddr, cwd: PathBuf) -> anyhow::Result<()> {
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
    addr: SocketAddr,
    cwd: PathBuf,
    shutdown: CancellationToken,
) -> anyhow::Result<()> {
    let config = AppConfig::from_env()?;
    let workspace = Workspace::detect(&cwd)?;
    workspace.ensure_state_dir()?;
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
        config: AppConfig,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self {
            inner: Arc::new(ApiStateInner {
                workspace,
                config,
                shutdown_token,
                jobs: RwLock::new(HashMap::new()),
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

    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let record = Arc::new(JobRecord {
        session_id,
        job_id,
        run_id,
        message: req.message.clone(),
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

    Ok(Json(CreateJobResponse { job_id, run_id }))
}

async fn job_events(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let record = find_job(&state, job_id).await?;
    let existing = record.events.lock().await.clone();
    let status = record.status.lock().await.clone();
    let replay = futures::stream::iter(existing);
    let live = if is_terminal(&status) {
        futures::stream::empty().boxed()
    } else {
        BroadcastStream::new(record.tx.subscribe())
            .filter_map(|event| futures::future::ready(event.ok()))
            .boxed()
    };
    let stream = replay
        .chain(live)
        .filter_map(|event| futures::future::ready(sse_event(event).ok()));

    Ok(Sse::new(stream.map(Ok)).keep_alive(KeepAlive::default()))
}

async fn job_state(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let record = find_job(&state, job_id).await?;
    Ok(Json(job_state_response(&record).await))
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
    reject_pending_approvals(&record).await;
    reject_pending_inputs(&record).await;

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
    let _ = pending.tx.send(req.answer);
    Ok(Json(job_state_response(&record).await))
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
    let engine = build_engine(state, &record.message, req, record.clone())?;
    let state_store = StateStore::new(&state.inner.workspace.state_dir);
    let run = state_store.start_run(record.session_id, record.job_id, record.run_id)?;
    let mut recorder = RunArtifactRecorder::new(
        record.session_id,
        record.job_id,
        record.run_id,
        record.message.clone(),
        None,
    );
    let model_id = engine.model_id().to_string();
    let workspace = engine.workspace().clone();
    let request = run.request(record.message.clone(), None);
    let mut stream = std::pin::pin!(engine.run_with_cancel(
        request,
        Some(run.trace_writer),
        record.cancel_token.clone(),
    ));
    let mut completed = false;
    let mut terminal_status = RunStatus::Error;
    while let Some(event) = stream.next().await {
        recorder.record_event(&event, &state_store).await;
        if let StreamEvent::RunCompleted { reason, .. } = &event {
            completed = true;
            terminal_status = status_for_reason(reason);
        }
        record.events.lock().await.push(event.clone());
        let _ = record.tx.send(event);
    }
    recorder
        .finalize(&state_store, &workspace, &model_id, &run.run_dir)
        .await;
    if matches!(terminal_status, RunStatus::Cancelled | RunStatus::Error) {
        reject_pending_approvals(record).await;
        reject_pending_inputs(record).await;
    }
    *record.status.lock().await = if completed {
        terminal_status
    } else {
        RunStatus::Error
    };
    Ok(())
}

async fn finalize_cancelled_job(state: &ApiState, record: &Arc<JobRecord>) {
    let cancel_event = StreamEvent::RunCompleted {
        reason: TerminationReason::Cancelled,
        output: None,
    };

    let events_for_recorder = {
        let mut events = record.events.lock().await;
        if !events
            .iter()
            .any(|event| matches!(event, StreamEvent::RunCompleted { .. }))
        {
            events.push(cancel_event.clone());
            let _ = record.tx.send(cancel_event.clone());
        }
        events.clone()
    };

    let state_store = StateStore::new(&state.inner.workspace.state_dir);
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
            &state.inner.workspace,
            state.inner.config.model.as_str(),
            &run_dir,
        )
        .await;
    *record.status.lock().await = RunStatus::Cancelled;
}

fn build_engine(
    state: &ApiState,
    message: &str,
    req: &CreateJobRequest,
    record: Arc<JobRecord>,
) -> anyhow::Result<Engine> {
    let config = &state.inner.config;
    let model_id = req.model.clone().unwrap_or_else(|| config.model.clone());
    let model: Box<dyn ModelClient> = match model_id.as_str() {
        "fake" => Box::new(FakeModelClient::new(format!("fake response: {message}"))),
        "fake-raw" => Box::new(FakeModelClient::new(message.to_string())),
        _ => build_openai_model_client(config, model_id),
    };

    let workspace = state.inner.workspace.clone();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(ReadMemoryTopicTool::new(workspace.root.clone())));
    registry.register(Box::new(SaveMemoryTool::new(workspace.root.clone())));
    registry.register(Box::new(UpdateMemoryIndexTool::new(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
    registry.register(Box::new(RequestInputTool));
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));

    let approval_policy = req.approval.unwrap_or(ApprovalPolicy::Ask);
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new(config.load_system_prompt()),
        EngineConfig {
            max_steps: req.max_steps.unwrap_or(config.max_steps),
            plan_enabled: true,
        },
        workspace,
        approval_policy,
    )
    .with_input_provider(Arc::new(ApiInputProvider {
        record: record.clone(),
    }));
    if approval_policy == ApprovalPolicy::Ask {
        Ok(engine.with_approval_provider(Arc::new(ApiApprovalProvider { record })))
    } else {
        Ok(engine)
    }
}

async fn find_job(state: &ApiState, job_id: JobId) -> Result<Arc<JobRecord>, ApiError> {
    state
        .inner
        .jobs
        .read()
        .await
        .get(&job_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("job not found"))
}

struct ApiApprovalProvider {
    record: Arc<JobRecord>,
}

#[async_trait]
impl ToolApprovalProvider for ApiApprovalProvider {
    async fn decide(&self, request: ToolApprovalRequest) -> ApprovalDecision {
        let (tx, rx) = oneshot::channel();
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
}

#[async_trait]
impl UserInputProvider for ApiInputProvider {
    async fn request_input(&self, request: UserInputRequest) -> Result<String, ToolError> {
        let input_id = CallId::new();
        let (tx, rx) = oneshot::channel();
        self.record
            .pending_inputs
            .lock()
            .await
            .insert(input_id, PendingInput { request, tx });
        rx.await.map_err(|_| ToolError::ExecutionFailed {
            reason: "input request cancelled".to_string(),
        })
    }
}

async fn job_state_response(record: &JobRecord) -> JobStateResponse {
    JobStateResponse {
        job_id: record.job_id,
        run_id: record.run_id,
        status: record.status.lock().await.clone(),
        event_count: record.events.lock().await.len(),
        pending_approvals: pending_approvals_response(record).await,
        pending_inputs: pending_inputs_response(record).await,
    }
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

async fn reject_pending_approvals(record: &JobRecord) {
    let pending = std::mem::take(&mut *record.pending_approvals.lock().await);
    for (_, approval) in pending {
        let _ = approval.tx.send(ApprovalDecision::Reject);
    }
}

async fn reject_pending_inputs(record: &JobRecord) {
    let _ = std::mem::take(&mut *record.pending_inputs.lock().await);
}

fn sse_event(event: StreamEvent) -> Result<Event, serde_json::Error> {
    let name = event.event_name();
    Ok(Event::default()
        .event(name)
        .data(serde_json::to_string(&event)?))
}

fn is_terminal(status: &RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Done | RunStatus::Error | RunStatus::Cancelled
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
