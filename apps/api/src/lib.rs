use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::panic::AssertUnwindSafe;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::rejection::JsonRejection;
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

use rove_app_bootstrap::{
    AppConfig, AppConfigOverrides, ProjectActivationState, ProjectTrustRepository, ProviderCatalog,
    ProviderCatalogService, UserConfigPaths,
};
use rove_app_bootstrap::{EngineOptions, ReviewEngineOptions, build_engine, build_review_engine};
use rove_app_bootstrap::{build_model_client_with_health, model_supports_images};
use rove_core::ToolError;
use rove_models::ModelClient;
use rove_models::fake::{FakeModelClient, FakeTurn};
use rove_models::health::{HealthConfig, ModelHealthStore};
use rove_runtime::agents::{AgentActivationError, SelectorError};
use rove_runtime::engine::{Engine, RunControlHandle};
use rove_runtime::events::StreamEvent;
use rove_runtime::review::{
    ReviewRuntimeEvidence, ReviewTargetSnapshot, apply_runtime_outcome, capture_target,
    finalize_result_with_evidence,
};
use rove_runtime::runtime_identity::RunModelSnapshot;
use rove_runtime::session::SessionEntry;
use rove_runtime::state::artifacts::RunArtifactRecorder;
use rove_runtime::state::index::{ResumeJobClaim, RunIndexRecord, StateIndex};
use rove_runtime::state::resume::resolve_resume_state;
use rove_runtime::state::store::{RunHandle, StateStore};
use rove_runtime::state::trace::TraceWriter;
use rove_runtime::tools::mcp_proxy::McpServerRuntimeSnapshot;
use rove_runtime::tools::review::ReviewSubmissionStore;
use rove_runtime::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, Message, PendingToolApproval,
    PendingUserInput, Role, RunId, RunMode, RunStatus, SessionId, TaskState, TerminationReason,
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
mod web;

mod assembly;
mod error;
mod events;
mod followup;
mod fork;
mod jobs;
mod launch;
mod review;
mod state;
mod supervisor;

use assembly::*;
use error::*;
use events::*;
use followup::*;
use fork::*;
use jobs::*;
use launch::*;
use review::*;
use supervisor::*;

use benchmark::BenchState;
pub use product::*;
pub use types::*;
pub use web::console_root as web_console_root;

use provider::{
    apply_provider_profile, normalize_provider_profile, provider_inventory, provider_key_env,
};

const EVENT_BUFFER: usize = 256;
/// Nudge capacity for `/product/events`.
///
/// A nudge carries no payload: every listener re-reads the durable event log
/// from its own cursor, so a lagged or dropped nudge costs one poll interval of
/// latency and never correctness. The capacity only decides how many wake-ups
/// can queue before a slow listener starts lagging.
const PRODUCT_EVENT_NOTIFY_BUFFER: usize = 64;
pub(crate) const PRODUCT_MIGRATION_PREPARATION_DEADLINE: Duration = Duration::from_secs(30);

/// Total start attempts one queued successor is allowed per process: the initial
/// claim plus three re-drains. A successor that still cannot start is abandoned
/// to the visible `needs_attention` state instead of being retried forever.
///
/// The budget lives in this process, so a restart re-arms it. That is deliberate:
/// the queue itself is durable, boot recovery re-drains it, and a restart is
/// itself a change to the process state the failure may have depended on.
const FOLLOWUP_START_MAX_ATTEMPTS: u32 = 4;
/// Wait before the first re-drain. It doubles per failed attempt (1s, 2s, 4s),
/// so the whole window is about seven seconds: long enough for a transient
/// store, workspace, or catalog failure to clear, short enough that a persistent
/// misconfiguration becomes visible in the same interactive session.
const FOLLOWUP_REDRIVE_INITIAL_BACKOFF: Duration = Duration::from_millis(1_000);
/// Hard ceiling on a single re-drain delay, so a larger attempt budget cannot
/// turn into an unbounded wait.
const FOLLOWUP_REDRIVE_MAX_BACKOFF: Duration = Duration::from_secs(4);
/// Upper bound on successors tracked for re-draining at once. The count is
/// forgotten as soon as a successor starts or escalates, so this only binds
/// under churn.
const FOLLOWUP_REDRIVE_MAX_TRACKED: usize = 256;

#[derive(Clone)]
pub struct ApiState {
    inner: Arc<ApiStateInner>,
}

