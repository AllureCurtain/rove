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
    #[schema(value_type = String, example = "ask")]
    pub approval: Option<ApprovalPolicy>,
    pub resume: Option<String>,
    pub workspace: Option<CreateJobWorkspace>,
    pub provider: Option<ProviderProfileRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProviderProfileRequest {
    /// User-facing provider type. Values: `openai`, `openai-responses`,
    /// `anthropic`, `ollama`, `fake`. Official and relay endpoints share the
    /// same type; only `api_base` / key / model differ.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Optional display label. When empty, the API derives a name from
    /// `api_base` (hostname). Use `channel` to select the type, not `name`.
    #[serde(default)]
    pub name: String,
    /// Advanced: open wire protocol id (`openai-chat`, `anthropic-messages`, …).
    /// Optional when `channel` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_protocol: Option<String>,
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
/// Requires a typed provider profile (`channel` or `wire_protocol` + `api_base`).
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

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateJobWorkspace {
    #[schema(value_type = String, example = "task")]
    pub kind: CreateJobWorkspaceKind,
    pub name: Option<String>,
    #[schema(value_type = String)]
    pub base: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CreateJobWorkspaceKind {
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobStreamEvent {
    pub seq: u64,
    #[schema(value_type = Object)]
    pub event: StreamEvent,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct CreateJobResponse {
    #[schema(value_type = String, format = "ulid")]
    pub job_id: JobId,
    #[schema(value_type = String, format = "ulid")]
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "ulid")]
    pub resumed_from_run_id: Option<RunId>,
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
    pub pending_approvals: Vec<PendingApprovalResponse>,
    pub pending_inputs: Vec<PendingInputResponse>,
}

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
    pub channel: Option<String>,
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
    pub channel: String,
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
