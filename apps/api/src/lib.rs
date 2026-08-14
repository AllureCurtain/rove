use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::{DefaultBodyLimit, Query};
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
use rove_app_bootstrap::{
    AppConfig, AppConfigOverrides, ProjectActivationState, ProjectTrustRepository, ProviderCatalog,
    ProviderCatalogService, UserConfigPaths,
};
use rove_app_bootstrap::{EngineOptions, build_engine};
use rove_core::ToolError;
use rove_models::ModelClient;
use rove_models::fake::FakeModelClient;
use rove_models::health::{HealthConfig, ModelHealthStore};
use rove_runtime::agents::{AgentActivationError, SelectorError};
use rove_runtime::engine::{Engine, RunControlHandle};
use rove_runtime::events::StreamEvent;
use rove_runtime::runtime_identity::RunModelSnapshot;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::index::{ResumeJobClaim, RunIndexRecord, StateIndex};
use rove_runtime::state::resume::resolve_resume_state;
use rove_runtime::state::store::{RunHandle, StateStore};
use rove_runtime::state::trace::TraceWriter;
use rove_runtime::tools::mcp_proxy::McpServerRuntimeSnapshot;
use rove_runtime::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, Message, PendingToolApproval,
    PendingUserInput, Role, RunId, RunStatus, SessionId, TaskState, TerminationReason,
    ToolApprovalProvider, ToolApprovalRequest, UserInputProvider, UserInputRequest,
};
use rove_runtime::workspace::Workspace;

mod benchmark;
mod debug;
mod docs;
mod pricing;
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
pub(crate) const PRODUCT_MIGRATION_PREPARATION_DEADLINE: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct ApiState {
    inner: Arc<ApiStateInner>,
}

struct ApiStateInner {
    workspace: Workspace,
    config: AppConfig,
    product_store_path: PathBuf,
    product_store: Option<Arc<dyn ProductStore>>,
    provider_catalog: ProviderCatalogService,
    project_trust: Option<Arc<ProjectTrustRepository>>,
    product_transcript_reader: Option<Arc<dyn ProductTranscriptReader>>,
    shutdown_token: CancellationToken,
    job_starts: TaskTracker,
    supervisors: TaskTracker,
    jobs: RwLock<HashMap<JobId, Arc<JobRecord>>>,
    mcp_health: RwLock<HashMap<PathBuf, Vec<McpServerRuntimeSnapshot>>>,
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

impl ApiProductRuntimeStateResolver {
    fn state_store_for_workspace(&self, mut workspace: Workspace) -> StateStore {
        let mut config = self.config.clone();
        config.source_summary.workspace_root = workspace.root.clone();
        config.source_summary.project_config_path = workspace.root.join(".rove/config.toml");
        config.source_summary.project_config_loaded = false;
        workspace.state_dir = config.state_dir();
        state_store_for_parts(&workspace, &config)
    }
}

impl ProductRuntimeStateResolver for ApiProductRuntimeStateResolver {
    fn state_store_for(
        &self,
        product_workspace: &ProductWorkspace,
    ) -> Result<StateStore, ProductStoreError> {
        let workspace = match product_workspace.kind {
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
        Ok(self.state_store_for_workspace(workspace))
    }
}

pub(crate) struct JobRecord {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    workspace: Workspace,
    config: AppConfig,
    message: String,
    resumed_from_run_id: Option<RunId>,
    resume_state: Option<TaskState>,
    /// Product session this job is bound to (if any). Used for steer delivery
    /// and follow-up drain after terminal.
    pub(crate) product_session_id: Option<ProductSessionId>,
    product_store: Option<Arc<dyn ProductStore>>,
    /// Captured by ProductStore while claiming this turn. It is intentionally
    /// immutable for the lifetime of the runtime job.
    product_model_config: Option<ProductSessionModelConfig>,
    run_model_snapshot: Option<RunModelSnapshot>,
    status: Mutex<RunStatus>,
    events: Mutex<Vec<JobStreamEvent>>,
    pending_approvals: Mutex<HashMap<CallId, PendingApproval>>,
    pending_inputs: Mutex<HashMap<CallId, PendingInput>>,
    tx: broadcast::Sender<JobStreamEvent>,
    handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// In-flight steer handle captured from the engine stream.
    pub(crate) control: Mutex<Option<RunControlHandle>>,
    /// The active run trace is retained for API-originated canonical control
    /// events such as `followup_queued`. These events do not alter runtime
    /// prompt/history projection: their durable scheduling authority is the
    /// ProductStore row, while `trace.jsonl` remains the transcript fact.
    control_event_trace: Mutex<Option<TraceWriter>>,
    control_event_trace_lock: Mutex<()>,
    /// Serializes API control creation against terminal control cleanup. A
    /// steer cannot become pending after the runtime has passed its final safe
    /// point and before the session turn is released.
    pub(crate) control_lifecycle_lock: Mutex<()>,
    /// Product control events submitted before the engine has persisted its
    /// mandatory `run_started` fact. The lifecycle lock protects this queue
    /// together with trace installation, so it cannot be stranded between the
    /// two phases.
    pending_product_events: Mutex<Vec<StreamEvent>>,
    completion: watch::Sender<bool>,
    cancel_token: CancellationToken,
}

struct JobLaunch {
    record: Arc<JobRecord>,
    engine: Engine,
    run: RunHandle,
    product_turn: Option<ProductTurnSupervisor>,
    startup_events: Vec<StreamEvent>,
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
    schedule_pending_followup_recovery(&state);
    let migration_router: OpenApiRouter<ApiState> = OpenApiRouter::new()
        .routes(routes!(product::routes::migrate_m1_browser_state))
        .route_layer(DefaultBodyLimit::max(MAX_M1_BROWSER_MIGRATION_BODY_BYTES));
    let (api_router, api) = OpenApiRouter::with_openapi(docs::ApiDoc::openapi())
        .routes(routes!(list_provider_models))
        .routes(routes!(test_provider))
        .routes(routes!(product::routes::list_product_workspaces))
        .routes(routes!(product::routes::create_product_workspace))
        .routes(routes!(product::routes::delete_product_workspace))
        .routes(routes!(product::routes::list_product_sessions))
        .routes(routes!(product::routes::create_product_session))
        .routes(routes!(product::routes::create_product_session_fork))
        .routes(routes!(product::routes::list_product_session_forks))
        .routes(routes!(product::routes::update_product_session))
        .routes(routes!(product::routes::delete_product_session))
        .routes(routes!(product::routes::get_product_session_transcript))
        .routes(routes!(product::routes::get_product_session_model_config))
        .routes(routes!(
            product::routes::update_product_session_model_config
        ))
        .routes(routes!(product::routes::list_product_session_run_models))
        .routes(routes!(product::usage::get_product_session_usage))
        .routes(routes!(product::files::list_workspace_files))
        .routes(routes!(product::files::get_workspace_file_content))
        .routes(routes!(product::files::download_workspace_file))
        .routes(routes!(product::files::preview_workspace_file))
        .routes(routes!(product::artifacts::list_session_artifacts))
        .routes(routes!(product::artifacts::get_artifact_content))
        .routes(routes!(product::artifacts::download_artifact))
        .routes(routes!(product::artifacts::preview_artifact))
        .routes(routes!(product::diff::get_session_diff))
        .routes(routes!(product::export::export_product_session))
        .routes(routes!(product::routes::list_product_provider_profiles))
        .routes(routes!(product::routes::create_product_provider_profile))
        .routes(routes!(product::routes::update_product_provider_profile))
        .routes(routes!(product::routes::delete_product_provider_profile))
        .routes(routes!(product::routes::list_product_provider_models))
        .routes(routes!(product::routes::get_product_preferences))
        .routes(routes!(product::routes::update_product_preferences))
        .routes(routes!(product::routes::create_product_session_steer))
        .routes(routes!(product::routes::create_product_session_followup))
        .routes(routes!(product::routes::create_product_session_message))
        .routes(routes!(product::routes::list_product_session_messages))
        .routes(routes!(product::routes::promote_product_session_message))
        .routes(routes!(product::routes::revoke_product_session_message))
        .routes(routes!(product::routes::list_product_session_controls))
        .routes(routes!(product::routes::revoke_product_session_control))
        .routes(routes!(product::routes::confirm_product_session_followup))
        .routes(routes!(product::platform::list_product_memory_topics))
        .routes(routes!(product::platform::create_product_memory_topic))
        .routes(routes!(product::platform::get_product_memory_topic))
        .routes(routes!(product::platform::update_product_memory_topic))
        .routes(routes!(product::platform::delete_product_memory_topic))
        .routes(routes!(product::mcp::list_product_mcp_servers))
        .routes(routes!(product::mcp::get_product_mcp_health))
        .routes(routes!(product::mcp::create_product_mcp_server))
        .routes(routes!(product::mcp::update_product_mcp_server))
        .routes(routes!(product::mcp::delete_product_mcp_server))
        .routes(routes!(product::mcp::probe_product_mcp_server))
        .routes(routes!(product::trust::get_project_trust))
        .routes(routes!(product::trust::decide_project_trust))
        .routes(routes!(product::platform::get_product_runtime_info))
        .merge(migration_router)
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

fn schedule_pending_followup_recovery(state: &ApiState) {
    if state.inner.shutdown_token.is_cancelled() || state.inner.job_starts.is_closed() {
        return;
    }
    let state = state.clone();
    let job_starts = state.inner.job_starts.clone();
    drop(job_starts.spawn(async move {
        recover_pending_followup_drains(state).await;
    }));
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
            trust_project: false,
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
    serve_state_listener(listener, state).await
}

/// Assemble API state for a trusted in-process delivery host. The API crate
/// retains ownership of AppConfig, Workspace, and ProductStore wiring so an
/// embedding host does not reproduce backend assembly across package layers.
pub fn embedded_api_state(
    cwd: &FsPath,
    bind_addr: SocketAddr,
    state_dir: PathBuf,
    bearer_token: String,
    cors_origins: Vec<String>,
    shutdown: CancellationToken,
) -> anyhow::Result<ApiState> {
    let mut config = AppConfig::load(
        cwd,
        AppConfigOverrides {
            api_bind_addr: Some(bind_addr.to_string()),
            trust_project: false,
            ..AppConfigOverrides::default()
        },
    )?;
    config.api.bind_addr = bind_addr.to_string();
    config.api.token_auth = Some(bearer_token);
    config.api.cors_origins = cors_origins;
    config.state.state_dir = state_dir.clone();
    config.state.sqlite_path = state_dir.join("state.sqlite");
    config.source_summary.workspace_root = cwd.to_path_buf();
    config.source_summary.project_config_path = cwd.join(".rove/config.toml");

    let mut workspace = Workspace::detect(cwd)?;
    workspace.state_dir = config.state_dir();
    workspace.ensure_state_dir()?;
    Ok(ApiState::with_shutdown(workspace, config, shutdown))
}

/// Serve an already-assembled API state and perform the same complete shutdown
/// drain used by the standalone API binary.
pub async fn serve_state_listener(
    listener: tokio::net::TcpListener,
    state: ApiState,
) -> anyhow::Result<()> {
    let shutdown = state.inner.shutdown_token.clone();
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
        config: AppConfig,
        shutdown_token: CancellationToken,
    ) -> Self {
        let project_trust = match ProjectTrustRepository::operator_default() {
            Ok(repository) => Some(Arc::new(repository)),
            Err(error) => {
                tracing::warn!("project trust authority is unavailable: {error}");
                None
            }
        };
        Self::with_shutdown_and_project_trust(workspace, config, shutdown_token, project_trust)
    }

