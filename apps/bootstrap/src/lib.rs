//! First-party config loading and product assembly for Rove apps.

pub mod assembly;
pub mod config;
pub mod factory;
pub mod project_trust;
pub mod provider;
pub mod registry;

pub use assembly::{EngineOptions, build_engine, build_engine_with_registry};
pub use config::{
    AgentConfig, ApiConfig, AppConfig, AppConfigOverrides, ConfigSourceSummary, MemoryConfig,
    ProviderConfig, ProviderOptions, RoutingConfig, RuntimeConfig, ShellConfig, StateConfig,
    ToolConfig, WebConfig,
};
pub use factory::{
    ModelClientFactory, build_model_client, build_model_client_with_health, try_build_model_client,
    try_build_model_client_with_health, try_build_model_client_with_registry,
};
pub use project_trust::{
    CAP_EXTERNAL_PATHS, CAP_HOOKS_EXTENSIONS, CAP_MCP_PROCESSES, CAP_PROJECT_CONFIGURATION,
    CAP_PROVIDER_CREDENTIALS, CAP_WORKSPACE_INSTRUCTIONS, PROJECT_TRUST_INVALID_INPUT_CODE,
    PROJECT_TRUST_REQUIRED_CODE, PROJECT_TRUST_STORE_ENV, PROJECT_TRUST_UNAVAILABLE_CODE,
    ProjectActivationSource, ProjectActivationState, ProjectTrustCapability, ProjectTrustDecision,
    ProjectTrustRecord, ProjectTrustRepository, ProjectTrustResolution, TRUSTED_WORKSPACES_ENV,
    all_capability_names, canonical_root_key, capability_digest_map,
    provider_capability_selector_for_workspace, resolve_project_trust_record,
    workspace_identity_digest,
};
pub use provider::{
    ProviderAuthConfig, ProviderHeaderValue, ProviderProfileConfig, SecretSource,
    default_wire_protocol_registry, wire_protocol_for_provider_type,
};
pub use registry::{
    register_extra_tools, tool_registry, tool_registry_for_config,
    tool_registry_for_config_with_environment, tool_registry_with_mcp,
    tool_registry_with_mcp_and_environment, tool_registry_with_shell_policy,
};
