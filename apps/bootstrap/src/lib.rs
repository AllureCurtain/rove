//! First-party config loading and product assembly for Rove apps.

pub mod assembly;
pub mod config;
pub mod factory;
pub mod home;
pub mod project_trust;
pub mod provider;
pub mod provider_catalog;
pub mod provider_migration;
pub mod registry;
pub mod session_selection;
pub mod state_migration;
pub mod user_config;
pub mod user_state;

pub use assembly::{EngineOptions, build_engine, build_engine_with_registry, build_review_engine};
pub use config::{
    AgentConfig, ApiConfig, AppConfig, AppConfigOverrides, ConfigSourceSummary, MemoryConfig,
    ProviderConfig, ProviderOptions, RoutingConfig, RuntimeConfig, ShellConfig, StateConfig,
    ToolConfig, WebConfig,
};
pub use factory::{
    ModelClientFactory, build_model_client, build_model_client_with_health, try_build_model_client,
    try_build_model_client_with_health, try_build_model_client_with_registry,
};
pub use home::{
    HomeError, LegacyRunMigration, RoveHome, ensure_home_legacy_run_migration, find_rove_home,
    migrate_workspace_legacy_runs,
};
pub use project_trust::{
    CAP_EXTERNAL_PATHS, CAP_HOOKS_EXTENSIONS, CAP_MCP_PROCESSES, CAP_PROJECT_CONFIGURATION,
    CAP_PROVIDER_CREDENTIALS, CAP_WORKSPACE_INSTRUCTIONS, PROJECT_TRUST_INVALID_INPUT_CODE,
    PROJECT_TRUST_REQUIRED_CODE, PROJECT_TRUST_STORE_ENV, PROJECT_TRUST_UNAVAILABLE_CODE,
    ProjectActivationSource, ProjectActivationState, ProjectTrustCapability, ProjectTrustDecision,
    ProjectTrustRecord, ProjectTrustRepository, ProjectTrustResolution, TRUSTED_WORKSPACES_ENV,
    all_capability_names, canonical_root_key, capability_digest_map,
    capability_digest_map_with_roots, provider_capability_selector_for_workspace,
    resolve_project_trust_record, workspace_identity_digest,
};
pub use provider::{
    KeyringReference, ProviderAuthConfig, ProviderHeaderValue, ProviderProfileConfig, SecretSource,
    default_wire_protocol_registry, wire_protocol_for_provider_type,
};
pub use provider_catalog::{
    CredentialReference, ModelDescriptor, ModelSelection, OnboardingCredential,
    OsProviderCredentialStore, ProviderCatalog, ProviderCatalogError, ProviderCatalogService,
    ProviderCredentialStore, ProviderOnboardingError, ProviderOnboardingReceipt,
    ProviderOnboardingRequest, ProviderOnboardingService, ProviderProbeFailureKind,
    ProviderProbeReceipt, ProviderProfile, ProviderProfileId, ResolvedRunModel, RunModelSnapshot,
    SelectionRevision,
};
pub use provider_migration::{
    PROVIDER_MIGRATION_RECEIPT_SCHEMA_VERSION, ProviderMigrationAction, ProviderMigrationError,
    ProviderMigrationOptions, ProviderMigrationOutcome, ProviderMigrationReport,
    ProviderMigrationSource, run_provider_migration,
};
pub use registry::{
    register_extra_tools, review_tool_registry, tool_registry, tool_registry_for_config,
    tool_registry_for_config_with_environment, tool_registry_with_mcp,
    tool_registry_with_mcp_and_environment, tool_registry_with_mcp_authority_and_environment,
    tool_registry_with_shell_policy,
};
pub use session_selection::{
    PersistedSessionSelection, SessionSelectionError, SessionSelectionStore,
};
pub use user_config::{
    USER_CONFIG_SCHEMA_VERSION, UserConfigDocument, UserConfigLoader, UserConfigPaths,
    UserConfigWriter,
};
pub use user_state::{
    DATA_ROOT_ENV, LEGACY_STATE_DIR, McpCatalogAuthority, UserStateError, UserStateRoots,
    WORKSPACE_MARKER_SCHEMA_VERSION, WorkspaceMarker, WorkspaceStateLayout,
    effective_default_mcp_authority, effective_default_mcp_catalog, ensure_workspace_layout,
    state_dir_for_run_discovery, verify_workspace_marker, workspace_storage_key,
};