    /// Build API state with an explicit canonical Project Trust authority.
    /// Embedders and tests can use the same repository instance as CLI or
    /// bootstrap without relying on process-global path overrides.
    pub fn with_project_trust_repository(
        workspace: Workspace,
        config: AppConfig,
        project_trust: Arc<ProjectTrustRepository>,
    ) -> Self {
        Self::with_shutdown_and_project_trust(
            workspace,
            config,
            CancellationToken::new(),
            Some(project_trust),
        )
    }

    fn with_shutdown_and_project_trust(
        workspace: Workspace,
        mut config: AppConfig,
        shutdown_token: CancellationToken,
        project_trust: Option<Arc<ProjectTrustRepository>>,
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
        let provider_catalog = ProviderCatalogService::new(UserConfigPaths::for_config_file(
            &config.source_summary.user_config_path,
        ));
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
        if let Some(repository) = project_trust.as_ref()
            && let Err(error) = repository.import_product_store_snapshot(&product_store_path)
        {
            tracing::warn!("project trust compatibility import failed: {error}");
        }
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
                provider_catalog,
                project_trust,
                product_transcript_reader,
                shutdown_token,
                job_starts: TaskTracker::new(),
                supervisors: TaskTracker::new(),
                jobs: RwLock::new(HashMap::new()),
                mcp_health: RwLock::new(HashMap::new()),
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

    pub(crate) fn provider_catalog_service(&self) -> ProviderCatalogService {
        self.inner.provider_catalog.clone()
    }

    pub(crate) async fn provider_catalog(&self) -> Result<ProviderCatalog, ApiError> {
        let service = self.provider_catalog_service();
        tokio::task::spawn_blocking(move || service.load())
            .await
            .map_err(|_| ApiError::internal("provider catalog operation did not complete"))?
            .map_err(product::provider_catalog::catalog_error)
    }

    pub(crate) fn project_trust(&self) -> Result<Arc<ProjectTrustRepository>, ApiError> {
        self.inner.project_trust.clone().ok_or_else(|| {
            ProductStoreError::new(
                ProductErrorCode::ProjectTrustUnavailable,
                "project trust authority is not available",
            )
            .into()
        })
    }

    pub(crate) async fn quarantine_workspace_jobs(&self, workspace_root: &FsPath) {
        let jobs = self.inner.jobs.read().await;
        for record in jobs.values() {
            if record.workspace.root == workspace_root {
                record.cancel_token.cancel();
            }
        }
    }

    pub(crate) fn product_transcript_reader(
        &self,
    ) -> Result<Arc<dyn ProductTranscriptReader>, ApiError> {
        self.inner
            .product_transcript_reader
            .clone()
            .ok_or_else(|| ProductStoreError::unavailable().into())
    }

    pub(crate) fn product_state_store_for_workspace(&self, workspace: &Workspace) -> StateStore {
        ApiProductRuntimeStateResolver {
            config: self.inner.config.clone(),
        }
        .state_store_for_workspace(workspace.clone())
    }

    pub(crate) fn product_state_store_for_product_workspace(
        &self,
        workspace: &ProductWorkspace,
    ) -> Result<StateStore, ProductStoreError> {
        ApiProductRuntimeStateResolver {
            config: self.inner.config.clone(),
        }
        .state_store_for(workspace)
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

    let response = start_tracked_job(state, req)
        .await
        .map_err(|_| ApiError::internal("job start task did not complete"))??;

    Ok(Json(response))
}

fn start_tracked_job(
    state: ApiState,
    req: CreateJobRequest,
) -> oneshot::Receiver<Result<CreateJobResponse, ApiError>> {
    let (response_tx, response_rx) = oneshot::channel();
    let job_starts = state.inner.job_starts.clone();
    drop(job_starts.spawn(async move {
        let result = prepare_and_start_job(state, req).await;
        let _ = response_tx.send(result);
    }));
    response_rx
}

async fn prepare_and_start_job(
    state: ApiState,
    req: CreateJobRequest,
) -> Result<CreateJobResponse, ApiError> {
    let launch = prepare_job_launch(&state, &req).await?;
    let record = Arc::clone(&launch.record);
    let response = CreateJobResponse {
        job_id: record.job_id,
        run_id: record.run_id,
        resumed_from_run_id: record.resumed_from_run_id,
        workspace_activation: record.config.project_activation_state().into(),
    };
    start_job_supervisor(state, launch).await;

    Ok(response)
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
        (status = 429, description = "Provider rate limited the inventory request", body = ApiErrorResponse, content_type = "application/json"),
        (status = 504, description = "Provider inventory request timed out", body = ApiErrorResponse, content_type = "application/json"),
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
        (status = 429, description = "Provider rate limited the inventory request", body = ApiErrorResponse, content_type = "application/json"),
        (status = 504, description = "Provider inventory request timed out", body = ApiErrorResponse, content_type = "application/json"),
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
            let approval_policy = resolve_product_job_approval_policy(state).await?;
            prepare_product_job_launch(state, req, product_session_id, approval_policy).await
        }
        None => {
            let approval_policy = req.approval.unwrap_or(ApprovalPolicy::Ask);
            prepare_generic_job_launch(state, req, approval_policy).await
        }
    }
}

async fn resolve_product_job_approval_policy(state: &ApiState) -> Result<ApprovalPolicy, ApiError> {
    let store = state.product_store()?;
    let preference = store.get_preferences().await?.default_approval_policy;
    Ok(match preference {
        ProductApprovalPreference::Ask => ApprovalPolicy::Ask,
        ProductApprovalPreference::Auto => ApprovalPolicy::Auto,
        ProductApprovalPreference::Never => ApprovalPolicy::Never,
    })
}

async fn prepare_generic_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
    approval_policy: ApprovalPolicy,
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
    let resumed_from_run_id = resume_state.as_ref().map(|task_state| task_state.run_id);
    let record = new_job_record(NewJobRecord {
        state,
        workspace,
        config,
        request: req,
        session_id,
        job_id,
        resume_state,
        resumed_from_run_id,
        product_session_id: None,
        product_store: None,
        product_model_config: None,
        run_model_snapshot: None,
    });
    let engine = match assemble_job_engine(
        state,
        &record.message,
        req,
        Arc::clone(&record),
        approval_policy,
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            tracing::warn!(job_id = %record.job_id, "failed to assemble job engine: {error}");
            return Err(ApiError::agent_engine_assembly(&error));
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
        startup_events: Vec::new(),
    })
}

async fn prepare_product_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
    product_session_id: &ProductSessionId,
    approval_policy: ApprovalPolicy,
) -> Result<JobLaunch, ApiError> {
    let store = state.product_store()?;
    let claim = store.claim_session_turn(product_session_id).await?;
    prepare_claimed_product_job_launch(
        state,
        req,
        product_session_id,
        store,
        claim,
        approval_policy,
        None,
    )
    .await
}

async fn prepare_followup_job_launch(
    state: &ApiState,
    product_session_id: &ProductSessionId,
    claim: ProductFollowupTurnClaim,
) -> Result<JobLaunch, ApiError> {
    let store = state.product_store()?;
    let request = CreateJobRequest {
        message: claim.control.content.clone(),
        model: None,
        max_steps: None,
        agent: None,
        approval: None,
        resume: None,
        workspace: None,
        provider: None,
        product_session_id: Some(product_session_id.clone()),
    };
    let approval_policy = match resolve_product_job_approval_policy(state).await {
        Ok(policy) => policy,
        Err(error) => {
            release_failed_followup_start(
                &store,
                &claim.turn.claim_id,
                &claim.control.id,
                false,
                "approval policy resolution",
            )
            .await;
            return Err(error);
        }
    };
    prepare_claimed_product_job_launch(
        state,
        &request,
        product_session_id,
        store,
        claim.turn,
        approval_policy,
        Some(claim.control.id),
    )
    .await
}

async fn release_failed_followup_start(
    store: &Arc<dyn ProductStore>,
    claim_id: &ProductTurnClaimId,
    control_id: &ProductControlId,
    needs_attention: bool,
    phase: &'static str,
) {
    let result = if needs_attention {
        store
            .abandon_followup_turn(claim_id, control_id, phase)
            .await
    } else {
        store.requeue_followup_turn(claim_id, control_id).await
    };
    if let Err(error) = result {
        tracing::warn!(
            control_id = %control_id,
            phase = phase,
            "failed to release automatic follow-up turn: {error}"
        );
    }
}

