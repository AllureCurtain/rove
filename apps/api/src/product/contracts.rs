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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductApprovalPreference {
    Ask,
    Auto,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductWorkspaceKind {
    Folder,
    Repo,
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
pub struct ProductPreferences {
    pub schema_version: u32,
    pub theme: ProductThemePreference,
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
    pub theme: ProductThemePreference,
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
    pub theme: ProductThemePreference,
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
}

#[derive(Debug, Clone)]
pub struct ProductTurnClaim {
    pub claim_id: ProductTurnClaimId,
    pub context: ProductSessionContext,
    pub previous_binding: Option<ProductRuntimeBinding>,
}

#[derive(Debug, Clone)]
pub struct CommitProductRunBinding {
    pub claim_id: ProductTurnClaimId,
    pub product_session_id: ProductSessionId,
    pub runtime_session_id: SessionId,
    pub runtime_job_id: JobId,
    pub runtime_run_id: RunId,
    pub resumed_from_run_id: Option<RunId>,
}

/// Runtime identity validated by the coordinator before a browser migration
/// enters the ProductStore transaction. Browser hints never construct this
/// type directly.
#[derive(Debug, Clone)]
pub struct VerifiedM1SessionRunBinding {
    pub source_session_id: String,
    pub ordinal: u64,
    pub runtime_session_id: SessionId,
    pub runtime_job_id: JobId,
    pub runtime_run_id: RunId,
    pub resumed_from_run_id: Option<RunId>,
}

/// Sanitized migration plus server-side runtime validation results. The store
/// commits entity mappings, verified bindings, issues, and receipt atomically.
#[derive(Debug, Clone)]
pub struct PreparedM1BrowserMigration {
    pub request: M1BrowserMigrationRequest,
    pub verified_run_bindings: Vec<VerifiedM1SessionRunBinding>,
    pub issues: Vec<M1MigrationIssue>,
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
    MigrationIdempotencyConflict,
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
            Self::MigrationIdempotencyConflict => "migration_idempotency_conflict",
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
    async fn get_session_context(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<ProductSessionContext, ProductStoreError>;
    async fn list_run_bindings(
        &self,
        session_id: &ProductSessionId,
    ) -> Result<Vec<ProductSessionRunBinding>, ProductStoreError>;

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

    async fn list_provider_profiles(
        &self,
    ) -> Result<Vec<ProductProviderProfile>, ProductStoreError>;
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
    async fn apply_m1_browser_migration(
        &self,
        migration: PreparedM1BrowserMigration,
    ) -> Result<M1BrowserMigrationResponse, ProductStoreError>;
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
