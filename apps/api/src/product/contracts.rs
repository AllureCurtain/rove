//! Public product-control contracts shared by the API, persistence, transcript,
//! and Web client lanes.
//!
//! Product metadata is intentionally separate from each workspace's runtime
//! state. These types may point at runtime sessions, jobs, and runs, but they
//! never copy canonical event facts into the product store.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use async_trait::async_trait;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;

use rove_runtime::state::store::StateStore;
use rove_runtime::types::{JobId, RunId, RunStatus, SessionId};

use crate::types::JobStreamEvent;

pub const M1_BROWSER_SOURCE_SCHEMA_VERSION: u32 = 1;
pub const MAX_PRODUCT_WORKSPACES: usize = 256;
pub const MAX_PRODUCT_SESSIONS: usize = 2_048;
pub const MAX_PRODUCT_PROVIDER_PROFILES: usize = 128;
pub const MAX_PRODUCT_TEXT_BYTES: usize = 512;
pub const MAX_PRODUCT_API_BASE_BYTES: usize = 2_048;
pub const MAX_PRODUCT_PATH_BYTES: usize = 32_768;
pub const MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES: usize = 128;
pub const MAX_M1_BROWSER_MIGRATION_BODY_BYTES: usize = 64 * 1_048_576;
pub const MAX_PRODUCT_MEMORY_CONTENT_BYTES: usize = 64 * 1_024;
pub const DEFAULT_PRODUCT_MAX_STEPS: u32 = 8;
pub const MAX_PRODUCT_MAX_STEPS: u32 = 256;
/// A fork preserves references to prior runtime runs, never copied event
/// payloads. Keep that ancestry bounded so a deeply branched catalog cannot
/// turn one fork or transcript read into an unbounded operation.
pub const MAX_PRODUCT_FORK_INHERITED_RUNS: usize = 512;
pub const MAX_PROJECT_TRUST_CAPABILITIES: usize = 6;

macro_rules! product_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
        #[serde(transparent)]
        #[schema(value_type = String, format = "ulid")]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(SessionId::new().to_string())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                serde_json::from_value::<SessionId>(serde_json::Value::String(value.to_string()))
                    .map(|id| Self(id.to_string()))
                    .map_err(|_| format!("invalid {}", stringify!($name)))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::from_str(&value).map_err(D::Error::custom)
            }
        }
    };
}