async fn prepare_claimed_product_job_launch(
    state: &ApiState,
    req: &CreateJobRequest,
    product_session_id: &ProductSessionId,
    store: Arc<dyn ProductStore>,
    claim: ProductTurnClaim,
    approval_policy: ApprovalPolicy,
    followup_control_id: Option<ProductControlId>,
) -> Result<JobLaunch, ApiError> {
    let claim_id = claim.claim_id.clone();
    let previous_product_status = claim.previous_status;
    let product_model_config = claim.model_config.clone();

    let (workspace, config, run_model_snapshot) = match workspace_and_config_for_product_job(
        state,
        req,
        &claim.context.workspace,
        &store,
        &product_model_config,
        claim.previous_binding.is_some(),
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            let provider_resume_failure = matches!(
                error.code,
                "provider_unavailable_for_resume" | "provider_changed_for_resume"
            );
            if let Some(control_id) = &followup_control_id {
                release_failed_followup_start(
                    &store,
                    &claim_id,
                    control_id,
                    provider_resume_failure,
                    "workspace validation",
                )
                .await;
            } else {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    None,
                    if provider_resume_failure {
                        ProductSessionStatus::NeedsAttention
                    } else {
                        previous_product_status
                    },
                    "workspace validation",
                )
                .await;
            }
            return Err(error);
        }
    };
    let state_store = state_store_for_parts(&workspace, &config);
    // A fork's first child turn starts with the source run's verified prompt
    // state, but must never resume the source job. The fresh runtime session
    // and job identities keep cancellation, controls, and later follow-ups
    // isolated from the parent. Subsequent child turns use their own binding.
    let (resume_state, mut resume_claim, fork_bootstrap) = match claim.previous_binding.as_ref() {
        Some(previous) => {
            match load_and_claim_product_resume(&state_store, previous, &run_model_snapshot).await {
                Ok((resume_state, resume_claim)) => (Some(resume_state), Some(resume_claim), false),
                Err(error) => {
                    if let Some(control_id) = &followup_control_id {
                        release_failed_followup_start(
                            &store,
                            &claim_id,
                            control_id,
                            true,
                            "exact runtime resume validation",
                        )
                        .await;
                    } else {
                        finish_failed_product_start(
                            &store,
                            &claim_id,
                            None,
                            ProductSessionStatus::NeedsAttention,
                            "exact runtime resume validation",
                        )
                        .await;
                    }
                    return Err(error);
                }
            }
        }
        None => match claim.context.fork.as_ref() {
            Some(fork) => match load_product_fork_resume(&state_store, &fork.fork).await {
                Ok(resume_state) => (Some(resume_state), None, true),
                Err(error) => {
                    if let Some(control_id) = &followup_control_id {
                        release_failed_followup_start(
                            &store,
                            &claim_id,
                            control_id,
                            true,
                            "fork source resume validation",
                        )
                        .await;
                    } else {
                        finish_failed_product_start(
                            &store,
                            &claim_id,
                            None,
                            ProductSessionStatus::NeedsAttention,
                            "fork source resume validation",
                        )
                        .await;
                    }
                    return Err(error);
                }
            },
            None => (None, None, false),
        },
    };
    let session_id = if fork_bootstrap {
        SessionId::new()
    } else {
        resume_state
            .as_ref()
            .map(|task_state| task_state.session_id)
            .unwrap_or_else(SessionId::new)
    };
    let job_id = if fork_bootstrap {
        JobId::new()
    } else {
        resume_state
            .as_ref()
            .map(|task_state| task_state.job_id)
            .unwrap_or_else(JobId::new)
    };
    // Fork bootstrap reuses the verified source task state only as a history
    // seed. It is a new runtime lineage, so the child binding must not be
    // classified as a normal resume of the parent run.
    let resumed_from_run_id = if fork_bootstrap {
        None
    } else {
        resume_state.as_ref().map(|task_state| task_state.run_id)
    };
    let record = new_job_record(NewJobRecord {
        state,
        workspace,
        config,
        request: req,
        session_id,
        job_id,
        resume_state,
        resumed_from_run_id,
        product_session_id: Some(product_session_id.clone()),
        product_store: Some(store.clone()),
        product_model_config: Some(product_model_config),
        run_model_snapshot: Some(run_model_snapshot),
    });
    let engine = match assemble_job_engine(
        state,
        &record.message,
        req,
        Arc::clone(&record),
        approval_policy,
    )
    .await
    {
        Ok(engine) => engine,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            if let Some(control_id) = &followup_control_id {
                release_failed_followup_start(
                    &store,
                    &claim_id,
                    control_id,
                    false,
                    "engine assembly",
                )
                .await;
            } else {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    None,
                    previous_product_status,
                    "engine assembly",
                )
                .await;
            }
            tracing::warn!(job_id = %record.job_id, "failed to assemble product job engine: {error}");
            return Err(ApiError::agent_engine_assembly(&error));
        }
    };

    if let Some(control_id) = &followup_control_id
        && let Err(error) = store
            .reserve_followup_run(&claim_id, control_id, record.run_id)
            .await
    {
        release_runtime_resume_claim(&state_store, resume_claim.take()).await;
        release_failed_followup_start(
            &store,
            &claim_id,
            control_id,
            true,
            "runtime run reservation",
        )
        .await;
        return Err(error.into());
    }
    let run = match state_store.start_run(record.session_id, record.job_id, record.run_id) {
        Ok(run) => run,
        Err(error) => {
            release_runtime_resume_claim(&state_store, resume_claim.take()).await;
            if let Some(control_id) = &followup_control_id {
                release_failed_followup_start(
                    &store,
                    &claim_id,
                    control_id,
                    true,
                    "runtime run start",
                )
                .await;
            } else {
                finish_failed_product_start(
                    &store,
                    &claim_id,
                    Some(record.run_id),
                    ProductSessionStatus::NeedsAttention,
                    "runtime run start",
                )
                .await;
            }
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
            followup_control_id: followup_control_id.clone(),
            model_config: record.product_model_config.clone().ok_or_else(|| {
                ProductStoreError::new(
                    ProductErrorCode::ProductBindingCorrupt,
                    "product run is missing its claimed model configuration",
                )
            })?,
            run_model_snapshot: record.run_model_snapshot.clone(),
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
        if let Some(control_id) = &followup_control_id {
            release_failed_followup_start(
                &store,
                &claim_id,
                control_id,
                true,
                "runtime binding commit",
            )
            .await;
        } else {
            finish_failed_product_start(
                &store,
                &claim_id,
                Some(record.run_id),
                ProductSessionStatus::NeedsAttention,
                "runtime binding commit",
            )
            .await;
        }
        return Err(error.into());
    }

    Ok(JobLaunch {
        record,
        engine,
        run,
        product_turn: Some(ProductTurnSupervisor {
            store: store.clone(),
            claim_id,
        }),
        startup_events: match followup_control_id {
            Some(control_id) => match store.get_message(product_session_id, &control_id).await {
                Ok(message) => vec![
                    StreamEvent::MessageQueued {
                        id: control_id.to_string(),
                        content: message.content,
                    },
                    StreamEvent::MessageClaimedSuccessor {
                        id: control_id.to_string(),
                    },
                ],
                Err(_) => vec![StreamEvent::FollowupDequeued {
                    id: control_id.to_string(),
                }],
            },
            None => Vec::new(),
        },
    })
}

struct NewJobRecord<'a> {
    state: &'a ApiState,
    workspace: Workspace,
    config: AppConfig,
    request: &'a CreateJobRequest,
    session_id: SessionId,
    job_id: JobId,
    resume_state: Option<TaskState>,
    resumed_from_run_id: Option<RunId>,
    product_session_id: Option<ProductSessionId>,
    product_store: Option<Arc<dyn ProductStore>>,
    product_model_config: Option<ProductSessionModelConfig>,
    run_model_snapshot: Option<RunModelSnapshot>,
}

