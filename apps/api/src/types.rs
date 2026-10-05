//! Request and response DTOs for the HTTP API.
//!
//! These are the serializable contract types exchanged over `/jobs`, `/runs`,
//! and the provider endpoints. They are deliberately free of server state so the
//! OpenAPI schema (`docs.rs`) and integration tests can depend on them without
//! reaching into the handler internals in [`super`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use rove_runtime::events::StreamEvent;
use rove_runtime::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, RunId, RunStatus, SessionId,
};

use crate::product::ProductSessionId;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceActivationState {
    Unknown,
    #[default]
    Restricted,
    Trusted,
    Revoked,
}

impl From<rove_app_bootstrap::ProjectActivationState> for WorkspaceActivationState {
    fn from(value: rove_app_bootstrap::ProjectActivationState) -> Self {
        match value {
            rove_app_bootstrap::ProjectActivationState::Unknown => Self::Unknown,
            rove_app_bootstrap::ProjectActivationState::Restricted => Self::Restricted,
            rove_app_bootstrap::ProjectActivationState::Trusted => Self::Trusted,
            rove_app_bootstrap::ProjectActivationState::Revoked => Self::Revoked,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiErrorResponse {
    /// Stable machine-readable error code.
    pub code: String,
    /// Human-readable safe error summary.
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PendingApprovalResponse {
    #[schema(value_type = String, format = "ulid")]
    pub call_id: CallId,
    pub name: String,
    #[schema(value_type = Object)]
    pub args: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PendingInputResponse {
    #[schema(value_type = String, format = "ulid")]
    pub input_id: CallId,
    pub prompt: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateJobRequest {
    pub message: String,
    pub model: Option<String>,
    pub max_steps: Option<u32>,
    /// Optional fully qualified Agent selector (`builtin:legacy` or
    /// `workspace:<id>`). Workspace sources still require Project Trust.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[schema(value_type = String, example = "ask")]
    pub approval: Option<ApprovalPolicy>,
    pub resume: Option<String>,
    pub workspace: Option<CreateJobWorkspace>,
    pub provider: Option<ProviderProfileRequest>,
    /// Optional server-owned product session. When present, the server resolves
    /// its exact runtime run binding; product callers must not use `latest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_session_id: Option<ProductSessionId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderProfileRequest {
    /// User-facing provider type. Values: `openai`, `openai-responses`,
    /// `anthropic`, `ollama`, `fake`. Official and relay endpoints share the
    /// same type; only `api_base` / key / model differ.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// Optional display label. When empty, the API derives a name from
    /// `api_base` (hostname). Use `provider_type` to select the type, not `name`.
    #[serde(default)]
    pub name: String,
    pub api_base: String,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderTestRequest {
    pub provider: ProviderProfileRequest,
    pub model: Option<String>,
    pub models_endpoint: Option<String>,
}

/// Request body for listing models available on a provider endpoint.
///
/// Requires a typed provider profile (`provider_type` + `api_base`).
/// For OpenAI/Anthropic families the API key is read from `api_key_env` on the
/// server process; Ollama and Fake do not need a key.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderModelsRequest {
    pub provider: ProviderProfileRequest,
    /// Optional override for the models inventory URL. When omitted the API
    /// uses the protocol default (`{api_base}/models`, Anthropic `/v1/models`,
    /// Ollama `/api/tags`).
    pub models_endpoint: Option<String>,
}

/// Per-job workspace binding for create-job.
///
/// - `task`: isolated workspace under `base`/`name` (existing behavior).
/// - `folder` / `repo`: bind tools/state to an absolute local `root`.
///   The opened path is the real execution root (not the API process cwd).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateJobWorkspace {
    #[schema(value_type = String, example = "folder")]
    pub kind: CreateJobWorkspaceKind,
    /// Task workspace name (`kind = task` only).
    pub name: Option<String>,
    /// Task base directory (`kind = task` only). Defaults to
    /// `<server state_dir>/tasks` when omitted.
    #[schema(value_type = String)]
    pub base: Option<PathBuf>,
    /// Absolute local directory for `folder` / `repo` binding.
    #[schema(value_type = String)]
    pub root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CreateJobWorkspaceKind {
    /// Plain local directory execution root.
    Folder,
    /// Local git repository execution root (requires `.git` at `root`).
    Repo,
    /// Isolated standalone task workspace under `base`/`name`.
    Task,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SubmitApprovalRequest {
    #[schema(value_type = String, example = "approve")]
    pub decision: ApprovalDecision,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SubmitInputRequest {
    pub answer: String,
}

pub use rove_product_store::JobStreamEvent;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateJobResponse {
    #[schema(value_type = String, format = "ulid")]
    pub job_id: JobId,
    #[schema(value_type = String, format = "ulid")]
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "ulid")]
    pub resumed_from_run_id: Option<RunId>,
    /// Whether repository-owned project config and MCP activation were enabled.
    #[serde(default)]
    pub workspace_activation: WorkspaceActivationState,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct JobStateResponse {
    #[schema(value_type = String, format = "ulid")]
    pub job_id: JobId,
    #[schema(value_type = String, format = "ulid")]
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "ulid")]
    pub resumed_from_run_id: Option<RunId>,
    #[schema(value_type = String, example = "running")]
    pub status: RunStatus,
    pub event_count: usize,
    pub events: Vec<JobStreamEvent>,
    /// Additive R2b projection of `events`: `true` only when this run's
    /// assistant answer was stopped by the user after it had already published
    /// text, so the text is a salvaged partial rather than a complete response.
    ///
    /// The field is omitted when it does not apply. A `true` value is always
    /// conclusive; the omission is conclusive only for a settled listing, which
    /// requires both of these:
    ///
    /// * `status` is terminal (`done`, `error`, `cancelled`, `interrupted`).
    ///   A run still in flight lists only the events published so far, and its
    ///   salvage message is published when the stopped turn settles — up to the
    ///   runtime's abort-salvage window after the stop — while `status` changes
    ///   only on the terminal event. An in-flight response can therefore omit
    ///   this field for a run that is about to keep a partial.
    /// * `events` is contiguous: its `seq` values run `1..=n` with no gap. A
    ///   persisted response is projected from the run index rather than from
    ///   `trace.jsonl`, so a gap there means the listing is not the run's whole
    ///   event list; the transcript endpoint reports that case explicitly
    ///   instead (`partial`, `missing_event_range`).
    ///
    /// Within those bounds the omission separates a complete answer
    /// (`aborted: false` on its `llm_message`), a turn that failed
    /// (`status: "error"`, no `llm_message`), and a stop that produced nothing
    /// (a `cancelled` run with no `llm_message`). All three still serialize
    /// exactly as they did before this field existed. A run resumed from
    /// `resumed_from_run_id` reports only its own events; the session-wide view
    /// is the transcript, whose segments project each bound run in turn.
    #[serde(default, skip_serializing_if = "is_false")]
    pub answer_aborted: bool,
    pub pending_approvals: Vec<PendingApprovalResponse>,
    pub pending_inputs: Vec<PendingInputResponse>,
}

/// Whether a run kept a partial answer: the user stopped the turn after it had
/// already published text, which is the one `llm_message` shape carrying the
/// additive R2b `aborted` marker.
///
/// This is a projection of the canonical events the response is about to send,
/// never a second source of truth for them.
pub(crate) fn answer_aborted(events: &[JobStreamEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(&event.event, StreamEvent::LlmMessage { aborted: true, .. }))
}

pub(crate) use rove_product_store::is_false;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListRunsResponse {
    pub runs: Vec<RunSummaryResponse>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RunSummaryResponse {
    #[schema(value_type = String, format = "ulid")]
    pub run_id: RunId,
    #[schema(value_type = String, format = "ulid")]
    pub session_id: SessionId,
    #[schema(value_type = String, format = "ulid")]
    pub job_id: JobId,
    #[schema(value_type = String, example = "done")]
    pub status: RunStatus,
    pub last_event_seq: u64,
    pub has_report: bool,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderTestResponse {
    pub status: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_protocol: Option<String>,
    pub api_base: String,
    pub key_env: String,
    pub key_present: bool,
    pub model: Option<String>,
    pub model_present: Option<bool>,
    pub models_count: usize,
}

/// Catalog of model ids returned by a provider inventory endpoint.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProviderModelsResponse {
    pub provider: String,
    pub provider_type: String,
    pub wire_protocol: String,
    pub api_base: String,
    pub key_env: String,
    pub key_present: bool,
    pub models: Vec<String>,
    pub models_count: usize,
}

// ─── Benchmark DTOs ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchSuiteInfoResponse {
    pub name: String,
    pub description: String,
    pub profiles: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListBenchSuitesResponse {
    pub suites: Vec<BenchSuiteInfoResponse>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StartBenchRunRequest {
    pub suite: String,
    #[serde(default = "default_bench_profile")]
    pub profile: String,
}

fn default_bench_profile() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StartBenchRunResponse {
    pub bench_run_id: String,
    pub suite: String,
    pub profile: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchRunSummary {
    pub bench_run_id: String,
    pub suite: String,
    pub profile: String,
    pub status: String,
    pub total_tasks: usize,
    pub passed_tasks: usize,
    pub failed_tasks: usize,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub evidence_root: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ListBenchRunsResponse {
    pub runs: Vec<BenchRunSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchCheckResultResponse {
    pub kind: String,
    pub description: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchArtifactsResponse {
    pub run_dir: String,
    pub trace_jsonl: String,
    pub task_state_json: String,
    pub report_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchTaskResultResponse {
    pub name: String,
    pub outcome: String,
    pub termination_reason: String,
    pub steps: u32,
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub artifacts: BenchArtifactsResponse,
    pub output: Option<String>,
    pub check_results: Vec<BenchCheckResultResponse>,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchRunDetailResponse {
    pub bench_run_id: String,
    pub suite: String,
    pub profile: String,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub total_tasks: usize,
    pub passed_tasks: usize,
    pub failed_tasks: usize,
    pub evidence_root: Option<String>,
    pub summary_md: Option<String>,
    pub tasks: Vec<BenchTaskResultResponse>,
}