product_id!(
    ProductWorkspaceId,
    "Server-owned identity for one product workspace catalog entry."
);
product_id!(
    ProductSessionId,
    "Server-owned product conversation identity, distinct from runtime SessionId."
);
product_id!(
    ProductProviderProfileId,
    "Server-owned identity for a persisted provider profile."
);
product_id!(
    ProductMigrationReceiptId,
    "Server-owned identity for a committed browser migration receipt."
);
product_id!(
    ProductTurnClaimId,
    "Internal compare-and-set token for one active product-session turn."
);
product_id!(
    ProductControlId,
    "Server-owned identity for one steer or follow-up control message."
);
product_id!(
    ProductForkId,
    "Server-owned identity for immutable product-session fork provenance."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductSessionStatus {
    Idle,
    Running,
    Error,
    NeedsAttention,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductProviderType {
    Openai,
    #[serde(rename = "openai-responses")]
    OpenaiResponses,
    Anthropic,
    Ollama,
    Fake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductThemePreference {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductApprovalPreference {
    #[default]
    Ask,
    Auto,
    Never,
}

/// Session model configuration is explicit about the provider default. The
/// API never invents a reasoning level for protocols that do not support it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductReasoningPreference {
    #[default]
    Default,
    Low,
    Medium,
    High,
}

impl ProductReasoningPreference {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl fmt::Display for ProductReasoningPreference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProductReasoningPreference {
    type Err = ProductStoreError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "default" => Ok(Self::Default),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err(ProductStoreError::new(
                ProductErrorCode::ProductInvalidInput,
                format!("invalid reasoning preference: {value}"),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductWorkspaceKind {
    Folder,
    Repo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductControlKind {
    Steer,
    Followup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductControlStatus {
    Pending,
    Accepted,
    Applied,
    Dropped,
    Abandoned,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductControl {
    pub id: ProductControlId,
    pub product_session_id: ProductSessionId,
    pub kind: ProductControlKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub content: String,
    pub status: ProductControlStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "ulid")]
    pub run_id: Option<RunId>,
    pub seq: i64,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductControlRequest {
    pub content: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductControlsResponse {
    pub controls: Vec<ProductControl>,
}

#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductControlStatusFilter {
    Pending,
    Accepted,
    Applied,
    Dropped,
    Abandoned,
    Revoked,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductWorkspace {
    pub id: ProductWorkspaceId,
    /// Canonical absolute execution root resolved by the server.
    #[schema(value_type = String)]
    pub canonical_root: PathBuf,
    pub kind: ProductWorkspaceKind,
    pub display_name: String,
    pub pinned: bool,
    pub last_opened_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductTrustState {
    Unknown,
    Restricted,
    Trusted,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductTrustCapability {
    ProjectConfiguration,
    WorkspaceInstructions,
    McpProcesses,
    HooksExtensions,
    ProviderCredentials,
    ExternalPaths,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductTrustDecision {
    Grant,
    Deny,
    Revoke,
}

impl ProductTrustCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectConfiguration => "project_configuration",
            Self::WorkspaceInstructions => "workspace_instructions",
            Self::McpProcesses => "mcp_processes",
            Self::HooksExtensions => "hooks_extensions",
            Self::ProviderCredentials => "provider_credentials",
            Self::ExternalPaths => "external_paths",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductTrustDecisionRequest {
    pub decision: ProductTrustDecision,
    #[serde(default)]
    pub capabilities: Vec<ProductTrustCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductTrustStatus {
    pub workspace_id: ProductWorkspaceId,
    pub state: ProductTrustState,
    pub identity_digest: String,
    pub invalidated_capabilities: Vec<String>,
    pub granted_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductRuntimeBinding {
    pub ordinal: u64,
    #[schema(value_type = String, format = "ulid")]
    pub runtime_session_id: SessionId,
    #[schema(value_type = String, format = "ulid")]
    pub latest_job_id: JobId,
    #[schema(value_type = String, format = "ulid")]
    pub latest_run_id: RunId,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSession {
    pub id: ProductSessionId,
    pub workspace_id: ProductWorkspaceId,
    pub title: String,
    pub status: ProductSessionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_binding: Option<ProductRuntimeBinding>,
    /// Parent product session retained as lineage even when the parent catalog
    /// row has subsequently been deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<ProductSessionId>,
    /// Exact parent runtime run from which this session inherited history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "ulid")]
    pub fork_point_run_id: Option<RunId>,
    /// Terminal canonical event sequence for `fork_point_run_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_point_seq: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSessionRunBinding {
    pub product_session_id: ProductSessionId,
    pub ordinal: u64,
    #[schema(value_type = String, format = "ulid")]
    pub runtime_session_id: SessionId,
    #[schema(value_type = String, format = "ulid")]
    pub runtime_job_id: JobId,
    #[schema(value_type = String, format = "ulid")]
    pub runtime_run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "ulid")]
    pub resumed_from_run_id: Option<RunId>,
    pub bound_at: String,
}

/// Immutable source boundary for one child product session. This contains
/// runtime identities and references only; canonical event facts continue to
/// live in the source workspace StateStore.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductFork {
    pub id: ProductForkId,
    pub parent_product_session_id: ProductSessionId,
    pub child_product_session_id: ProductSessionId,
    pub parent_workspace_id: ProductWorkspaceId,
    pub parent_title: String,
    #[schema(value_type = String, format = "ulid")]
    pub source_runtime_session_id: SessionId,
    #[schema(value_type = String, format = "ulid")]
    pub source_runtime_job_id: JobId,
    #[schema(value_type = String, format = "ulid")]
    pub source_runtime_run_id: RunId,
    pub fork_at_event_seq: u64,
    pub idempotency_key: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductForkRequest {
    /// The already-final parent run to branch from. The server derives the
    /// terminal event sequence and rejects anything incomplete or corrupt.
    #[schema(value_type = String, format = "ulid")]
    pub fork_at_run_id: RunId,
    #[serde(default)]
    pub title: Option<String>,
    /// Required client-generated key. Retrying the exact same action returns
    /// the same child; a body mismatch returns a typed conflict.
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductForkResponse {
    pub fork: ProductFork,
    pub session: ProductSession,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductForksResponse {
    pub forks: Vec<ProductFork>,
}

/// One runtime-run reference projected into a child's inherited prefix. It
/// deliberately has no event content and remains readable if a parent Product
/// session is later removed from the catalog.
#[derive(Debug, Clone)]
pub struct ProductForkInheritedRun {
    pub ordinal: u64,
    pub source_product_session_id: ProductSessionId,
    pub runtime_session_id: SessionId,
    pub runtime_job_id: JobId,
    pub runtime_run_id: RunId,
    /// Present on the exact terminal source boundary; earlier inherited runs
    /// are projected through their own complete canonical records.
    pub through_event_seq: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ProductForkContext {
    pub fork: ProductFork,
    pub inherited_runs: Vec<ProductForkInheritedRun>,
}

/// Runtime boundary verified by the coordinator. Browser JSON can never
/// construct this; ProductStore rechecks it against its immutable binding
/// ledger in the transaction that creates the child.
#[derive(Debug, Clone)]
pub struct VerifiedProductForkBoundary {
    pub parent_product_session_id: ProductSessionId,
    pub parent_workspace_id: ProductWorkspaceId,
    pub parent_title: String,
    pub source_runtime_session_id: SessionId,
    pub source_runtime_job_id: JobId,
    pub source_runtime_run_id: RunId,
    pub fork_at_event_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductProviderProfile {
    pub id: ProductProviderProfileId,
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductProviderSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProductProviderProfileId>,
    pub model: String,
    pub approval: ProductApprovalPreference,
    pub max_steps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSessionModelConfig {
    pub product_session_id: ProductSessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProductProviderProfileId>,
    pub model: String,
    pub reasoning: ProductReasoningPreference,
    pub max_steps: u32,
    pub revision: u64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductSessionModelConfigRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProductProviderProfileId>,
    pub model: String,
    #[serde(default)]
    pub reasoning: ProductReasoningPreference,
    #[serde(default = "default_product_max_steps")]
    pub max_steps: u32,
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductModelDescriptor {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub supports_reasoning: bool,
    #[serde(default)]
    pub supported_reasoning: Vec<ProductReasoningPreference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductProviderModelsResponse {
    pub profile_id: ProductProviderProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    pub models: Vec<ProductModelDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSessionRunModelView {
    pub product_session_id: ProductSessionId,
    pub ordinal: u64,
    #[schema(value_type = String, format = "ulid")]
    pub runtime_run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<ProductProviderProfileId>,
    pub model: String,
    pub reasoning: ProductReasoningPreference,
    pub max_steps: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_availability: Option<ProductPricingAvailability>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_mtok_prompt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_mtok_completion: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_mtok_cache_read: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSessionRunModelsResponse {
    pub runs: Vec<ProductSessionRunModelView>,
}

/// Whether a run's cost can be computed from a trusted price snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductPricingAvailability {
    Priced,
    LocalZero,
    Unpriced,
}

impl ProductPricingAvailability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Priced => "priced",
            Self::LocalZero => "local_zero",
            Self::Unpriced => "unpriced",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "priced" => Some(Self::Priced),
            "local_zero" => Some(Self::LocalZero),
            "unpriced" => Some(Self::Unpriced),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default, PartialEq, Eq)]
pub struct ProductUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    pub cached_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct ProductCostBreakdown {
    pub currency: String,
    pub availability: ProductPricingAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ProductContextOccupancy {
    pub token_estimate: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    pub estimate_kind: String,
    pub included_history_messages: u64,
    pub dropped_history_messages: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_mode: Option<String>,
    #[serde(default)]
    pub compaction_degraded: bool,
    #[serde(default)]
    pub compaction_auto_triggered: bool,
    #[serde(default)]
    pub compacted_history_messages: u64,
    #[serde(default)]
    pub compaction_source_messages: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_prompt_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductRunUsage {
    #[schema(value_type = String, format = "ulid")]
    pub runtime_run_id: RunId,
    pub ordinal: u64,
    pub model: String,
    pub usage: ProductUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ProductCostBreakdown>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<ProductContextOccupancy>,
    pub steps: u32,
    pub tool_calls: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSessionUsageResponse {
    pub product_session_id: ProductSessionId,
    pub totals: ProductUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totals_cost: Option<ProductCostBreakdown>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_context: Option<ProductContextOccupancy>,
    pub runs: Vec<ProductRunUsage>,
    pub partial_reasons: Vec<String>,
}

const fn default_product_max_steps() -> u32 {
    DEFAULT_PRODUCT_MAX_STEPS
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductPreferences {
    pub schema_version: u32,
    #[serde(default)]
    pub revision: u64,
    pub theme: ProductThemePreference,
    #[serde(default)]
    pub default_approval_policy: ProductApprovalPreference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_workspace_id: Option<ProductWorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<ProductSessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_selection: Option<ProductProviderSelection>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductWorkspaceRequest {
    #[schema(value_type = String)]
    pub root: PathBuf,
    pub kind: ProductWorkspaceKind,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductSessionRequest {
    pub workspace_id: ProductWorkspaceId,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductProviderProfileRequest {
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductProviderProfileRequest {
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductPreferencesRequest {
    pub schema_version: u32,
    /// Optional for C0 compatibility. Settings clients send the last observed
    /// revision so concurrent writes fail instead of silently overwriting.
    #[serde(default)]
    pub expected_revision: Option<u64>,
    pub theme: ProductThemePreference,
    /// Omission preserves the current value for C0 clients.
    #[serde(default)]
    pub default_approval_policy: Option<ProductApprovalPreference>,
    #[serde(default)]
    pub active_workspace_id: Option<ProductWorkspaceId>,
    #[serde(default)]
    pub active_session_id: Option<ProductSessionId>,
    #[serde(default)]
    pub provider_selection: Option<ProductProviderSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductWorkspacesResponse {
    pub workspaces: Vec<ProductWorkspace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductSessionsResponse {
    pub sessions: Vec<ProductSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductProviderProfilesResponse {
    pub provider_profiles: Vec<ProductProviderProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductMemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductMemoryScope {
    Global,
    Project,
    Session,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductMemoryLayer {
    Durable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductMemorySource {
    ProductSettings,
    LlmTool,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMemoryTopic {
    pub slug: String,
    pub title: String,
    pub layer: ProductMemoryLayer,
    pub memory_type: ProductMemoryType,
    pub scope: ProductMemoryScope,
    pub source: ProductMemorySource,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    pub description: String,
    pub metadata_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMemoryTopicsResponse {
    pub topics: Vec<ProductMemoryTopic>,
    pub total: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMemoryTopicContentResponse {
    pub topic: ProductMemoryTopic,
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductMemoryTopicRequest {
    pub slug: String,
    pub title: String,
    pub memory_type: ProductMemoryType,
    pub scope: ProductMemoryScope,
    pub confidence: f32,
    #[serde(default)]
    pub description: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductMemoryTopicRequest {
    pub title: String,
    pub memory_type: ProductMemoryType,
    pub scope: ProductMemoryScope,
    pub confidence: f32,
    #[serde(default)]
    pub description: String,
    pub content: String,
    #[serde(default)]
    pub expected_updated_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductMcpTransport {
    Stdio,
    /// Deprecated HTTP+SSE transport, retained for existing configurations.
    Sse,
    /// Current MCP HTTP transport with negotiated session and version.
    StreamableHttp,
}

impl ProductMcpTransport {
    /// True for a transport retained only for compatibility. Surfaced so
    /// product diagnostics can mark it without guessing from the name.
    pub fn is_deprecated(self) -> bool {
        matches!(self, Self::Sse)
    }

    /// True when the transport is configured with a URL rather than a command.
    pub fn is_http(self) -> bool {
        matches!(self, Self::Sse | Self::StreamableHttp)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMcpServer {
    pub name: String,
    pub enabled: bool,
    pub required: bool,
    pub transport: ProductMcpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub request_timeout_ms: u64,
    /// Server-owned deprecation verdict for this server's transport, so the
    /// client renders one truth instead of hardcoding which name is legacy.
    pub transport_deprecated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMcpServersResponse {
    pub servers: Vec<ProductMcpServer>,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductMcpHealthStatus {
    Ready,
    Degraded,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMcpHealthSnapshot {
    pub server_name: String,
    pub required: bool,
    pub transport: ProductMcpTransport,
    pub status: ProductMcpHealthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_config_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_identity_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot_id: Option<String>,
    pub tool_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMcpHealthResponse {
    pub servers: Vec<ProductMcpHealthSnapshot>,
    pub total: usize,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateProductMcpServerRequest {
    pub name: String,
    #[serde(default = "default_product_mcp_enabled")]
    pub enabled: bool,
    #[serde(default = "default_product_mcp_required")]
    pub required: bool,
    pub transport: ProductMcpTransport,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env_names: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default = "default_product_mcp_timeout_ms")]
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProductMcpServerRequest {
    pub enabled: bool,
    #[serde(default = "default_product_mcp_required")]
    pub required: bool,
    pub transport: ProductMcpTransport,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env_names: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    pub request_timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMcpToolDescriptor {
    pub name: String,
    pub description: String,
    pub destructive: bool,
    pub parallel_safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductMcpProbeResponse {
    pub server_name: String,
    pub transport: ProductMcpTransport,
    pub tools: Vec<ProductMcpToolDescriptor>,
    pub tested_at: String,
}

const fn default_product_mcp_enabled() -> bool {
    true
}

const fn default_product_mcp_required() -> bool {
    true
}

const fn default_product_mcp_timeout_ms() -> u64 {
    30_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductConnectionStatus {
    Connected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductStoreStatus {
    Ready,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductResumeHealthStatus {
    Healthy,
    NeedsAttention,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductResumeHealth {
    pub status: ProductResumeHealthStatus,
    pub workspace_count: u64,
    pub session_count: u64,
    pub bound_session_count: u64,
    pub running_session_count: u64,
    pub needs_attention_session_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductExecutionAdapter {
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductExecutionWorkspaceKind {
    Folder,
    Repo,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProductExecutionCapabilities {
    pub filesystem_read: bool,
    pub filesystem_write: bool,
    pub process_run: bool,
    pub process_stdio: bool,
    pub observations: bool,
    pub process_background: bool,
    pub process_pty: bool,
    pub workspace_checkpoints: bool,
    pub artifact_projection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductExecutionEnvironmentInfo {
    pub adapter: ProductExecutionAdapter,
    pub workspace_kind: ProductExecutionWorkspaceKind,
    pub workspace_digest: String,
    pub capabilities: ProductExecutionCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductAgentRuntimeInfo {
    /// Configured base selector. A request may still provide an explicit
    /// selector, and the resolved run identity is emitted canonically.
    pub selector: String,
    pub workspace_source_authorized: bool,
    pub workspace_instructions_enabled: bool,
    pub allow_remediation_procedures: bool,
    pub max_procedure_selections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductRuntimeInfo {
    pub api_version: String,
    pub connection: ProductConnectionStatus,
    pub product_store: ProductStoreStatus,
    pub execution_environment: ProductExecutionEnvironmentInfo,
    pub agent: ProductAgentRuntimeInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_health: Option<ProductResumeHealth>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductTranscriptStatus {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductTranscriptPartialReasonCode {
    MissingRunMapping,
    RuntimeRunMissing,
    RuntimeStateUnavailable,
    RuntimeIdentityMismatch,
    MissingEventRange,
    CorruptEvent,
    CorruptArtifact,
    CleanedHistory,
    ResponseLimitReached,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductTranscriptPartialReason {
    pub code: ProductTranscriptPartialReasonCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_ordinal: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "ulid")]
    pub run_id: Option<RunId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductTranscriptFallbackSource {
    Report,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductTranscriptFallback {
    pub source: ProductTranscriptFallbackSource,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductTranscriptRunSegment {
    pub binding: ProductSessionRunBinding,
    /// True when this segment is a read-only reference inherited at a fork
    /// boundary rather than an event written by the requested product session.
    #[serde(default)]
    pub inherited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_product_session_id: Option<ProductSessionId>,
    #[schema(value_type = String, example = "done")]
    pub run_status: RunStatus,
    pub observed_through_seq: u64,
    pub last_event_seq: u64,
    pub events: Vec<JobStreamEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<ProductTranscriptFallback>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProductTranscriptResponse {
    pub product_session_id: ProductSessionId,
    pub workspace_id: ProductWorkspaceId,
    pub status: ProductTranscriptStatus,
    pub partial_reasons: Vec<ProductTranscriptPartialReason>,
    pub segments: Vec<ProductTranscriptRunSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum M1BrowserMigrationSource {
    WebM1LocalStorage,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct M1WorkspaceImport {
    pub source_id: String,
    #[schema(value_type = String)]
    pub root: PathBuf,
    pub kind: ProductWorkspaceKind,
    pub display_name: String,
    pub pinned: bool,
    pub last_opened_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct M1SessionImport {
    pub source_id: String,
    pub source_workspace_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub legacy_active_job_id: Option<String>,
    #[serde(default)]
    pub legacy_active_run_id: Option<String>,
    #[serde(default)]
    pub legacy_resumed_from_run_id: Option<String>,
    #[serde(default)]
    pub legacy_has_durable_turn: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct M1ProviderProfileImport {
    pub source_id: String,
    pub label: String,
    pub provider_type: ProductProviderType,
    pub api_base: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct M1ProviderSelectionImport {
    #[serde(default)]
    pub source_profile_id: Option<String>,
    pub model: String,
    pub approval: ProductApprovalPreference,
    pub max_steps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct M1SafePreferencesImport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ProductThemePreference>,
    #[serde(default)]
    pub source_active_workspace_id: Option<String>,
    #[serde(default)]
    pub source_active_session_id: Option<String>,
    #[serde(default)]
    pub provider_selection: Option<M1ProviderSelectionImport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct M1BrowserMigrationRequest {
    pub source: M1BrowserMigrationSource,
    pub source_schema_version: u32,
    pub idempotency_key: String,
    pub workspaces: Vec<M1WorkspaceImport>,
    pub sessions: Vec<M1SessionImport>,
    pub provider_profiles: Vec<M1ProviderProfileImport>,
    pub safe_preferences: M1SafePreferencesImport,
}

/// Stable digest persisted with the migration receipt. The request is already
/// reduced to strict allowlisted fields before this helper can be called.
pub fn m1_browser_migration_digest(
    request: &M1BrowserMigrationRequest,
) -> Result<String, serde_json::Error> {
    let canonical = serde_json::to_string(request)?;
    Ok(rove_runtime::context::stable_hash(&canonical))
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct M1WorkspaceIdMapping {
    pub source_id: String,
    pub workspace_id: ProductWorkspaceId,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct M1SessionIdMapping {
    pub source_id: String,
    pub product_session_id: ProductSessionId,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct M1ProviderProfileIdMapping {
    pub source_id: String,
    pub provider_profile_id: ProductProviderProfileId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum M1MigrationDisposition {
    Applied,
    AlreadyApplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum M1MigrationIssueCode {
    InvalidWorkspace,
    MissingWorkspace,
    InvalidRuntimeHint,
    AmbiguousRuntimeBinding,
    RuntimeBindingNotFound,
    InvalidPreferenceReference,
    PreferenceWriteConflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct M1MigrationIssue {
    pub code: M1MigrationIssueCode,
    pub entity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct M1BrowserMigrationResponse {
    pub source_schema_version: u32,
    pub idempotency_key: String,
    pub receipt_id: ProductMigrationReceiptId,
    pub disposition: M1MigrationDisposition,
    pub workspace_mappings: Vec<M1WorkspaceIdMapping>,
    pub session_mappings: Vec<M1SessionIdMapping>,
    pub provider_profile_mappings: Vec<M1ProviderProfileIdMapping>,
    pub issues: Vec<M1MigrationIssue>,
    pub applied_at: String,
}

#[derive(Debug, Clone)]
pub struct ProductSessionContext {
    pub workspace: ProductWorkspace,
    pub session: ProductSession,
    pub fork: Option<ProductForkContext>,
}

#[derive(Debug, Clone)]
pub struct ProductTurnClaim {
    pub claim_id: ProductTurnClaimId,
    pub context: ProductSessionContext,
    pub previous_status: ProductSessionStatus,
    pub previous_binding: Option<ProductRuntimeBinding>,
    pub model_config: ProductSessionModelConfig,
}

#[derive(Debug, Clone)]
pub struct CommitProductRunBinding {
    pub claim_id: ProductTurnClaimId,
    pub product_session_id: ProductSessionId,
    pub runtime_session_id: SessionId,
    pub runtime_job_id: JobId,
    pub runtime_run_id: RunId,
    pub resumed_from_run_id: Option<RunId>,
    /// The durable follow-up that caused this turn, when this is an automatic
    /// queued continuation. ProductStore commits its delivery state and the
    /// run binding together so restart recovery can distinguish a started
    /// follow-up from one that must be confirmed by the user.
    pub followup_control_id: Option<ProductControlId>,
    /// The config captured while the product turn was claimed. The binding
    /// transaction records it with the run so later edits cannot rewrite the
    /// model used by an already-started run.
    pub model_config: ProductSessionModelConfig,
}

/// One atomically claimed queued follow-up and its exclusive product turn.
///
/// The store creates the turn claim, changes the session to `running`, and
/// moves the control from `pending` to `accepted` in one transaction. A second
/// coordinator therefore cannot dequeue the next follow-up while this one is
/// being prepared.
#[derive(Debug, Clone)]
pub struct ProductFollowupTurnClaim {
    pub control: ProductControl,
    pub turn: ProductTurnClaim,
}

/// Result of closing a product turn with a non-final or indeterminate
/// outcome. The control rows are returned so the coordinator can surface the
/// corresponding canonical lifecycle events before it publishes the terminal
/// run result.
#[derive(Debug, Clone)]
pub struct ProductTurnControlFinish {
    pub dropped_steers: Vec<ProductControl>,
    pub abandoned_followups: Vec<ProductControl>,
}

/// Runtime identity validated by the coordinator before a browser migration
/// enters the ProductStore transaction. Browser hints never construct this
/// type directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedM1SessionRunBinding {
    pub source_session_id: String,
    pub ordinal: u64,
    pub runtime_session_id: SessionId,
    pub runtime_job_id: JobId,
    pub runtime_run_id: RunId,
    pub resumed_from_run_id: Option<RunId>,
    pub(crate) verified_workspace_root: PathBuf,
    pub(crate) verified_workspace_kind: ProductWorkspaceKind,
}

/// Server-owned compare-and-set token captured before runtime migration
/// inspection. It is intentionally absent from the browser request digest.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum M1PreferencesBaseline {
    NotRequested,
    Revision(u64),
}

/// Result of the atomic migration receipt/preferences preflight read.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub enum M1BrowserMigrationPreflight {
    Replay(M1BrowserMigrationResponse),
    Prepare(M1PreferencesBaseline),
}

/// Sanitized migration plus server-side runtime validation results. The store
/// commits entity mappings, verified bindings, issues, and receipt atomically.
#[derive(Debug, Clone)]
pub struct PreparedM1BrowserMigration {
    pub request: M1BrowserMigrationRequest,
    pub verified_run_bindings: Vec<VerifiedM1SessionRunBinding>,
    pub issues: Vec<M1MigrationIssue>,
    pub(crate) preferences_baseline: M1PreferencesBaseline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductErrorCode {
    ProductNotFound,
    ProductInvalidInput,
    ProductStoreUnavailable,
    ProductSessionActive,
    ProductSessionWorkspaceMismatch,
    ProductSessionResumeConflict,
    ProductSessionRuntimeStateMissing,
    ProductSessionRuntimeStateCorrupt,
    ProductBindingCorrupt,
    ProductRevisionConflict,
    ProductMemoryInvalidSlug,
    ProductMemoryNotFound,
    ProductMemoryConflict,
    ProductMcpInvalidInput,
    ProductMcpNotFound,
    ProductMcpConflict,
    ProjectTrustInvalidInput,
    ProjectTrustUnavailable,
    ProjectTrustRequired,
    MigrationIdempotencyConflict,
    ProductControlConflict,
    ProductControlRejected,
    ProductForkConflict,
    ProductForkSourceInvalid,
    ProductSessionModelConfigConflict,
    ProductProviderProfileUnavailable,
    ProductStorageFailure,
}

impl ProductErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProductNotFound => "product_not_found",
            Self::ProductInvalidInput => "product_invalid_input",
            Self::ProductStoreUnavailable => "product_store_unavailable",
            Self::ProductSessionActive => "product_session_active",
            Self::ProductSessionWorkspaceMismatch => "product_session_workspace_mismatch",
            Self::ProductSessionResumeConflict => "product_session_resume_conflict",
            Self::ProductSessionRuntimeStateMissing => "product_session_runtime_state_missing",
            Self::ProductSessionRuntimeStateCorrupt => "product_session_runtime_state_corrupt",
            Self::ProductBindingCorrupt => "product_binding_corrupt",
            Self::ProductRevisionConflict => "product_revision_conflict",
            Self::ProductMemoryInvalidSlug => "product_memory_invalid_slug",
            Self::ProductMemoryNotFound => "product_memory_not_found",
            Self::ProductMemoryConflict => "product_memory_conflict",
            Self::ProductMcpInvalidInput => "product_mcp_invalid_input",
            Self::ProductMcpNotFound => "product_mcp_not_found",
            Self::ProductMcpConflict => "product_mcp_conflict",
            Self::ProjectTrustInvalidInput => rove_app_bootstrap::PROJECT_TRUST_INVALID_INPUT_CODE,
            Self::ProjectTrustUnavailable => rove_app_bootstrap::PROJECT_TRUST_UNAVAILABLE_CODE,
            Self::ProjectTrustRequired => rove_app_bootstrap::PROJECT_TRUST_REQUIRED_CODE,
            Self::MigrationIdempotencyConflict => "migration_idempotency_conflict",
            Self::ProductControlConflict => "product_control_conflict",
            Self::ProductControlRejected => "product_control_rejected",
            Self::ProductForkConflict => "product_fork_conflict",
            Self::ProductForkSourceInvalid => "product_fork_source_invalid",
            Self::ProductSessionModelConfigConflict => "product_session_model_config_conflict",
            Self::ProductProviderProfileUnavailable => "product_provider_profile_unavailable",
            Self::ProductStorageFailure => "product_storage_failure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductStoreError {
    pub code: ProductErrorCode,
    pub message: String,
}

impl fmt::Display for ProductStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProductStoreError {}

impl ProductStoreError {
    pub fn new(code: ProductErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn unavailable() -> Self {
        Self::new(
            ProductErrorCode::ProductStoreUnavailable,
            "product store is not available",
        )
    }
}

#[async_trait]
pub trait ProductStore: Send + Sync {
    /// Mark claims left active by a previous API process as needing attention.
    async fn recover_stale_turn_claims(&self) -> Result<u64, ProductStoreError>;

    async fn list_workspaces(&self) -> Result<Vec<ProductWorkspace>, ProductStoreError>;
    async fn get_workspace(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<ProductWorkspace, ProductStoreError>;
    async fn create_workspace(
        &self,
        request: CreateProductWorkspaceRequest,
    ) -> Result<ProductWorkspace, ProductStoreError>;
    async fn delete_workspace(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<(), ProductStoreError>;
    async fn list_sessions(
        &self,
        workspace_id: &ProductWorkspaceId,
    ) -> Result<Vec<ProductSession>, ProductStoreError>;
    async fn create_session(
        &self,
        request: CreateProductSessionRequest,
    ) -> Result<ProductSession, ProductStoreError>;
    async fn update_session(
        &self,
        session_id: &ProductSessionId,
        request: UpdateProductSessionRequest,
    ) -> Result<ProductSession, ProductStoreError>;
    async fn delete_session(&self, session_id: &ProductSessionId) -> Result<(), ProductStoreError>;
    async fn get_session_model_config(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductSessionModelConfig, ProductStoreError>;
    async fn update_session_model_config(
        &self,
        session_id: &ProductSessionId,
        request: UpdateProductSessionModelConfigRequest,
    ) -> Result<ProductSessionModelConfig, ProductStoreError>;
    async fn list_session_run_models(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductSessionRunModelView>, ProductStoreError>;
    async fn get_session_context(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductSessionContext, ProductStoreError>;
    async fn list_run_bindings(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductSessionRunBinding>, ProductStoreError>;

    /// Atomically materialize an immutable child session from an already
    /// coordinator-verified terminal parent boundary. `already_exists` is true
    /// only for an idempotent replay of the exact same request.
    async fn create_fork(
        &self,
        request: CreateProductForkRequest,
        boundary: VerifiedProductForkBoundary,
    ) -> Result<(ProductSession, ProductFork, bool /* already_exists */), ProductStoreError>;

    /// Resolve an existing fork before inspecting the parent runtime boundary.
    /// This keeps an exact idempotent retry recoverable after the parent
    /// catalog row has been removed, while a body mismatch remains a conflict.
    async fn replay_fork(
        &self,
        parent_session_id: &ProductSessionId,
        request: &CreateProductForkRequest,
    ) -> Result<Option<(ProductSession, ProductFork)>, ProductStoreError>;

    /// Direct children only, ordered and bounded for branch-tree expansion.
    async fn list_forks(
        &self,
        parent_session_id: &ProductSessionId,
    ) -> Result<Vec<ProductFork>, ProductStoreError>;

    async fn claim_session_turn(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductTurnClaim, ProductStoreError>;
    async fn commit_run_binding(
        &self,
        binding: CommitProductRunBinding,
    ) -> Result<ProductSessionRunBinding, ProductStoreError>;
    async fn finish_session_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        status: ProductSessionStatus,
    ) -> Result<(), ProductStoreError>;

    /// Finish a successfully-final product turn and atomically claim the
    /// oldest queued follow-up, if one exists. This closes the enqueue/final
    /// race: a follow-up written before this transaction is claimed here;
    /// one written after observes the session idle and is claimed by its route.
    async fn finish_session_turn_and_claim_followup(
        &self,
        claim_id: &ProductTurnClaimId,
    ) -> Result<Option<ProductFollowupTurnClaim>, ProductStoreError>;

    /// Drop steers which did not reach a model turn, while the current product
    /// turn claim still owns the session. The coordinator publishes the
    /// returned rows as canonical `steer_dropped` events before it writes the
    /// run terminal event. `applied` steers are historical facts and are never
    /// changed by this operation.
    async fn drop_unapplied_steers_for_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        run_id: RunId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError>;

    /// Close an unsuccessful product turn and transition all controls that
    /// have not reached a terminal control lifecycle at that exact boundary.
    /// This is one transaction so a concurrently submitted follow-up is either
    /// part of the old queue and explicitly abandoned, or is a new instruction
    /// submitted after the session has reached its next state.
    /// `run_id` is `None` only before a runtime run has been allocated. Once
    /// a run id exists, dropped steers retain it for auditability.
    async fn finish_session_turn_and_abandon_pending_controls(
        &self,
        claim_id: &ProductTurnClaimId,
        run_id: Option<RunId>,
        status: ProductSessionStatus,
        reason: &str,
    ) -> Result<ProductTurnControlFinish, ProductStoreError>;

    async fn list_provider_profiles(
        &self,
    ) -> Result<Vec<ProductProviderProfile>, ProductStoreError>;
    async fn get_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
    ) -> Result<ProductProviderProfile, ProductStoreError>;
    async fn create_provider_profile(
        &self,
        request: CreateProductProviderProfileRequest,
    ) -> Result<ProductProviderProfile, ProductStoreError>;
    async fn update_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
        request: UpdateProductProviderProfileRequest,
    ) -> Result<ProductProviderProfile, ProductStoreError>;
    async fn delete_provider_profile(
        &self,
        profile_id: &ProductProviderProfileId,
    ) -> Result<(), ProductStoreError>;

    async fn get_preferences(&self) -> Result<ProductPreferences, ProductStoreError>;
    async fn update_preferences(
        &self,
        request: UpdateProductPreferencesRequest,
    ) -> Result<ProductPreferences, ProductStoreError>;
    async fn get_resume_health(&self) -> Result<ProductResumeHealth, ProductStoreError>;
    /// Validate the sanitized request and replay an existing receipt before
    /// callers inspect runtime artifacts that may have since been cleaned.
    async fn preflight_m1_browser_migration(
        &self,
        request: &M1BrowserMigrationRequest,
    ) -> Result<M1BrowserMigrationPreflight, ProductStoreError>;
    async fn apply_m1_browser_migration(
        &self,
        migration: PreparedM1BrowserMigration,
    ) -> Result<M1BrowserMigrationResponse, ProductStoreError>;

    /// Persist a steer or follow-up control. Same idempotency key + same body
    /// returns the existing row (`already_existed = true`); same key + different
    /// body returns [`ProductErrorCode::ProductControlConflict`].
    async fn create_control(
        &self,
        session_id: &ProductSessionId,
        kind: ProductControlKind,
        request: CreateProductControlRequest,
    ) -> Result<(ProductControl, bool /* already_existed */), ProductStoreError>;

    async fn list_controls(
        &self,
        session_id: &ProductSessionId,
        filter: Option<ProductControlStatus>,
    ) -> Result<Vec<ProductControl>, ProductStoreError>;

    async fn get_control(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
    ) -> Result<ProductControl, ProductStoreError>;

    /// Compare-and-swap control status. Rejects when the stored status is not `from`.
    async fn transition_control(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
        from: ProductControlStatus,
        to: ProductControlStatus,
        applied_run_id: Option<&RunId>,
    ) -> Result<ProductControl, ProductStoreError>;

    /// Explicitly confirm that an abandoned follow-up may be retried. This is
    /// the only path that moves an indeterminate queued continuation back to
    /// `pending`; ordinary restart recovery never does so after uncertainty.
    async fn confirm_abandoned_followup(
        &self,
        session_id: &ProductSessionId,
        control_id: &ProductControlId,
    ) -> Result<ProductControl, ProductStoreError>;

    /// Mark every still-pending control abandoned (non-final run termination).
    async fn abandon_pending_controls(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<u64, ProductStoreError>;

    /// List pending follow-ups in seq order (read-only; prefer
    /// [`Self::claim_next_pending_followup`] for drain).
    async fn list_pending_followups(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductControl>, ProductStoreError>;

    /// Atomically claim the next pending follow-up (`pending` → `accepted`) so
    /// crash restart cannot double-start the same control.
    async fn claim_next_pending_followup(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Option<ProductControl>, ProductStoreError>;

    /// Atomically dequeue one pending follow-up and claim the product session
    /// turn required to start it. Only idle sessions without an active turn
    /// claim are eligible.
    async fn claim_next_followup_turn(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Option<ProductFollowupTurnClaim>, ProductStoreError>;

    /// Undo an automatic follow-up claim before any runtime run is started.
    /// This keeps a transient assembly failure retryable without exposing a
    /// second runnable session turn.
    async fn requeue_followup_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
    ) -> Result<(), ProductStoreError>;

    /// Persist the exact runtime run id that is about to be started for a
    /// claimed follow-up. Once this succeeds, restart recovery must assume
    /// the runtime side effect may have started and require confirmation
    /// rather than risk a duplicate run.
    async fn reserve_followup_run(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
        run_id: RunId,
    ) -> Result<(), ProductStoreError>;

    /// Release an automatic follow-up claim into a conservative state when
    /// runtime preparation reached an uncertain boundary.
    async fn abandon_followup_turn(
        &self,
        claim_id: &ProductTurnClaimId,
        control_id: &ProductControlId,
        reason: &str,
    ) -> Result<(), ProductStoreError>;

    /// Sessions whose final prior turn is idle and which still own a queued
    /// follow-up. Used once at API startup to resume safe server-side drains.
    async fn list_idle_sessions_with_pending_followups(
        &self,
    ) -> Result<Vec<ProductSessionId>, ProductStoreError>;

    /// Close queued steer controls that were never observed at a runtime safe
    /// point. The returned rows are used to publish canonical dropped events
    /// before the run terminal event.
    async fn drop_pending_steers(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError>;

    /// Move queued follow-ups to explicit confirmation after a non-final or
    /// otherwise indeterminate run outcome. Returned rows become canonical
    /// `followup_abandoned` events before the terminal event is published.
    async fn abandon_pending_followups(
        &self,
        session_id: &ProductSessionId,
        reason: &str,
    ) -> Result<Vec<ProductControl>, ProductStoreError>;
}

pub trait ProductRuntimeStateResolver: Send + Sync {
    fn state_store_for(
        &self,
        workspace: &ProductWorkspace,
    ) -> Result<StateStore, ProductStoreError>;
}

#[async_trait]
pub trait ProductTranscriptReader: Send + Sync {
    async fn read_transcript(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductTranscriptResponse, ProductStoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_ids_reject_legacy_browser_ids() {
        let error = "sess_browser-owned"
            .parse::<ProductSessionId>()
            .unwrap_err();

        assert_eq!(error, "invalid ProductSessionId");
    }

    #[test]
    fn provider_type_preserves_public_hyphenated_name() {
        let value = serde_json::to_value(ProductProviderType::OpenaiResponses).unwrap();

        assert_eq!(value, "openai-responses");
    }

    #[test]
    fn browser_migration_rejects_unknown_raw_key_fields() {
        let payload = serde_json::json!({
            "source": "web_m1_local_storage",
            "source_schema_version": 1,
            "idempotency_key": "migration-1",
            "workspaces": [],
            "sessions": [],
            "provider_profiles": [{
                "source_id": "prov_legacy",
                "label": "unsafe",
                "provider_type": "openai",
                "api_base": "https://api.openai.com/v1",
                "api_key_env": "OPENAI_API_KEY",
                "api_key": "must-not-cross-the-boundary",
                "default_model": "gpt-test",
                "updated_at": "2026-07-26T00:00:00Z"
            }],
            "safe_preferences": {
                "theme": "system"
            }
        });

        let error = serde_json::from_value::<M1BrowserMigrationRequest>(payload).unwrap_err();

        assert!(error.to_string().contains("unknown field `api_key`"));
    }
}
