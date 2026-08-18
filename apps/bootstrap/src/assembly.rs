use std::sync::Arc;

use rove_core::ToolRegistry;
use rove_models::ModelClient;
use rove_runtime::agents::{AgentActivationConfig, AgentSelector};
use rove_runtime::context::{ContextBudget, ContextManager};
use rove_runtime::engine::{Engine, EngineConfig, EngineEnvironmentOptions};
use rove_runtime::environment::{
    ExecutionEnvironment, LocalExecutionEnvironment, local_environment,
};
use rove_runtime::execution::ExecutionPolicy;
use rove_runtime::review::{ReviewTargetSnapshot, resolve_external_state_root};
use rove_runtime::runtime_identity::RunModelSnapshot;
use rove_runtime::tools::hooks::HookRegistry;
use rove_runtime::tools::review::ReviewSubmissionStore;
use rove_runtime::types::{
    ApprovalDecision, ApprovalPolicy, ToolApprovalProvider, UserInputProvider,
};
use rove_runtime::workspace::Workspace;

use crate::config::AppConfig;
use crate::project_trust::CAP_WORKSPACE_INSTRUCTIONS;
use crate::registry::review_tool_registry;
use crate::registry::tool_registry_for_config_with_environment;

/// Options shared by first-party CLI/API engine construction.
pub struct EngineOptions<'a> {
    pub model: Box<dyn ModelClient>,
    pub workspace: &'a Workspace,
    pub config: &'a AppConfig,
    pub max_steps: u32,
    /// Request-scoped selector override. Product/API callers use this instead
    /// of mutating the persisted application configuration.
    pub agent_selector: Option<String>,
    pub approval_policy: ApprovalPolicy,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    pub environment: Option<Arc<dyn ExecutionEnvironment>>,
    pub run_model_snapshot: Option<RunModelSnapshot>,
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
    build_engine_with_registry(options, registry)
}

