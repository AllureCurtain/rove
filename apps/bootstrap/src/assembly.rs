use std::sync::Arc;

use rove_core::ToolRegistry;
use rove_models::ModelClient;
use rove_runtime::context::{ContextBudget, ContextManager};
use rove_runtime::engine::{Engine, EngineConfig, EngineEnvironmentOptions};
use rove_runtime::environment::{ExecutionEnvironment, local_environment};
use rove_runtime::types::{
    ApprovalDecision, ApprovalPolicy, ToolApprovalProvider, UserInputProvider,
};
use rove_runtime::workspace::Workspace;

use crate::config::AppConfig;
use crate::registry::tool_registry_for_config_with_environment;

/// Options shared by first-party CLI/API engine construction.
pub struct EngineOptions<'a> {
    pub model: Box<dyn ModelClient>,
    pub workspace: &'a Workspace,
    pub config: &'a AppConfig,
    pub max_steps: u32,
    pub approval_policy: ApprovalPolicy,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    pub environment: Option<Arc<dyn ExecutionEnvironment>>,
}

/// Build the shared first-party Engine used by CLI and API.
pub async fn build_engine(mut options: EngineOptions<'_>) -> anyhow::Result<Engine> {
    let environment = options
        .environment
        .clone()
        .unwrap_or_else(|| local_environment(options.workspace));
    let registry = tool_registry_for_config_with_environment(
        options.workspace,
        options.config,
        environment.clone(),
    )
    .await?;
    options.environment = Some(environment);
    Ok(build_engine_with_registry(options, registry))
}

/// Build a product Engine when the caller already assembled a registry.
pub fn build_engine_with_registry(options: EngineOptions<'_>, registry: ToolRegistry) -> Engine {
    let context_manager = ContextManager::with_token_budget(
        options.config.load_system_prompt(),
        ContextBudget {
            soft_limit_tokens: options.config.runtime.context_soft_limit_tokens,
            hard_limit_tokens: options.config.runtime.context_hard_limit_tokens,
            reserved_tokens: options.config.runtime.context_reserved_tokens,
        },
    );

    let environment = options
        .environment
        .unwrap_or_else(|| local_environment(options.workspace));
    let mut engine = Engine::with_workspace_and_approval_decision_and_environment(
        options.model,
        registry,
        context_manager,
        EngineConfig {
            max_steps: options.max_steps,
            plan_enabled: true,
        },
        options.workspace.clone(),
        EngineEnvironmentOptions {
            approval_policy: options.approval_policy,
            approval_decision: ApprovalDecision::Reject,
            environment,
        },
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

    engine
}
