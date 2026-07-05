use std::sync::Arc;

use crate::config::AppConfig;
use crate::core::context::{ContextBudget, ContextManager};
use crate::core::engine::{Engine, EngineConfig};
use crate::core::types::{ApprovalPolicy, ToolApprovalProvider, UserInputProvider};
use crate::core::workspace::Workspace;
use crate::models::traits::ModelClient;
use crate::tools::runtime_tool_registry;

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
    let context_manager = ContextManager::with_token_budget(
        options.config.load_system_prompt(),
        ContextBudget {
            soft_limit_tokens: options.config.runtime.context_soft_limit_tokens,
            hard_limit_tokens: options.config.runtime.context_hard_limit_tokens,
            reserved_tokens: options.config.runtime.context_reserved_tokens,
        },
    );

    let mut engine = Engine::with_workspace(
        options.model,
        registry,
        context_manager,
        EngineConfig {
            max_steps: options.max_steps,
            plan_enabled: true,
        },
        options.workspace.clone(),
        options.approval_policy,
    )
    .with_planner_prompt(options.config.load_planner_prompt())
    .with_memory_paths(options.config.memory_paths())
    .with_model_compaction(
        options.config.runtime.model_compaction_enabled,
        options.config.runtime.compaction_failure_threshold,
    );

    if let Some(input_provider) = options.input_provider {
        engine = engine.with_input_provider(input_provider);
    }
    if let Some(approval_provider) = options.approval_provider {
        engine = engine.with_approval_provider(approval_provider);
    }

    Ok(engine)
}