/// Build a product Engine when the caller already assembled a registry.
pub fn build_engine_with_registry(
    options: EngineOptions<'_>,
    registry: ToolRegistry,
) -> anyhow::Result<Engine> {
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
    let execution_policy =
        options
            .config
            .runtime
            .execution
            .apply_to(ExecutionPolicy::from_max_steps_and_plan_flag(
                options.max_steps,
                true,
            ));
    let selector_text = options
        .agent_selector
        .as_deref()
        .unwrap_or(&options.config.runtime.agent.selector);
    let selector = AgentSelector::parse(selector_text).map_err(anyhow::Error::new)?;
    let context_tokens = u32::try_from(options.config.runtime.context_hard_limit_tokens).ok();
    let agent_activation = AgentActivationConfig {
        selector,
        workspace_source_authorized: options
            .config
            .project_capability_allowed(CAP_WORKSPACE_INSTRUCTIONS),
        load_workspace_instructions: options.config.runtime.agent.workspace_instructions,
        allow_remediation_procedures: options.config.runtime.agent.allow_remediation_procedures,
        constraints: rove_runtime::agents::validation::OperatorConstraints {
            max_steps_cap: Some(options.max_steps),
            max_tool_calls_cap: execution_policy.budgets.max_tool_calls,
            max_procedure_selections_cap: Some(
                options.config.runtime.agent.max_procedure_selections,
            ),
            ..rove_runtime::agents::validation::OperatorConstraints::unconstrained()
        },
        context_tokens,
    };

    let mut engine = Engine::with_workspace_and_approval_decision_and_environment(
        options.model,
        registry,
        context_manager,
        EngineConfig {
            max_steps: options.max_steps,
            plan_enabled: true,
            // Configured dimensions overlay the deterministic projection, so an
            // unconfigured deployment keeps its existing behavior exactly.
            execution_policy: Some(execution_policy),
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
    )
    .with_run_model_snapshot(options.run_model_snapshot)
    .with_agent_activation(agent_activation)
    .map_err(anyhow::Error::new)?;

    if let Some(input_provider) = options.input_provider {
        engine = engine.with_input_provider(input_provider);
    }
    if let Some(approval_provider) = options.approval_provider {
        engine = engine.with_approval_provider(approval_provider);
    }

    Ok(engine)
}

/// Build the shared Engine under the hard read-only Review profile. The target
/// snapshot is supplied by the caller and every registered read tool closes
/// over it; the live workspace is never used as a model-read authority.
pub fn build_review_engine(
    model: Box<dyn ModelClient>,
    workspace: &Workspace,
    snapshot: Arc<ReviewTargetSnapshot>,
    review_id: impl Into<String>,
    state_root: Option<&std::path::Path>,
    run_model_snapshot: Option<RunModelSnapshot>,
    max_steps: u32,
) -> anyhow::Result<(Engine, ReviewSubmissionStore)> {
    let external_state =
        resolve_external_state_root(workspace, state_root).map_err(anyhow::Error::new)?;
    let mut review_workspace = workspace.clone();
    review_workspace.state_dir = external_state;
    let (registry, submission_store) = review_tool_registry(snapshot, review_id);
    let context = ContextManager::with_token_budget(
        "You are Rove's hard read-only code review agent. Analyze only the immutable target snapshot. Use the review tools, submit one complete bounded finding set, and never claim that unchecked files were inspected.".to_string(),
        ContextBudget {
            soft_limit_tokens: 12_000,
            hard_limit_tokens: 16_000,
            reserved_tokens: 2_000,
        },
    );
    let environment: Arc<dyn ExecutionEnvironment> =
        Arc::new(LocalExecutionEnvironment::read_only(&review_workspace));
    let engine = Engine::with_workspace_and_approval_decision_and_environment(
        model,
        registry,
        context,
        EngineConfig {
            max_steps: max_steps.clamp(1, 256),
            plan_enabled: false,
            execution_policy: Some(ExecutionPolicy::from_max_steps_and_plan_flag(
                max_steps.clamp(1, 256),
                false,
            )),
        },
        review_workspace.clone(),
        EngineEnvironmentOptions {
            approval_policy: ApprovalPolicy::Never,
            approval_decision: ApprovalDecision::Reject,
            environment,
        },
    )
    .with_hooks(HookRegistry::default())
    .with_memory_paths(rove_runtime::memory::paths::MemoryPaths {
        session_dir: review_workspace.state_dir.join("memory").join("sessions"),
        durable_dir: review_workspace.state_dir.join("memory"),
        recall_limit: 0,
    })
    .with_model_compaction(false, 1)
    .with_run_model_snapshot(run_model_snapshot)
    .with_run_mode(rove_runtime::types::RunMode::Review);
    Ok((engine, submission_store))
}

#[cfg(test)]
mod tests {
    use rove_core::ToolRegistry;
    use rove_models::fake::FakeModelClient;
    use rove_runtime::agents::AgentActivationError;
    use rove_runtime::types::ApprovalPolicy;
    use rove_runtime::workspace::Workspace;

    use crate::{AppConfig, ProjectActivationState};

    use super::{EngineOptions, build_engine_with_registry};

    #[test]
    fn workspace_agent_source_requires_the_independent_trust_capability() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        let mut config = AppConfig::default();
        config.rebase_to_workspace(&workspace.root);
        config.runtime.agent.selector = "workspace:ops".to_string();
        config.source_summary.project_activation = ProjectActivationState::Restricted;
        config
            .source_summary
            .project_trust_granted_capabilities
            .clear();

        let result = build_engine_with_registry(
            EngineOptions {
                model: Box::new(FakeModelClient::new("unused".to_string())),
                workspace: &workspace,
                config: &config,
                max_steps: 2,
                agent_selector: None,
                approval_policy: ApprovalPolicy::Never,
                input_provider: None,
                approval_provider: None,
                environment: None,
                run_model_snapshot: None,
            },
            ToolRegistry::new(),
        );
        let error = match result {
            Ok(_) => panic!("unauthorized workspace Agent must fail assembly"),
            Err(error) => error,
        };

        let activation_error = error
            .downcast_ref::<AgentActivationError>()
            .expect("activation error must remain downcastable through anyhow");
        assert_eq!(activation_error.code(), "workspace_source_not_authorized");
    }
}