fn new_job_record(input: NewJobRecord<'_>) -> Arc<JobRecord> {
    let NewJobRecord {
        state,
        workspace,
        config,
        request,
        session_id,
        job_id,
        resume_state,
        resumed_from_run_id,
        product_session_id,
        product_store,
        product_model_config,
        run_model_snapshot,
    } = input;
    let run_id = RunId::new();
    let (tx, _) = broadcast::channel(EVENT_BUFFER);
    let (completion, _) = watch::channel(false);
    Arc::new(JobRecord {
        session_id,
        job_id,
        run_id,
        workspace,
        config,
        message: request.message.clone(),
        resumed_from_run_id,
        resume_state,
        product_session_id,
        product_store,
        product_model_config,
        run_model_snapshot,
        status: Mutex::new(RunStatus::Running),
        events: Mutex::new(Vec::new()),
        pending_approvals: Mutex::new(HashMap::new()),
        pending_inputs: Mutex::new(HashMap::new()),
        tx,
        handle: Mutex::new(None),
        control: Mutex::new(None),
        control_event_trace: Mutex::new(None),
        control_event_trace_lock: Mutex::new(()),
        control_lifecycle_lock: Mutex::new(()),
        pending_product_events: Mutex::new(Vec::new()),
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
    current_run_model: &RunModelSnapshot,
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
    validate_product_resume_model(&resume_state, current_run_model)?;
    let resume_state = project_product_follow_up_state(resume_state)?;
    let Some(claim) = claim_runtime_resume(state_store, Some(&resume_state), true).await? else {
        return Err(ApiError::internal(
            "product resume validation did not acquire a runtime claim",
        ));
    };
    Ok((resume_state, claim))
}

fn validate_product_resume_model(
    resume_state: &TaskState,
    current: &RunModelSnapshot,
) -> Result<(), ApiError> {
    let saved = resume_state
        .checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.runtime_identity.as_ref())
        .or(resume_state.runtime_identity.as_ref())
        .and_then(|identity| identity.run_model.as_ref());
    let Some(saved) = saved else {
        return Ok(());
    };

    let selection_changed = saved.profile_id != current.profile_id
        || saved.model != current.model
        || saved.reasoning != current.reasoning;
    if selection_changed {
        return Ok(());
    }
    let compatible = saved.provider_type == current.provider_type
        && saved.wire_protocol == current.wire_protocol
        && saved.endpoint == current.endpoint
        && saved.safe_config_digest == current.safe_config_digest;
    if compatible {
        Ok(())
    } else {
        Err(ApiError::conflict_with_code(
            ProductErrorCode::ProviderChangedForResume.as_str(),
            "the selected Provider changed since the previous run; start a new session or restore the original Provider identity",
        ))
    }
}

/// Verify a fork request against the parent session's immutable product binding
/// and the canonical terminal runtime artifacts. The browser supplies only a
/// run id; every other source identity and the terminal event sequence come
/// from server-owned state.
pub(crate) async fn verify_product_fork_boundary(
    state: &ApiState,
    parent_session_id: &ProductSessionId,
    fork_at_run_id: RunId,
) -> Result<VerifiedProductForkBoundary, ApiError> {
    let store = state.product_store()?;
    let context = store.get_session_context(parent_session_id).await?;
    if context.session.status == ProductSessionStatus::Running {
        return Err(ProductStoreError::new(
            ProductErrorCode::ProductSessionActive,
            "a product session can only be forked after its active turn reaches a terminal boundary",
        )
        .into());
    }
    if context.session.status != ProductSessionStatus::Idle {
        return Err(fork_source_rejection(
            "only an idle product session with a final durable run can be forked",
        ));
    }
    let bindings = store.list_run_bindings(parent_session_id).await?;
    let Some(binding) = bindings
        .iter()
        .find(|binding| binding.runtime_run_id == fork_at_run_id)
    else {
        return Err(fork_source_rejection(
            "the requested runtime run is not bound to the parent product session",
        ));
    };
    let state_store = state.product_state_store_for_product_workspace(&context.workspace)?;
    let (_, terminal_event_seq) = load_final_fork_source_state(
        &state_store,
        binding.runtime_session_id,
        binding.runtime_job_id,
        binding.runtime_run_id,
        None,
    )
    .await?;
    Ok(VerifiedProductForkBoundary {
        parent_product_session_id: context.session.id,
        parent_workspace_id: context.workspace.id,
        parent_title: context.session.title,
        source_runtime_session_id: binding.runtime_session_id,
        source_runtime_job_id: binding.runtime_job_id,
        source_runtime_run_id: binding.runtime_run_id,
        fork_at_event_seq: terminal_event_seq,
    })
}

async fn load_product_fork_resume(
    state_store: &StateStore,
    fork: &ProductFork,
) -> Result<TaskState, ApiError> {
    let (state, _) = load_final_fork_source_state(
        state_store,
        fork.source_runtime_session_id,
        fork.source_runtime_job_id,
        fork.source_runtime_run_id,
        Some(fork.fork_at_event_seq),
    )
    .await?;
    project_product_follow_up_state(state)
}

async fn load_final_fork_source_state(
    state_store: &StateStore,
    runtime_session_id: SessionId,
    runtime_job_id: JobId,
    runtime_run_id: RunId,
    expected_terminal_event_seq: Option<u64>,
) -> Result<(TaskState, u64), ApiError> {
    let job = state_store
        .index
        .job_record_async(runtime_job_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| fork_source_rejection("the fork source runtime job is missing"))?;
    if job.session_id != runtime_session_id || job.run_id != Some(runtime_run_id) {
        return Err(fork_source_rejection(
            "the fork source runtime job identity does not match its product binding",
        ));
    }
    let run = load_runtime_run_record(&state_store.index, runtime_run_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| fork_source_rejection("the fork source runtime run is missing"))?;
    if run.session_id != runtime_session_id
        || run.job_id != runtime_job_id
        || run.run_id != runtime_run_id
        || run.status != "done"
        || run.task_state_path.is_none()
        || run.report_path.is_none()
        || run.last_event_seq == 0
    {
        return Err(fork_source_rejection(
            "the fork source runtime run is not a complete durable terminal boundary",
        ));
    }
    if expected_terminal_event_seq.is_some_and(|expected| expected != run.last_event_seq) {
        return Err(fork_source_rejection(
            "the fork source terminal event sequence no longer matches its stored boundary",
        ));
    }
    let task_state = state_store
        .load_task_state(runtime_run_id)
        .await
        .map_err(|_| {
            fork_source_rejection("the fork source task state is unavailable or corrupt")
        })?;
    if task_state.session_id != runtime_session_id
        || task_state.job_id != runtime_job_id
        || task_state.run_id != runtime_run_id
        || task_state
            .checkpoint
            .as_ref()
            .and_then(|checkpoint| checkpoint.last_event_seq)
            != Some(run.last_event_seq)
    {
        return Err(fork_source_rejection(
            "the fork source task-state identity or terminal checkpoint is invalid",
        ));
    }
    let report = state_store
        .load_report(runtime_run_id)
        .await
        .map_err(|_| fork_source_rejection("the fork source report is unavailable or corrupt"))?;
    if report.session_id != runtime_session_id
        || report.job_id != runtime_job_id
        || report.run_id != runtime_run_id
        || report.status != "success"
        || report.termination_reason != TerminationReason::Final
    {
        return Err(fork_source_rejection(
            "the fork source report does not prove a final completed run",
        ));
    }
    let snapshot = state_store
        .index
        .run_event_snapshot_async(runtime_run_id, run.last_event_seq.saturating_sub(1), 1)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| fork_source_rejection("the fork source canonical event is missing"))?;
    if snapshot.high_water_seq != run.last_event_seq
        || snapshot.has_more
        || snapshot.events.len() != 1
        || snapshot.events[0].seq != run.last_event_seq
    {
        return Err(fork_source_rejection(
            "the fork source canonical terminal event range is incomplete",
        ));
    }
    let terminal = serde_json::from_str::<StreamEvent>(&snapshot.events[0].event_json)
        .map_err(|_| fork_source_rejection("the fork source terminal event is corrupt"))?;
    if !matches!(
        terminal,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            ..
        }
    ) {
        return Err(fork_source_rejection(
            "the fork source does not end with a final canonical completion event",
        ));
    }
    Ok((task_state, run.last_event_seq))
}