struct ApiStateInner {
    workspace: Workspace,
    config: AppConfig,
    product_store_path: PathBuf,
    product_store: Option<Arc<dyn ProductStore>>,
    /// Session-scoped attachment payloads, a sibling of `product.sqlite` under
    /// the same pinned user-data root and therefore never inside a workspace.
    attachment_storage: product::attachments::AttachmentStorage,
    /// Process-wide upload slots. Each in-flight upload holds up to 20 MiB and a
    /// file handle, so this is a hard ceiling rather than a queue.
    attachment_uploads: Arc<tokio::sync::Semaphore>,
    provider_catalog: ProviderCatalogService,
    project_trust: Option<Arc<ProjectTrustRepository>>,
    product_transcript_reader: Option<Arc<dyn ProductTranscriptReader>>,
    preview: product::preview::PreviewRegistry,
    shutdown_token: CancellationToken,
    /// Wakes open `/product/events` streams after a product mutation committed.
    product_events: broadcast::Sender<()>,
    job_starts: TaskTracker,
    supervisors: TaskTracker,
    /// Failed start attempts per queued successor control, for this process only.
    /// A successor that starts or escalates is forgotten here.
    followup_start_attempts: Mutex<HashMap<ProductControlId, u32>>,
    jobs: RwLock<HashMap<JobId, Arc<JobRecord>>>,
    review_jobs: RwLock<HashMap<JobId, Arc<ReviewExecution>>>,
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
    /// The image content blocks the run's user message projects. Empty for
    /// every run whose message references no image, and never re-read on
    /// resume: a resumed run replays the blocks its trace already carries.
    content_blocks: Vec<rove_models::ContentBlock>,
    resumed_from_run_id: Option<RunId>,
    resume_state: Option<TaskState>,
    /// Product session this job is bound to (if any). Used for steer delivery
    /// and follow-up drain after terminal.
    pub(crate) product_session_id: Option<ProductSessionId>,
    product_store: Option<Arc<dyn ProductStore>>,
    /// The payload root this job resolves a message's attachments from.
    ///
    /// Held here rather than reached through `ApiState` because the steer replay
    /// runs inside the run supervisor, which owns a `JobRecord` and nothing else.
    /// A job bound to no product session never reads it.
    attachment_storage: Option<product::attachments::AttachmentStorage>,
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
struct ReviewExecution {
    review_id: ProductReviewId,
    snapshot: Arc<ReviewTargetSnapshot>,
    submission_store: ReviewSubmissionStore,
    product_store: Arc<dyn ProductStore>,
    started_at: Instant,
    state_root: PathBuf,
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
    // Composing the OpenApiRouter walks the whole schema tree more than once —
    // materializing the document, then again while merging each route's
    // components — and that recursion overflows the 1 MiB stack Windows gives
    // the binary's main thread. Compose on a dedicated thread with a generous
    // stack so the rove-api binary, the embedded desktop server, and tests all
    // share the fix. The runtime-dependent followup recovery stays above: the
    // composition thread carries no tokio context.
    let compose_state = state.clone();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("rove-router-build".into())
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(scope, move || router_composed(compose_state))
            .expect("failed to spawn the router composition thread")
            .join()
            .expect("the router composition thread panicked")
    })
}

