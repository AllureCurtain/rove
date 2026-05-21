use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_stream::wrappers::BroadcastStream;

use crate::config::AppConfig;
use crate::core::context::ContextManager;
use crate::core::engine::{Engine, EngineConfig};
use crate::core::events::StreamEvent;
use crate::core::types::{ApprovalPolicy, JobId, RunId, RunRequest, RunStatus, SessionId};
use crate::core::workspace::Workspace;
use crate::models::fake::FakeModelClient;
use crate::models::openai::OpenAiClient;
use crate::models::traits::ModelClient;
use crate::state::store::StateStore;
use crate::tools::echo::EchoTool;
use crate::tools::fs::{FsReadTool, FsWriteTool};
#[cfg(feature = "rag")]
use crate::tools::rag::RagRetrieveTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::shell::ShellTool;

const EVENT_BUFFER: usize = 256;

#[derive(Clone)]
pub struct ApiState {
    inner: Arc<ApiStateInner>,
}

struct ApiStateInner {
    workspace: Workspace,
    config: AppConfig,
    jobs: RwLock<HashMap<JobId, Arc<JobRecord>>>,
}

struct JobRecord {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    message: String,
    status: Mutex<RunStatus>,
    events: Mutex<Vec<StreamEvent>>,
    tx: broadcast::Sender<StreamEvent>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub message: String,
    pub model: Option<String>,
    pub max_steps: Option<u32>,
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
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/jobs", post(create_job))
        .route("/jobs/{job_id}/events", get(job_events))
        .route("/jobs/{job_id}/state", get(job_state))
        .route("/jobs/{job_id}/cancel", post(cancel_job))
        .with_state(state)
}

pub async fn serve(addr: SocketAddr, cwd: PathBuf) -> anyhow::Result<()> {
    let config = AppConfig::from_env()?;
    let workspace = Workspace::detect(&cwd)?;
    workspace.ensure_state_dir()?;
    let state = ApiState::new(workspace, config);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}

impl ApiState {
    pub fn new(workspace: Workspace, config: AppConfig) -> Self {
        Self {
            inner: Arc::new(ApiStateInner {
                workspace,
                config,
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
        tx,
        handle: Mutex::new(None),
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
    Ok(Json(JobStateResponse {
        job_id: record.job_id,
        run_id: record.run_id,
        status: record.status.lock().await.clone(),
        event_count: record.events.lock().await.len(),
    }))
}

async fn cancel_job(
    State(state): State<ApiState>,
    Path(job_id): Path<JobId>,
) -> Result<Json<JobStateResponse>, ApiError> {
    let record = find_job(&state, job_id).await?;
    if let Some(handle) = record.handle.lock().await.take() {
        handle.abort();
    }
    *record.status.lock().await = RunStatus::Cancelled;
    Ok(Json(JobStateResponse {
        job_id: record.job_id,
        run_id: record.run_id,
        status: RunStatus::Cancelled,
        event_count: record.events.lock().await.len(),
    }))
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
    let engine = build_engine(state, &record.message, req)?;
    let state_store = StateStore::new(&state.inner.workspace.state_dir);
    let trace_writer = state_store.run_store.create_trace(&record.run_id).ok();
    let request = RunRequest {
        session_id: record.session_id,
        job_id: record.job_id,
        run_id: record.run_id,
        user_message: record.message.clone(),
        resume_state: None,
    };
    let mut stream = std::pin::pin!(engine.run(request, trace_writer));
    while let Some(event) = stream.next().await {
        if matches!(event, StreamEvent::RunCompleted { .. }) {
            *record.status.lock().await = RunStatus::Done;
        }
        record.events.lock().await.push(event.clone());
        let _ = record.tx.send(event);
    }
    Ok(())
}

fn build_engine(state: &ApiState, message: &str, req: &CreateJobRequest) -> anyhow::Result<Engine> {
    let config = &state.inner.config;
    let model_id = req.model.clone().unwrap_or_else(|| config.model.clone());
    let model: Box<dyn ModelClient> = if model_id == "fake" {
        Box::new(FakeModelClient::new(format!("fake response: {message}")))
    } else {
        Box::new(OpenAiClient::new(
            config.api_base.clone(),
            config.api_key.clone(),
            model_id,
        ))
    };

    let workspace = state.inner.workspace.clone();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    #[cfg(feature = "rag")]
    registry.register(Box::new(RagRetrieveTool::code(workspace.root.clone())));
    #[cfg(feature = "rag")]
    registry.register(Box::new(RagRetrieveTool::docs(workspace.root.clone())));
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));

    Ok(Engine::with_workspace(
        model,
        registry,
        ContextManager::new(config.load_system_prompt()),
        EngineConfig {
            max_steps: req.max_steps.unwrap_or(config.max_steps),
            plan_enabled: true,
        },
        workspace,
        ApprovalPolicy::Auto,
    ))
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