fn fork_source_rejection(message: impl Into<String>) -> ApiError {
    ProductStoreError::new(ProductErrorCode::ProductForkSourceInvalid, message).into()
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
    run_id: Option<RunId>,
    status: ProductSessionStatus,
    phase: &'static str,
) {
    if let Err(error) = store
        .finish_session_turn_and_abandon_pending_controls(claim_id, run_id, status, phase)
        .await
    {
        tracing::warn!(
            phase = phase,
            "failed to classify controls after product start failure: {error}"
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
    let trace_persisted = append_trace_event(&run.trace_writer, record, &terminal).await;
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
        startup_events,
    } = launch;
    let record_for_task = Arc::clone(&record);
    let recovery_record = Arc::clone(&record);
    let recovery_product_turn = product_turn.clone();
    let completion = record.completion.clone();
    let state_for_supervisor = state.clone();
    // Register before spawning the stream. A control submitted after the
    // ProductStore turn claim but before `consume_job_stream` installs its
    // handle is either replayed from the durable queue or explicitly
    // rejected; it cannot be inserted after the one-time replay and become
    // stranded forever.
    state
        .inner
        .jobs
        .write()
        .await
        .insert(record.job_id, Arc::clone(&record));
    let handle = state.inner.supervisors.spawn(async move {
        let _completion_guard = JobCompletionGuard::new(completion);
        let outcome = AssertUnwindSafe(run_job_supervisor(
            state_for_supervisor,
            record_for_task,
            engine,
            run,
            product_turn,
            startup_events,
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
}

async fn run_job_supervisor(
    state: ApiState,
    record: Arc<JobRecord>,
    engine: Engine,
    run: RunHandle,
    product_turn: Option<ProductTurnSupervisor>,
    startup_events: Vec<StreamEvent>,
) {
    let trust_monitor = start_project_trust_monitor(&state, &record).await;
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
        startup_events,
    ))
    .catch_unwind()
    .await;
    stop_project_trust_monitor(trust_monitor).await;
    let (terminal, mut needs_attention, mut stream_trace_complete) = match stream_outcome {
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
    let terminal_status = terminal_run_status(&terminal);
    if matches!(terminal_status, RunStatus::Cancelled | RunStatus::Error) {
        reject_pending_approvals(&record, &state_store.index).await;
        reject_pending_inputs(&record, &state_store.index).await;
    }

    let mut final_candidate = matches!(
        &terminal,
        StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            ..
        }
    ) && !needs_attention
        && stream_trace_complete;
    let _control_lifecycle = record.control_lifecycle_lock.lock().await;
    if final_candidate {
        match drop_unapplied_product_steers_before_terminal(
            product_turn.as_ref(),
            &record,
            &run.trace_writer,
            &mut recorder,
            &state_store,
        )
        .await
        {
            Ok(trace_complete) => {
                stream_trace_complete &= trace_complete;
                if !stream_trace_complete {
                    final_candidate = false;
                    needs_attention = true;
                }
            }
            Err(error) => {
                tracing::warn!(
                    job_id = %record.job_id,
                    run_id = %record.run_id,
                    "failed to close unapplied steers before product terminal: {error}"
                );
                final_candidate = false;
                needs_attention = true;
                stream_trace_complete = false;
            }
        }
    }
    if !final_candidate {
        finish_nonfinal_product_turn(
            product_turn.clone(),
            &terminal,
            needs_attention || !stream_trace_complete,
            &record,
            &run.trace_writer,
            &mut recorder,
            &state_store,
        )
        .await;
    }

    let trace_persisted = persist_terminal_and_finalize(
        &run.trace_writer,
        &record,
        terminal.clone(),
        &mut recorder,
        &state_store,
        &engine,
        &run,
    )
    .await;
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

    let claimed_followup = if final_candidate && runtime_durable {
        finish_final_product_turn(product_turn.clone(), record.run_id).await
    } else if final_candidate {
        // The terminal trace is already incomplete, so do not append late
        // lifecycle events after it. Persist the conservative store state;
        // transcript projection will expose the durable-artifact failure.
        finish_product_turn_needs_attention(
            product_turn.clone(),
            Some(record.run_id),
            "run completed with incomplete terminal artifacts",
        )
        .await;
        None
    } else {
        None
    };

    if let (Some(psid), Some(claim)) = (record.product_session_id.clone(), claimed_followup) {
        schedule_claimed_followup_start(&state, psid, claim);
    }

    // The terminal event is the public lifecycle barrier. A client that sees
    // it must never observe the old product turn still claimed, so publish
    // only after ProductStore has released or atomically replaced that claim.
    publish_terminal_event(&record, terminal).await;
}

struct ProjectTrustMonitor {
    stop: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}

async fn start_project_trust_monitor(
    state: &ApiState,
    record: &Arc<JobRecord>,
) -> Option<ProjectTrustMonitor> {
    let authority = state.project_trust().ok()?;
    let provider_selector = if let (Some(store), Some(product_session_id)) =
        (&record.product_store, &record.product_session_id)
    {
        let catalog = state.provider_catalog().await.ok()?;
        match store.get_session_context(product_session_id).await {
            Ok(context) => product::trust::product_provider_capability_selector(
                store,
                &catalog,
                &context.workspace.id,
                &record.workspace.root,
            )
            .await
            .ok()?,
            Err(error) => {
                tracing::warn!(
                    job_id = %record.job_id,
                    "project trust monitor could not resolve the product session: {error}"
                );
                return None;
            }
        }
    } else {
        rove_app_bootstrap::provider_capability_selector_for_workspace(&record.workspace.root)
    };
    let digests = rove_app_bootstrap::capability_digest_map(
        &record.workspace.root,
        None,
        Some(&provider_selector),
    );
    let initial = match authority.resolve(
        &record.workspace.root,
        record.workspace.kind.clone(),
        &digests,
    ) {
        Ok(resolution) => resolution,
        Err(error) => {
            tracing::warn!(
                job_id = %record.job_id,
                "project trust monitor could not read its initial state: {error}"
            );
            return None;
        }
    };
    let initially_trusted = initial.state == ProjectActivationState::Trusted;
    let initially_granted = initial.granted_capabilities;
    let root = record.workspace.root.clone();
    let kind = record.workspace.kind.clone();
    let cancel = record.cancel_token.clone();
    let job_id = record.job_id;
    let stop = CancellationToken::new();
    let monitor_stop = stop.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The initial state was captured above; wait one interval before the
        // first comparison instead of performing an immediate duplicate read.
        interval.tick().await;
        loop {
            tokio::select! {
                _ = monitor_stop.cancelled() => break,
                _ = interval.tick() => {
                    let resolution = match authority.resolve(&root, kind.clone(), &digests) {
                        Ok(resolution) => resolution,
                        Err(error) => {
                            tracing::warn!(job_id = %job_id, "project trust monitor failed closed: {error}");
                            cancel.cancel();
                            break;
                        }
                    };
                    let revoked = resolution.state == ProjectActivationState::Revoked;
                    let trusted_authority_lost = initially_trusted
                        && (resolution.state != ProjectActivationState::Trusted
                            || !initially_granted.is_subset(&resolution.granted_capabilities));
                    if revoked || trusted_authority_lost {
                        tracing::warn!(job_id = %job_id, "project trust changed while the job was active; cancelling the run");
                        cancel.cancel();
                        break;
                    }
                }
            }
        }
    });
    Some(ProjectTrustMonitor { stop, handle })
}

async fn stop_project_trust_monitor(monitor: Option<ProjectTrustMonitor>) {
    let Some(ProjectTrustMonitor { stop, mut handle }) = monitor else {
        return;
    };
    stop.cancel();
    if tokio::time::timeout(std::time::Duration::from_secs(1), &mut handle)
        .await
        .is_err()
    {
        handle.abort();
    }
}

async fn consume_job_stream(
    record: &JobRecord,
    engine: &Engine,
    run: &RunHandle,
    state_store: &StateStore,
    recorder: &mut RunArtifactRecorder,
    startup_events: Vec<StreamEvent>,
) -> Option<(StreamEvent, bool)> {
    let request = run.request(record.message.clone(), record.resume_state.clone());
    let mut stream =
        std::pin::pin!(engine.run_with_cancel(request, None, record.cancel_token.clone(),));
    recorder.set_runtime_identity(stream.runtime_identity().clone());
    recorder.set_agent_profile(stream.agent_profile().cloned());
    // Capture the control handle so HTTP steer handlers can inject mid-run.
    {
        let _control_lifecycle = record.control_lifecycle_lock.lock().await;
        let handle = stream.control().clone();
        *record.control.lock().await = Some(handle);
        // A request can persist a steer after the product turn is claimed but
        // before this supervisor installs the in-memory handle. Replay the
        // bounded durable queue while holding the same lifecycle lock used by
        // route delivery, so no control can arrive between replay and the
        // live-handle handoff.
        replay_pending_product_steers(record).await;
    }
    let mut terminal = None;
    let mut protocol_invalid = false;
    let mut trace_complete = true;
    let mut saw_run_started = false;
    let mut startup_events = Some(startup_events);
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
        if !saw_run_started {
            if !matches!(&event, StreamEvent::RunStarted { .. }) {
                protocol_invalid = true;
                continue;
            }
            saw_run_started = true;
        }
        let is_run_started = matches!(&event, StreamEvent::RunStarted { .. });
        // Reflect control lifecycle events back into ProductStore before the
        // canonical event is persisted and published.
        reflect_control_event(record, &event).await;
        trace_complete &= persist_record_and_publish_runtime_event(
            &run.trace_writer,
            record,
            event,
            recorder,
            state_store,
        )
        .await;
        if is_run_started {
            // Only expose the trace to concurrent API-originated control
            // events after its mandatory first event has been persisted.
            let _control_lifecycle = record.control_lifecycle_lock.lock().await;
            *record.control_event_trace.lock().await = Some(run.trace_writer.clone());
            let queued_product_events =
                std::mem::take(&mut *record.pending_product_events.lock().await);
            for queued_event in queued_product_events {
                trace_complete &= persist_record_and_publish_runtime_event(
                    &run.trace_writer,
                    record,
                    queued_event,
                    recorder,
                    state_store,
                )
                .await;
            }
            for startup_event in startup_events.take().unwrap_or_default() {
                // A claimed follow-up becomes applied only after its successor
                // has actually reached the durable run-start boundary. Keep the
                // ProductStore transition coupled to the same canonical event
                // path as ordinary stream events.
                reflect_control_event(record, &startup_event).await;
                trace_complete &= persist_record_and_publish_runtime_event(
                    &run.trace_writer,
                    record,
                    startup_event,
                    recorder,
                    state_store,
                )
                .await;
            }
        }
    }
    // Drop the control handle once the stream ends so further steers are rejected.
    {
        let _control_lifecycle = record.control_lifecycle_lock.lock().await;
        *record.control.lock().await = None;
    }
    if protocol_invalid || !saw_run_started {
        None
    } else {
        terminal.map(|terminal| (terminal, trace_complete))
    }
}

async fn reflect_control_event(record: &JobRecord, event: &StreamEvent) {
    let (Some(product_session_id), Some(store)) = (
        record.product_session_id.as_ref(),
        record.product_store.as_ref(),
    ) else {
        return;
    };
    match event {
        StreamEvent::SteerAccepted { id, .. } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                tracing::warn!(control_id = %id, "steer_accepted id is not a product control id");
                return;
            };
            if let Err(error) = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Accepted,
                    Some(&record.run_id),
                )
                .await
            {
                // Already accepted/applied is fine on replay.
                tracing::debug!(
                    control_id = %control_id,
                    "steer_accepted store transition: {error}"
                );
            }
        }
        StreamEvent::SteerApplied { id } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                tracing::warn!(control_id = %id, "steer_applied id is not a product control id");
                return;
            };
            if let Err(error) = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await
            {
                // Replayed canonical events may encounter the already-applied
                // historical row. They must not mutate a completed fact.
                tracing::debug!(control_id = %control_id, "steer_applied store transition: {error}");
            }
        }
        StreamEvent::SteerDropped { id, .. } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let pending = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Dropped,
                    Some(&record.run_id),
                )
                .await;
            if pending.is_err()
                && let Err(error) = store
                    .transition_control(
                        product_session_id,
                        &control_id,
                        ProductControlStatus::Accepted,
                        ProductControlStatus::Dropped,
                        Some(&record.run_id),
                    )
                    .await
            {
                tracing::debug!(
                    control_id = %control_id,
                    "steer_dropped store transition: {error}"
                );
            }
        }
        StreamEvent::FollowupDequeued { id } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                return;
            };
            // Claim already moved pending→accepted; mark applied once the new run starts.
            if let Err(error) = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await
            {
                tracing::debug!(
                    control_id = %control_id,
                    "followup_dequeued store transition: {error}"
                );
            }
        }
        StreamEvent::FollowupAbandoned { id, .. } => {
            let Ok(control_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Abandoned,
                    None,
                )
                .await;
            let _ = store
                .transition_control(
                    product_session_id,
                    &control_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Abandoned,
                    None,
                )
                .await;
        }
        StreamEvent::MessageInterventionRequested { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Accepted,
                    Some(&record.run_id),
                )
                .await;
        }
        StreamEvent::MessageAppliedCurrentRun { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await;
        }
        StreamEvent::MessageNeedsAttention { id, .. } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let pending = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Abandoned,
                    Some(&record.run_id),
                )
                .await;
            if pending.is_err() {
                let _ = store
                    .transition_control(
                        product_session_id,
                        &message_id,
                        ProductControlStatus::Accepted,
                        ProductControlStatus::Abandoned,
                        Some(&record.run_id),
                    )
                    .await;
            }
        }
        StreamEvent::MessageClaimedSuccessor { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Accepted,
                    ProductControlStatus::Applied,
                    Some(&record.run_id),
                )
                .await;
        }
        StreamEvent::MessageRevoked { id } => {
            let Ok(message_id) = id.parse::<ProductControlId>() else {
                return;
            };
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Pending,
                    ProductControlStatus::Revoked,
                    Some(&record.run_id),
                )
                .await;
            let _ = store
                .transition_control(
                    product_session_id,
                    &message_id,
                    ProductControlStatus::Abandoned,
                    ProductControlStatus::Revoked,
                    Some(&record.run_id),
                )
                .await;
        }
        _ => {}
    }
}

async fn replay_pending_product_steers(record: &JobRecord) {
    let (Some(session_id), Some(store)) = (
        record.product_session_id.as_ref(),
        record.product_store.as_ref(),
    ) else {
        return;
    };
    let pending = match store
        .list_controls(session_id, Some(ProductControlStatus::Pending))
        .await
    {
        Ok(controls) => controls,
        Err(error) => {
            tracing::warn!(job_id = %record.job_id, "failed to load pending steer controls: {error}");
            return;
        }
    };
    let handle = record.control.lock().await.clone();
    let Some(handle) = handle else {
        return;
    };
    for control in pending
        .into_iter()
        .filter(|control| control.kind == ProductControlKind::Steer)
    {
        let unified = store.get_message(session_id, &control.id).await.is_ok();
        let steer = if unified {
            rove_runtime::engine::SteerMessage::for_message(control.id.as_str(), control.content)
        } else {
            rove_runtime::engine::SteerMessage::with_id(control.id.as_str(), control.content)
        };
        if !handle.try_send_steer(steer) {
            tracing::warn!(
                job_id = %record.job_id,
                control_id = %control.id,
                "pending steer could not be replayed into the bounded runtime channel"
            );
            // The persistent pending bound mirrors the runtime channel bound,
            // so this is only possible if a concurrently delivered control
            // filled the channel. Leave this and later controls pending; the
            // next safe-point/attachment pass will classify or deliver them.
            break;
        }
    }
}

