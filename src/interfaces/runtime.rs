use std::sync::Arc;

use crate::config::AppConfig;
use crate::core::types::{ApprovalPolicy, ToolApprovalProvider, UserInputProvider};
use crate::core::workspace::Workspace;
use crate::models::traits::ModelClient;
use crate::tools::runtime_tool_registry;
use rove_app_bootstrap::assembly::{ProductEngineOptions, build_product_engine_with_registry};
use rove_runtime::engine::Engine;

pub(crate) struct EngineAssemblyOptions<'a> {
    pub model: Box<dyn ModelClient>,
    pub workspace: &'a Workspace,
    pub config: &'a AppConfig,
    pub max_steps: u32,
    pub approval_policy: ApprovalPolicy,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
}

pub(crate) async fn build_interface_engine(
    options: EngineAssemblyOptions<'_>,
) -> anyhow::Result<Engine> {
    let registry = runtime_tool_registry(
        options.workspace,
        options.config.shell_policy(),
        options
            .config
            .resolve_path(&options.config.tool.mcp_config_path),
    )
    .await?;
    Ok(build_product_engine_with_registry(
        ProductEngineOptions {
            model: options.model,
            workspace: options.workspace,
            config: options.config,
            max_steps: options.max_steps,
            approval_policy: options.approval_policy,
            input_provider: options.input_provider,
            approval_provider: options.approval_provider,
        },
        registry,
    ))
}
