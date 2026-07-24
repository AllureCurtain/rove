//! First-party config loading and product assembly for Rove apps.

pub mod assembly;
pub mod config;
pub mod factory;
pub mod provider;
pub mod registry;

pub use assembly::{
    EngineAssemblyOptions, ProductEngineOptions, build_interface_engine, build_product_engine,
    build_product_engine_with_registry,
};
pub use config::{
    ApiConfig, AppConfig, AppConfigOverrides, ConfigSourceSummary, FallbackProviderConfig,
    MemoryConfig, ProviderConfig, ProviderOptions, RoutingConfig, RuntimeConfig, ShellConfig,
    StateConfig, ToolConfig, WebConfig,
};
pub use factory::{
    ModelClientFactory, build_anthropic_model_client, build_model_client,
    build_model_client_with_health, build_ollama_model_client, build_openai_model_client,
    try_build_model_client, try_build_model_client_with_health,
    try_build_model_client_with_registry,
};
pub use provider::{
    ProviderAuthConfig, ProviderHeaderValue, ProviderProfileConfig, SecretSource,
    default_wire_protocol_registry,
};
pub use registry::{
    default_tool_registry, default_tool_registry_with_shell_policy, product_runtime_tool_registry,
    product_tool_registry, product_tool_registry_with_shell_policy, register_extra_tools,
    runtime_tool_registry,
};