/// Start a server-owned queued follow-up drain without nesting it inside the
/// just-completed run supervisor. The product-store CAS makes duplicate
/// scheduler wake-ups harmless.
pub(crate) fn schedule_followup_drain(state: &ApiState, session_id: ProductSessionId) {
    if state.inner.shutdown_token.is_cancelled() {
        return;
    }
    let state = state.clone();
    let job_starts = state.inner.job_starts.clone();
    drop(job_starts.spawn(async move {
        drain_followup_for_session(state, session_id).await;
    }));
}

/// Claim the next pending follow-up (if any) and start it as a fresh durable
/// product run. The store atomically claims both the control and session turn;
/// run-id reservation makes post-reservation interruption conservative.
pub(crate) async fn drain_followup_for_session(state: ApiState, session_id: ProductSessionId) {
    if state.inner.shutdown_token.is_cancelled() {
        return;
    }
    let Ok(store) = state.product_store() else {
        return;
    };
    let claimed = match store.claim_next_followup_turn(&session_id).await {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                product_session_id = %session_id,
                "failed to claim pending follow-up turn: {error}"
            );
            return;
        }
    };
    let Some(claimed) = claimed else {
        return;
    };
    let control_id = claimed.control.id.clone();
    tracing::info!(
        product_session_id = %session_id,
        control_id = %control_id,
        "starting claimed follow-up as a new product run"
    );

    match prepare_followup_job_launch(&state, &session_id, claimed).await {
        Ok(launch) => start_job_supervisor(state, launch).await,
        Err(error) => tracing::warn!(
            product_session_id = %session_id,
            control_id = %control_id,
            "failed to prepare follow-up job: {error:?}"
        ),
    }
}

/// Recover safe queued follow-ups once an API process is serving requests.
/// Stale automatic turn claims are handled synchronously when the store opens:
/// only claims without a reserved runtime run id return to `pending`.
pub(crate) async fn recover_pending_followup_drains(state: ApiState) {
    let Ok(store) = state.product_store() else {
        return;
    };
    let sessions = match store.list_idle_sessions_with_pending_followups().await {
        Ok(sessions) => sessions,
        Err(error) => {
            tracing::warn!("failed to list queued follow-ups for recovery: {error}");
            return;
        }
    };
    for session_id in sessions {
        schedule_followup_drain(&state, session_id);
    }
}

/// Start a follow-up that was already claimed while its previous run was
/// finalizing. It deliberately bypasses the generic drain because the
/// exclusive store claim already exists.
fn schedule_claimed_followup_start(
    state: &ApiState,
    session_id: ProductSessionId,
    claim: ProductFollowupTurnClaim,
) {
    if state.inner.shutdown_token.is_cancelled() || state.inner.job_starts.is_closed() {
        return;
    }
    let state = state.clone();
    let job_starts = state.inner.job_starts.clone();
    drop(job_starts.spawn(async move {
        let control_id = claim.control.id.clone();
        match prepare_followup_job_launch(&state, &session_id, claim).await {
            Ok(launch) => start_job_supervisor(state, launch).await,
            Err(error) => tracing::warn!(
                product_session_id = %session_id,
                control_id = %control_id,
                "failed to prepare atomically claimed follow-up job: {error:?}"
            ),
        }
    }));
}

async fn finish_final_product_turn(
    product_turn: Option<ProductTurnSupervisor>,
    run_id: RunId,
) -> Option<ProductFollowupTurnClaim> {
    let product_turn = product_turn?;
    match product_turn
        .store
        .finish_session_turn_and_claim_followup(&product_turn.claim_id)
        .await
    {
        Ok(claim) => claim,
        Err(error) => {
            tracing::warn!("failed to atomically finish final product turn: {error}");
            finish_product_turn_needs_attention(
                Some(product_turn),
                Some(run_id),
                "final product-turn completion could not be committed",
            )
            .await;
            None
        }
    }
}

/// Close every steer which is still pending or only safe-point accepted before
/// the terminal fact is persisted. The runtime itself emits dropped events for
/// its in-memory channel; this covers the durable race window between that
/// channel closing and the product turn's terminal transaction.
async fn drop_unapplied_product_steers_before_terminal(
    product_turn: Option<&ProductTurnSupervisor>,
    record: &JobRecord,
    trace_writer: &TraceWriter,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
) -> Result<bool, ProductStoreError> {
    let Some(product_turn) = product_turn else {
        return Ok(true);
    };
    let reason = "run completed before the steer reached a model turn";
    let dropped = product_turn
        .store
        .drop_unapplied_steers_for_turn(&product_turn.claim_id, record.run_id, reason)
        .await?;
    let mut trace_complete = true;
    for control in dropped {
        trace_complete &= persist_record_and_publish_runtime_event(
            trace_writer,
            record,
            StreamEvent::SteerDropped {
                id: control.id.to_string(),
                reason: reason.to_string(),
            },
            recorder,
            state_store,
        )
        .await;
    }
    Ok(trace_complete)
}

async fn finish_product_turn_needs_attention(
    product_turn: Option<ProductTurnSupervisor>,
    run_id: Option<RunId>,
    reason: &'static str,
) {
    let Some(product_turn) = product_turn else {
        return;
    };
    if let Err(error) = product_turn
        .store
        .finish_session_turn_and_abandon_pending_controls(
            &product_turn.claim_id,
            run_id,
            ProductSessionStatus::NeedsAttention,
            reason,
        )
        .await
    {
        tracing::warn!("failed to conservatively classify product turn controls: {error}");
    }
}