fn router_composed(state: ApiState) -> Router {
    let migration_router: OpenApiRouter<ApiState> = OpenApiRouter::new()
        .routes(routes!(product::routes::migrate_m1_browser_state))
        .route_layer(DefaultBodyLimit::max(MAX_M1_BROWSER_MIGRATION_BODY_BYTES));
    // The upload body is raw bytes rather than JSON, so it gets its own
    // route-scoped limit: an oversized `Content-Length` is refused before the
    // body is read, and the permit taken here bounds concurrent uploads from
    // the first byte rather than from the first write.
    let attachment_router: OpenApiRouter<ApiState> = OpenApiRouter::new()
        .routes(routes!(
            product::attachments::upload_product_session_attachment
        ))
        .route_layer(DefaultBodyLimit::max(
            product::MAX_PRODUCT_ATTACHMENT_UPLOAD_BODY_BYTES,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            product::attachments::guard_attachment_upload,
        ));
    // Reads share the route but not the upload guard: a download must not
    // consume an upload slot or inherit the upload deadline.
    let attachment_read_router: OpenApiRouter<ApiState> = OpenApiRouter::new().routes(routes!(
        product::attachments::get_product_session_attachment
    ));
    // Materializing the document walks the whole schema tree (StreamEvent's
    // event-kind union alone nests several levels of payload schemas), which
    // overflows the 1 MiB stack Windows hands to the binary's main thread.
    // Build it on a dedicated thread with a generous stack so every caller —
    // the rove-api binary, the embedded desktop server, and tests — shares
    // the fix.
    let (api_router, api) = OpenApiRouter::with_openapi(docs::ApiDoc::openapi())
        .routes(routes!(list_provider_models))
        .routes(routes!(test_provider))
        .routes(routes!(product::routes::list_product_workspaces))
        .routes(routes!(product::routes::create_product_workspace))
        .routes(routes!(
            product::workspace_picker::pick_product_workspace_folder
        ))
        .routes(routes!(product::routes::delete_product_workspace))
        .routes(routes!(product::routes::list_product_sessions))
        .routes(routes!(product::routes::create_product_session))
        .routes(routes!(product::routes::create_product_session_fork))
        .routes(routes!(product::routes::list_product_session_forks))
        .routes(routes!(product::routes::update_product_session))
        .routes(routes!(product::routes::delete_product_session))
        .routes(routes!(product::routes::get_product_session_transcript))
        .routes(routes!(product::review::create_product_review))
        .routes(routes!(product::review::list_product_reviews))
        .routes(routes!(product::review::get_product_review))
        .routes(routes!(product::review::list_product_review_findings))
        .routes(routes!(product::review::cancel_product_review))
        .routes(routes!(product::routes::get_product_session_model_config))
        .routes(routes!(
            product::routes::update_product_session_model_config
        ))
        .routes(routes!(product::routes::list_product_session_run_models))
        .routes(routes!(product::usage::get_product_session_usage))
        .routes(routes!(product::compaction::compact_product_session))
        .routes(routes!(product::files::list_workspace_files))
        .routes(routes!(product::files::get_workspace_file_content))
        .routes(routes!(product::files::download_workspace_file))
        .routes(routes!(product::files::preview_workspace_file))
        .routes(routes!(product::preview::create_product_preview))
        .routes(routes!(product::preview::close_product_preview))
        .routes(routes!(product::artifacts::list_session_artifacts))
        .routes(routes!(product::artifacts::get_artifact_content))
        .routes(routes!(product::artifacts::download_artifact))
        .routes(routes!(product::artifacts::preview_artifact))
        .routes(routes!(product::diff::get_session_diff))
        .routes(routes!(
            product::authorizations::list_product_session_authorizations
        ))
        .routes(routes!(product::export::export_product_session))
        .routes(routes!(product::routes::list_product_provider_profiles))
        .routes(routes!(product::routes::create_product_provider_profile))
        .routes(routes!(product::routes::update_product_provider_profile))
        .routes(routes!(product::routes::delete_product_provider_profile))
        .routes(routes!(product::routes::list_product_provider_models))
        .routes(routes!(
            product::provider_onboarding::onboard_product_provider
        ))
        .routes(routes!(product::routes::get_product_preferences))
        .routes(routes!(product::routes::update_product_preferences))
        .routes(routes!(product::routes::create_product_session_steer))
        .routes(routes!(product::routes::create_product_session_followup))
        .routes(routes!(product::routes::create_product_session_message))
        .routes(routes!(product::routes::list_product_session_messages))
        .routes(routes!(product::routes::search_product_session_messages))
        .routes(routes!(product::routes::search_product))
        .routes(routes!(product::routes::promote_product_session_message))
        .routes(routes!(product::routes::revoke_product_session_message))
        .routes(routes!(product::routes::reorder_product_session_messages))
        .routes(routes!(product::routes::list_product_session_controls))
        .routes(routes!(product::routes::revoke_product_session_control))
        .routes(routes!(product::routes::confirm_product_session_followup))
        .routes(routes!(product::routes::product_events))
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
        .merge(attachment_router)
        .merge(attachment_read_router)
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

/// Like [`router`], but additionally serves the Web console's static bundle
/// at the origin root and mounts the same API under `/api` so the bundle's
/// same-origin calls resolve without a proxy. See [`web::with_console`].
pub fn router_with_web(state: ApiState, web_root: &FsPath) -> anyhow::Result<Router> {
    web::with_console(router(state), web_root)
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

pub async fn serve(
    addr: Option<SocketAddr>,
    cwd: PathBuf,
    web_root: Option<PathBuf>,
) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    tokio::spawn(async move {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!("failed to listen for Ctrl+C: {err}");
        }
        signal_shutdown.cancel();
    });
    serve_with_shutdown(addr, cwd, shutdown, web_root).await
}

