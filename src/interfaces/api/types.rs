//! Request and response DTOs for the HTTP API.
//!
//! These are the serializable contract types exchanged over `/jobs`, `/runs`,
//! and the provider endpoints. They are deliberately free of server state so the
//! OpenAPI schema (`docs.rs`) and integration tests can depend on them without
//! reaching into the handler internals in [`super`].

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::core::events::StreamEvent;
use crate::core::types::{
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
    pub api_base: String,
    pub key_env: String,
    pub key_present: bool,
    pub model: Option<String>,
    pub model_present: Option<bool>,
    pub models_count: usize,
}