async fn finish_nonfinal_product_turn(
    product_turn: Option<ProductTurnSupervisor>,
    terminal: &StreamEvent,
    needs_attention: bool,
    record: &JobRecord,
    trace_writer: &TraceWriter,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
) {
    let Some(product_turn) = product_turn else {
        return;
    };
    let terminal_status = terminal_run_status(terminal);
    let status = if needs_attention {
        ProductSessionStatus::NeedsAttention
    } else {
        match terminal_status {
            RunStatus::Error | RunStatus::Interrupted => ProductSessionStatus::Error,
            // Explicit cancellation is a known user decision. The existing
            // product continuation contract permits a later fresh turn from
            // its durable terminal state; queued follow-ups remain abandoned
            // and still require explicit confirmation.
            RunStatus::Cancelled => ProductSessionStatus::Idle,
            RunStatus::Done => ProductSessionStatus::NeedsAttention,
            RunStatus::Init | RunStatus::Running => ProductSessionStatus::NeedsAttention,
        }
    };
    let reason = match terminal_status {
        RunStatus::Done => "run completed without a final answer",
        RunStatus::Cancelled => "run cancelled",
        RunStatus::Error | RunStatus::Interrupted => "run did not complete normally",
        RunStatus::Init | RunStatus::Running => "run ended without a durable terminal",
    };
    let finished = match product_turn
        .store
        .finish_session_turn_and_abandon_pending_controls(
            &product_turn.claim_id,
            Some(record.run_id),
            status,
            reason,
        )
        .await
    {
        Ok(finished) => finished,
        Err(error) => {
            tracing::warn!("failed to atomically close non-final product turn: {error}");
            finish_product_turn_needs_attention(
                Some(product_turn),
                Some(record.run_id),
                "non-final product-turn completion could not be committed",
            )
            .await;
            return;
        }
    };
    for control in finished.dropped_steers {
        let _ = persist_record_and_publish_runtime_event(
            trace_writer,
            record,
            StreamEvent::SteerDropped {
                id: control.id.to_string(),
                reason: reason.to_string(),
            },
            recorder,
            state_store,
        )
        .await;
    }
    for control in finished.abandoned_followups {
        let _ = persist_record_and_publish_runtime_event(
            trace_writer,
            record,
            StreamEvent::FollowupAbandoned {
                id: control.id.to_string(),
                reason: reason.to_string(),
            },
            recorder,
            state_store,
        )
        .await;
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
    let finish_outcome = AssertUnwindSafe(finish_product_turn_needs_attention(
        product_turn,
        Some(record.run_id),
        "job supervisor failed before control completion",
    ))
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

async fn append_trace_event(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    event: &StreamEvent,
) -> bool {
    let _sequence = record.control_event_trace_lock.lock().await;
    append_trace_event_unlocked(trace_writer, record, event)
}

fn append_trace_event_unlocked(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    event: &StreamEvent,
) -> bool {
    if let Err(error) = trace_writer.append(event) {
        tracing::warn!(job_id = %record.job_id, run_id = %record.run_id, "failed to append runtime trace event: {error}");
        return false;
    }
    true
}

/// Persist one runtime-owned event, project it into the resumable artifacts,
/// then make it visible to live SSE consumers. The ordering ensures a client
/// never observes an event that cannot subsequently be replayed from the
/// canonical run index.
async fn persist_record_and_publish_runtime_event(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    event: StreamEvent,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
) -> bool {
    if !append_trace_event(trace_writer, record, &event).await {
        return false;
    }
    recorder.record_event(&event, state_store).await;
    append_job_event(record, event).await;
    true
}

/// Persist the terminal event and finish the artifact projections before the
/// product supervisor settles its turn claim. Publication is deliberately a
/// separate final step: product clients treat the visible terminal event as
/// proof that their session is idle or has an atomically claimed successor.
async fn persist_terminal_and_finalize(
    trace_writer: &TraceWriter,
    record: &JobRecord,
    terminal: StreamEvent,
    recorder: &mut RunArtifactRecorder,
    state_store: &StateStore,
    engine: &Engine,
    run: &RunHandle,
) -> bool {
    let persisted = append_trace_event(trace_writer, record, &terminal).await;
    recorder.record_event(&terminal, state_store).await;
    recorder
        .finalize(
            state_store,
            engine.workspace(),
            engine.model_id(),
            &run.run_dir,
        )
        .await;
    persisted
}

async fn publish_terminal_event(record: &JobRecord, terminal: StreamEvent) {
    append_job_event(record, terminal).await;
    *record.control_event_trace.lock().await = None;
}

/// Queue an API-originated product control event until `run_started` is
/// durable, or persist and publish it immediately when the run trace is live.
/// Callers hold `control_lifecycle_lock`, which also serializes this decision
/// against terminal cleanup.
pub(crate) async fn queue_or_publish_product_control_event(record: &JobRecord, event: StreamEvent) {
    let persisted = {
        let _sequence = record.control_event_trace_lock.lock().await;
        let trace = record.control_event_trace.lock().await.clone();
        match trace {
            Some(trace) => append_trace_event_unlocked(&trace, record, &event),
            None => {
                record.pending_product_events.lock().await.push(event);
                return;
            }
        }
    };
    if persisted {
        append_job_event(record, event).await;
    } else {
        tracing::warn!(
            job_id = %record.job_id,
            run_id = %record.run_id,
            "failed to persist API-originated product control event"
        );
    }
}

async fn assemble_job_engine(
    state: &ApiState,
    message: &str,
    req: &CreateJobRequest,
    record: Arc<JobRecord>,
    approval_policy: ApprovalPolicy,
) -> anyhow::Result<Engine> {
    let mut config = record.config.clone();
    if record.product_session_id.is_some() {
        config.tool.mcp_config_path = config.workspace_bounded_mcp_config_path()?;
    }
    let model_id = record
        .product_model_config
        .as_ref()
        .map(|model_config| model_config.model.clone())
        .or_else(|| req.model.clone())
        .unwrap_or_else(|| config.provider.model.clone());
    let model: Box<dyn ModelClient> = match model_id.as_str() {
        "fake" => Box::new(FakeModelClient::new(format!("fake response: {message}"))),
        "fake-raw" => Box::new(FakeModelClient::with_compatibility_text(
            message.to_string(),
        )),
        _ => build_model_client_with_health(&config, model_id, state.inner.model_health.clone()),
    };

    let workspace = record.workspace.clone();
    let state_store = state_store_for_record(&record);
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

    let engine = build_engine(EngineOptions {
        model,
        workspace: &workspace,
        config: &config,
        max_steps: record
            .product_model_config
            .as_ref()
            .map(|model_config| model_config.max_steps)
            .or(req.max_steps)
            .unwrap_or(config.runtime.max_steps),
        agent_selector: req.agent.clone(),
        approval_policy,
        input_provider: Some(input_provider),
        approval_provider,
        environment: None,
        run_model_snapshot: record.run_model_snapshot.clone(),
    })
    .await?;
    let mcp_config_path = config.workspace_bounded_mcp_config_path()?;
    state.inner.mcp_health.write().await.insert(
        mcp_config_path,
        engine.runtime_identity().mcp_servers.clone(),
    );
    Ok(engine)
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

async fn workspace_and_config_for_product_job(
    state: &ApiState,
    req: &CreateJobRequest,
    product_workspace: &ProductWorkspace,
    store: &Arc<dyn ProductStore>,
    model_config: &ProductSessionModelConfig,
    resume_expected: bool,
) -> Result<(Workspace, AppConfig, RunModelSnapshot), ApiError> {
    if req.model.is_some()
        || req.max_steps.is_some()
        || req.approval.is_some()
        || req.provider.is_some()
        || req.resume.is_some()
    {
        return Err(ApiError::bad_request(
            "product job requests must leave model, reasoning, approval, provider, max_steps, and resume to the server",
        ));
    }
    let workspace = open_product_workspace(product_workspace)?;
    if let Some(requested) = req.workspace.as_ref() {
        validate_product_workspace_hint(requested, product_workspace, &workspace)?;
    }
    let (workspace, mut config) = rebased_workspace_config(state, workspace)?;
    let authority = state.project_trust()?;
    let catalog = state.provider_catalog().await.map_err(|error| {
        if resume_expected {
            ApiError::conflict_with_code(
                ProductErrorCode::ProviderUnavailableForResume.as_str(),
                "the Provider catalog required to resume this session is unavailable",
            )
        } else {
            error
        }
    })?;
    let provider_selector = product::trust::product_provider_capability_selector(
        store,
        &catalog,
        &product_workspace.id,
        &workspace.root,
    )
    .await?;
    let trust = product::trust::resolve_product_workspace_trust(
        &authority,
        &workspace.root,
        workspace.kind.clone(),
        &provider_selector,
    )
    .await?;
    if trust.state == ProjectActivationState::Revoked {
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProjectTrustRequired.as_str(),
            "project trust was revoked for this workspace",
        ));
    }
    config.apply_project_trust_resolution(trust);
    if let Some(profile_id) = model_config.profile_id.as_ref() {
        let provider_identity = store.get_provider_profile(profile_id).await?;
        if provider_identity.provider_type != ProductProviderType::Fake
            && !config.project_capability_allowed(rove_app_bootstrap::CAP_PROVIDER_CREDENTIALS)
        {
            return Err(ApiError::conflict_with_code(
                ProductErrorCode::ProjectTrustRequired.as_str(),
                "project trust must grant provider_credentials before using the selected Provider",
            ));
        }
    }
    let Some(profile_id) = model_config.profile_id.as_ref() else {
        if matches!(model_config.model.as_str(), "fake" | "fake-raw")
            && config
                .provider
                .profiles
                .values()
                .any(|profile| profile.provider_type == "fake")
        {
            config.provider.model = model_config.model.clone();
            let snapshot = RunModelSnapshot {
                profile_id: "programmatic-fake".to_string(),
                provider_type: "fake".to_string(),
                wire_protocol: "fake".to_string(),
                endpoint: String::new(),
                model: model_config.model.clone(),
                reasoning: model_config.reasoning.as_str().to_string(),
                catalog_revision: "programmatic".to_string(),
                safe_config_digest: rove_runtime::context::stable_hash("programmatic-fake"),
            };
            return Ok((workspace, config, snapshot));
        }
        return Err(ApiError::conflict_with_code(
            ProductErrorCode::ProductProviderProfileUnavailable.as_str(),
            "product session has no Provider profile selection; configure ~/.rove/config.toml and select a profile",
        ));
    };
    let catalog_profile_id = product::provider_catalog::catalog_id(profile_id)?;
    let selection = rove_app_bootstrap::ModelSelection {
        profile_id: catalog_profile_id.clone(),
        model: model_config.model.clone(),
        reasoning: model_config.reasoning.as_str().to_string(),
        revision: catalog.revision().to_string(),
    };
    let run_model_snapshot = catalog
        .snapshot(&selection, &workspace.root)
        .map_err(|error| {
            if resume_expected {
                ApiError::conflict_with_code(
                    ProductErrorCode::ProviderUnavailableForResume.as_str(),
                    "the Provider profile required to resume this session is unavailable",
                )
            } else {
                product::provider_catalog::catalog_error(error)
            }
        })?;
    let profile = catalog
        .profile_config(&catalog_profile_id)
        .map_err(|error| {
            if resume_expected {
                ApiError::conflict_with_code(
                    ProductErrorCode::ProviderUnavailableForResume.as_str(),
                    "the Provider profile required to resume this session is unavailable",
                )
            } else {
                product::provider_catalog::catalog_error(error)
            }
        })?
        .clone();
    profile
        .resolve(&workspace.root, true, Some(&model_config.model))
        .map_err(|_| {
            let (code, message) = if resume_expected {
                (
                    ProductErrorCode::ProviderUnavailableForResume.as_str(),
                    "the credential required to resume this session is unavailable",
                )
            } else {
                (
                    ProductErrorCode::ProductProviderProfileUnavailable.as_str(),
                    "the selected Provider credential is unavailable",
                )
            };
            ApiError::conflict_with_code(code, message)
        })?;
    config.provider.active = Some(catalog_profile_id.to_string());
    config.provider.profiles.clear();
    config
        .provider
        .profiles
        .insert(catalog_profile_id.to_string(), profile.clone());
    config.provider.fallback_profiles.clear();
    config.provider.fallback_models.clear();
    config.provider.model = model_config.model.clone();
    let provider_type = profile.provider_type;
    apply_product_reasoning(&mut config, &provider_type, model_config.reasoning)?;
    Ok((workspace, config, run_model_snapshot))
}

fn apply_product_reasoning(
    config: &mut AppConfig,
    provider_type: &str,
    reasoning: ProductReasoningPreference,
) -> Result<(), ApiError> {
    if reasoning == ProductReasoningPreference::Default {
        return Ok(());
    }
    if provider_type != "openai-responses" {
        return Err(ApiError::bad_request(
            "the selected provider does not support reasoning controls; choose default reasoning",
        ));
    }
    let active = config
        .provider
        .active
        .clone()
        .ok_or_else(|| ApiError::bad_request("the selected provider has no active protocol"))?;
    let profile =
        config.provider.profiles.get_mut(&active).ok_or_else(|| {
            ApiError::bad_request("the selected provider protocol is unavailable")
        })?;
    profile.protocol_options = serde_json::json!({
        "reasoning_effort": reasoning.as_str(),
    });
    Ok(())
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
    let mut completion = record.completion.subscribe();
    while !*completion.borrow_and_update() {
        if completion.changed().await.is_err() {
            tracing::warn!(job_id = %record.job_id, "job completion signal closed unexpectedly");
            return;
        }
    }
}

async fn drain_job_supervisors(state: &ApiState) {
    state.inner.job_starts.close();
    state.inner.job_starts.wait().await;
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

pub(crate) fn is_terminal(status: &RunStatus) -> bool {
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
    fn agent_engine_assembly(error: &anyhow::Error) -> Self {
        if let Some(error) = error.downcast_ref::<SelectorError>() {
            return Self {
                status: StatusCode::BAD_REQUEST,
                code: error.code(),
                message: error.to_string(),
            };
        }
        if let Some(error) = error.downcast_ref::<AgentActivationError>() {
            return Self {
                status: if matches!(error, AgentActivationError::WorkspaceSourceNotAuthorized) {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::BAD_REQUEST
                },
                code: error.code(),
                message: error.to_string(),
            };
        }
        Self::internal("failed to assemble job engine")
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
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

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }

    pub(crate) fn not_found_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
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

    pub(crate) fn bad_gateway_with_code(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn too_many_requests_with_code(
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn gateway_timeout_with_code(
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn internal(err: impl std::fmt::Display) -> Self {
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
            ProductErrorCode::ProductNotFound
            | ProductErrorCode::ProductMemoryNotFound
            | ProductErrorCode::ProductMcpNotFound => StatusCode::NOT_FOUND,
            ProductErrorCode::ProductInvalidInput
            | ProductErrorCode::ProductMemoryInvalidSlug
            | ProductErrorCode::ProductMcpInvalidInput
            | ProductErrorCode::ProjectTrustInvalidInput => StatusCode::BAD_REQUEST,
            ProductErrorCode::ProductStoreUnavailable
            | ProductErrorCode::ProjectTrustUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ProductErrorCode::ProductStorageFailure => StatusCode::INTERNAL_SERVER_ERROR,
            ProductErrorCode::ProductSessionActive
            | ProductErrorCode::ProductSessionWorkspaceMismatch
            | ProductErrorCode::ProductSessionResumeConflict
            | ProductErrorCode::ProductSessionRuntimeStateMissing
            | ProductErrorCode::ProductSessionRuntimeStateCorrupt
            | ProductErrorCode::ProductBindingCorrupt
            | ProductErrorCode::ProductRevisionConflict
            | ProductErrorCode::ProductMemoryConflict
            | ProductErrorCode::ProductMcpConflict
            | ProductErrorCode::ProjectTrustRequired
            | ProductErrorCode::MigrationIdempotencyConflict
            | ProductErrorCode::ProductControlConflict
            | ProductErrorCode::ProductControlRejected
            | ProductErrorCode::ProductForkConflict
            | ProductErrorCode::ProductForkSourceInvalid
            | ProductErrorCode::ProductSessionModelConfigConflict
            | ProductErrorCode::ProviderUnavailableForResume
            | ProductErrorCode::ProviderChangedForResume => StatusCode::CONFLICT,
            ProductErrorCode::ProductProviderProfileUnavailable => StatusCode::NOT_FOUND,
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
            product_session_id: None,
            product_store: None,
            product_model_config: None,
            run_model_snapshot: None,
            status: Mutex::new(RunStatus::Running),
            events: Mutex::new(Vec::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            pending_inputs: Mutex::new(HashMap::new()),
            tx,
            handle: Mutex::new(None),
            control: Mutex::new(None),
            control_event_trace: Mutex::new(None),
            control_event_trace_lock: Mutex::new(()),
            control_lifecycle_lock: Mutex::new(()),
            pending_product_events: Mutex::new(Vec::new()),
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
    async fn completion_waiter_waits_for_registration_gap_to_close() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let (_state, record, _) = test_job_record(&temp_dir).await;
        assert!(record.handle.lock().await.is_none());

        let waiting_record = Arc::clone(&record);
        let waiter = tokio::spawn(async move {
            wait_for_job_completion(&waiting_record).await;
        });
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "a live record must not look complete merely because its supervisor is registering"
        );

        record.completion.send_replace(true);
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("completion waiter should observe the durable completion signal")
            .unwrap();
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

        let emitted = stream.next().await.expect("terminal event");
        assert_eq!(emitted.seq, terminal.seq);
        assert!(matches!(
            emitted.event,
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                ..
            }
        ));
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
            product_session_id: None,
            product_store: None,
            product_model_config: None,
            run_model_snapshot: None,
            status: Mutex::new(RunStatus::Running),
            events: Mutex::new(Vec::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            pending_inputs: Mutex::new(HashMap::new()),
            tx,
            handle: Mutex::new(None),
            control: Mutex::new(None),
            control_event_trace: Mutex::new(None),
            control_event_trace_lock: Mutex::new(()),
            control_lifecycle_lock: Mutex::new(()),
            pending_product_events: Mutex::new(Vec::new()),
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
            product_session_id: None,
            product_store: None,
            product_model_config: None,
            run_model_snapshot: None,
            status: Mutex::new(RunStatus::Running),
            events: Mutex::new(Vec::new()),
            pending_approvals: Mutex::new(HashMap::new()),
            pending_inputs: Mutex::new(HashMap::new()),
            tx,
            handle: Mutex::new(None),
            control: Mutex::new(None),
            control_event_trace: Mutex::new(None),
            control_event_trace_lock: Mutex::new(()),
            control_lifecycle_lock: Mutex::new(()),
            pending_product_events: Mutex::new(Vec::new()),
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
        assert!(state.inner.job_starts.is_closed());
        assert!(state.inner.job_starts.is_empty());
        assert!(state.inner.supervisors.is_closed());
        assert!(state.inner.supervisors.is_empty());
        let first_handle = first.handle.lock().await.take().unwrap();
        first_handle.await.unwrap();
        assert!(second.handle.lock().await.is_none());
    }

    #[tokio::test]
    async fn dropped_job_start_waiter_does_not_cancel_a_blocked_product_claim() {
        let server = tempfile::TempDir::new().unwrap();
        let workspace_root = server.path().join("product-workspace");
        std::fs::create_dir_all(&workspace_root).unwrap();
        let workspace = Workspace::detect(server.path()).unwrap();
        let mut config = AppConfig::default();
        config.state.state_dir = PathBuf::from("api-state");
        config.state.sqlite_busy_timeout_ms = 5_000;
        let user_paths = UserConfigPaths::from_root(server.path().join("user-config"));
        let mut user_document = rove_app_bootstrap::UserConfigDocument::default();
        user_document.provider.profiles.insert(
            "test-fake".to_string(),
            config.provider.profiles["default"].clone(),
        );
        user_document.model.default_profile = Some("test-fake".to_string());
        user_document.model.default_model = Some("fake".to_string());
        rove_app_bootstrap::UserConfigWriter::new(user_paths.clone())
            .update(None, &user_document)
            .unwrap();
        config.source_summary.user_config_path = user_paths.config_file;
        let state = ApiState::new(workspace, config);
        let store = state.product_store().unwrap();
        let product_workspace = store
            .create_workspace(CreateProductWorkspaceRequest {
                root: workspace_root,
                kind: ProductWorkspaceKind::Folder,
                display_name: Some("Tracked start".to_string()),
                pinned: false,
            })
            .await
            .unwrap();
        let product_session = store
            .create_session(CreateProductSessionRequest {
                workspace_id: product_workspace.id.clone(),
                title: Some("Disconnect during claim".to_string()),
            })
            .await
            .unwrap();
        let test_profile_id = ProductProviderProfileId::from_catalog_id("test-fake").unwrap();
        store
            .upsert_provider_catalog_identity(
                &test_profile_id,
                "Test Fake",
                ProductProviderType::Fake,
                &user_document.revision(),
            )
            .await
            .unwrap();
        store
            .update_session_model_config(
                &product_session.id,
                UpdateProductSessionModelConfigRequest {
                    profile_id: Some(test_profile_id),
                    model: "fake".to_string(),
                    reasoning: ProductReasoningPreference::Default,
                    max_steps: DEFAULT_PRODUCT_MAX_STEPS,
                    expected_revision: None,
                },
            )
            .await
            .unwrap();

        let blocker = rusqlite::Connection::open(state.product_store_path()).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let response = start_tracked_job(
            state.clone(),
            CreateJobRequest {
                message: "finish after the response waiter disconnects".to_string(),
                model: None,
                max_steps: None,
                agent: None,
                approval: None,
                resume: None,
                workspace: None,
                provider: None,
                product_session_id: Some(product_session.id.clone()),
            },
        );
        assert_eq!(state.inner.job_starts.len(), 1);
        tokio::time::sleep(Duration::from_millis(25)).await;
        drop(response);
        assert_eq!(
            state.inner.job_starts.len(),
            1,
            "dropping the HTTP response waiter must not cancel the owned start task"
        );

        blocker.execute_batch("ROLLBACK").unwrap();
        drop(blocker);
        tokio::time::timeout(Duration::from_secs(5), drain_job_supervisors(&state))
            .await
            .expect("tracked job start and supervisor should drain after the database unlock");

        let sessions = store.list_sessions(&product_workspace.id).await.unwrap();
        let session = sessions
            .into_iter()
            .find(|session| session.id == product_session.id)
            .expect("product session");
        assert_eq!(session.status, ProductSessionStatus::Idle);
        assert!(session.runtime_binding.is_some());
        assert_eq!(
            store
                .list_run_bindings(&product_session.id)
                .await
                .unwrap()
                .len(),
            1
        );
        let claim = store.claim_session_turn(&product_session.id).await.unwrap();
        store
            .finish_session_turn(&claim.claim_id, claim.previous_status)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn shutdown_drains_job_starts_before_supervisors() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp_dir.path()).unwrap();
        let state = ApiState::new(workspace, AppConfig::default());
        let supervisors = state.inner.supervisors.clone();
        let (start_entered_tx, start_entered_rx) = oneshot::channel();
        let (release_start_tx, release_start_rx) = oneshot::channel();
        let (supervisor_entered_tx, supervisor_entered_rx) = oneshot::channel();
        let (release_supervisor_tx, release_supervisor_rx) = oneshot::channel();
        drop(state.inner.job_starts.spawn(async move {
            let _ = start_entered_tx.send(());
            let _ = release_start_rx.await;
            drop(supervisors.spawn(async move {
                let _ = supervisor_entered_tx.send(());
                let _ = release_supervisor_rx.await;
            }));
        }));
        start_entered_rx.await.unwrap();

        let drain_state = state.clone();
        let drain = tokio::spawn(async move {
            drain_job_supervisors(&drain_state).await;
        });
        tokio::task::yield_now().await;
        assert!(state.inner.job_starts.is_closed());
        assert!(!state.inner.supervisors.is_closed());
        assert!(!drain.is_finished());

        release_start_tx.send(()).unwrap();
        supervisor_entered_rx.await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while !state.inner.supervisors.is_closed() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("supervisor tracker should close after all job starts drain");
        assert!(!drain.is_finished());

        release_supervisor_tx.send(()).unwrap();
        drain.await.unwrap();
        assert!(state.inner.job_starts.is_empty());
        assert!(state.inner.supervisors.is_empty());
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