pub async fn serve_with_shutdown(
    addr: Option<SocketAddr>,
    cwd: PathBuf,
    shutdown: CancellationToken,
    web_root: Option<PathBuf>,
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
    if config.state_dir_is_contract_managed() {
        config.ensure_contract_layout()?;
    }
    rove_app_bootstrap::ensure_home_legacy_run_migration(workspace.root.as_path());
    let addr: SocketAddr = config.api.bind_addr.parse()?;
    let state = ApiState::with_shutdown(workspace, config, shutdown.clone());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let app = match web_root.as_deref() {
        Some(root) => {
            let app = router_with_web(state.clone(), root)?;
            tracing::info!(web_root = %root.display(), %addr, "serving the web console");
            if state.inner.config.api.token_auth.is_some() {
                // The bundle itself is public; the API still demands Bearer.
                // There is no browser credential hand-off yet, so a served
                // console under token auth cannot call its own API.
                tracing::warn!(
                    "api.token_auth is set: the served web console cannot authenticate; \
                     serve without a token or use the desktop/next proxy path"
                );
            }
            app
        }
        None => router(state.clone()),
    };
    serve_state_app(listener, state, app).await
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
            data_root: Some(state_dir.clone()),
            ..AppConfigOverrides::default()
        },
    )?;
    config.api.bind_addr = bind_addr.to_string();
    config.api.token_auth = Some(bearer_token);
    config.api.cors_origins = cors_origins;
    config.source_summary.workspace_root = cwd.to_path_buf();
    config.source_summary.project_config_path = cwd.join(".rove/config.toml");

    let mut workspace = Workspace::detect(cwd)?;
    workspace.state_dir = config.state_dir();
    workspace.ensure_state_dir()?;
    config.ensure_contract_layout()?;
    rove_app_bootstrap::ensure_home_legacy_run_migration(cwd);
    Ok(ApiState::with_shutdown(workspace, config, shutdown))
}

/// Serve an already-assembled API state and perform the same complete shutdown
/// drain used by the standalone API binary.
pub async fn serve_state_listener(
    listener: tokio::net::TcpListener,
    state: ApiState,
) -> anyhow::Result<()> {
    let app = router(state.clone());
    serve_state_app(listener, state, app).await
}

/// Shared tail of the serve paths: preview listener, graceful shutdown, and
/// the supervisor drain, identical no matter which router was assembled.
async fn serve_state_app(
    listener: tokio::net::TcpListener,
    state: ApiState,
    app: Router,
) -> anyhow::Result<()> {
    let shutdown = state.inner.shutdown_token.clone();
    spawn_preview_listener(&state).await;
    let result = serve_listener(listener, app, shutdown).await;
    state.inner.shutdown_token.cancel();
    drain_job_supervisors(&state).await;
    result
}

/// Bind the isolated preview origin on an ephemeral loopback port and serve
/// it until shutdown (plan P5b). In-process hosts and integration tests call
/// this once per state before creating preview sessions. A bind failure is
/// recorded on the registry so the product create route answers with a typed
/// `product_preview_unavailable` 503 instead of pretending previews work.
pub async fn spawn_preview_listener(state: &ApiState) {
    match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => {
            let addr = match listener.local_addr() {
                Ok(addr) => addr,
                Err(error) => {
                    state
                        .inner
                        .preview
                        .record_listener_unavailable(format!(
                            "preview listener address is unavailable: {error}"
                        ))
                        .await;
                    return;
                }
            };
            state.inner.preview.record_listener_bound(addr).await;
            let registry = state.inner.preview.clone();
            let shutdown = state.inner.shutdown_token.clone();
            state.inner.supervisors.spawn(async move {
                if let Err(error) = serve_listener(
                    listener,
                    product::preview::preview_router(registry),
                    shutdown,
                )
                .await
                {
                    tracing::warn!("preview listener failed: {error}");
                }
            });
        }
        Err(error) => {
            tracing::warn!("preview listener could not bind: {error}");
            state
                .inner
                .preview
                .record_listener_unavailable(format!("preview listener could not bind: {error}"))
                .await;
        }
    }
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

#[cfg(test)]
mod tests;
