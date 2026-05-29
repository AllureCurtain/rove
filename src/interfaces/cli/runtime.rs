use std::path::PathBuf;

use crate::config::{AppConfig, AppConfigOverrides};
use crate::core::context::{ContextBudget, ContextManager};
use crate::core::engine::{Engine, EngineConfig};
use crate::core::types::ApprovalPolicy;
use crate::core::workspace::Workspace;
use crate::interfaces::cli::approval::stdin_approval_provider;
use crate::interfaces::cli::args::CliApprovalPolicy;
use crate::interfaces::cli::input::stdin_input_provider;
use crate::models::factory::build_model_client;
use crate::models::fake::FakeModelClient;
use crate::models::traits::ModelClient;
use crate::state::store::StateStore;

pub struct CliRuntimeOptions {
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub max_steps: Option<u32>,
    pub approval: CliApprovalPolicy,
    pub task_workspace: Option<String>,
    pub task_base: Option<PathBuf>,
    pub initial_fake_response: Option<String>,
}

pub struct CliRuntime {
    pub workspace: Workspace,
    pub config: AppConfig,
    pub engine: Engine,
    pub state_store: StateStore,
}

pub async fn build_cli_runtime(options: CliRuntimeOptions) -> anyhow::Result<CliRuntime> {
    let cwd = options
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let detected_workspace = Workspace::detect(&cwd)?;

    let mut config = AppConfig::load(
        &detected_workspace.root,
        AppConfigOverrides {
            model: options.model.clone(),
            max_steps: options.max_steps,
            api_bind_addr: None,
        },
    )?;
    let workspace = if let Some(task_name) = options.task_workspace.as_deref() {
        let base = options
            .task_base
            .clone()
            .unwrap_or_else(|| config.state_dir().join("tasks"));
        let task_workspace = Workspace::task(&base, task_name)?;
        config.rebase_to_workspace(&task_workspace.root);
        task_workspace
    } else {
        detected_workspace
    };

    let configured_state_dir = config.state_dir();
    if configured_state_dir != workspace.state_dir {
        std::fs::create_dir_all(&configured_state_dir)?;
    }
    let workspace = Workspace {
        state_dir: configured_state_dir,
        ..workspace
    };
    workspace.ensure_state_dir()?;

    tracing::info!(
        workspace_root = %workspace.root.display(),
        workspace_kind = ?workspace.kind,
        "Workspace detected"
    );

    let model_id = config.provider.model.clone();
    let model: Box<dyn ModelClient> = if model_id == "fake" {
        Box::new(FakeModelClient::new(
            options
                .initial_fake_response
                .unwrap_or_else(|| "fake response".to_string()),
        ))
    } else {
        build_model_client(&config, model_id)
    };

    let mcp_config_path = config.resolve_path(&config.tool.mcp_config_path);
    let registry =
        crate::tools::runtime_tool_registry(&workspace, config.shell_policy(), mcp_config_path)
            .await?;

    let system_prompt = config.load_system_prompt();
    let context_manager = ContextManager::with_token_budget(
        system_prompt,
        ContextBudget {
            soft_limit_tokens: config.runtime.context_soft_limit_tokens,
            hard_limit_tokens: config.runtime.context_hard_limit_tokens,
            reserved_tokens: config.runtime.context_reserved_tokens,
        },
    );

    let memory_paths = config.memory_paths();
    let engine_config = EngineConfig {
        max_steps: config.runtime.max_steps,
        plan_enabled: true,
    };
    let approval_policy = cli_approval_policy(options.approval);
    let engine = Engine::with_workspace(
        model,
        registry,
        context_manager,
        engine_config,
        workspace.clone(),
        approval_policy,
    )
    .with_planner_prompt(config.load_planner_prompt())
    .with_memory_paths(memory_paths)
    .with_model_compaction(
        config.runtime.model_compaction_enabled,
        config.runtime.compaction_failure_threshold,
    )
    .with_input_provider(stdin_input_provider());
    let engine = if approval_policy == ApprovalPolicy::Ask {
        engine.with_approval_provider(stdin_approval_provider())
    } else {
        engine
    };

    let state_store = StateStore::with_index_path(
        &workspace.state_dir,
        config.sqlite_path(),
        config.state.sqlite_busy_timeout_ms,
    );

    Ok(CliRuntime {
        workspace,
        config,
        engine,
        state_store,
    })
}

fn cli_approval_policy(policy: CliApprovalPolicy) -> ApprovalPolicy {
    match policy {
        CliApprovalPolicy::Ask => ApprovalPolicy::Ask,
        CliApprovalPolicy::Auto => ApprovalPolicy::Auto,
        CliApprovalPolicy::Never => ApprovalPolicy::Never,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::interfaces::cli::args::CliApprovalPolicy;

    use super::{CliRuntimeOptions, build_cli_runtime};

    #[tokio::test]
    async fn runtime_builder_rebases_configured_state_dir_into_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();

        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: Some(2),
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("ready".to_string()),
        })
        .await
        .unwrap();

        assert_eq!(runtime.workspace.root, canonicalize(tmp.path()));
        assert!(runtime.workspace.state_dir.ends_with(".rove"));
        assert_eq!(runtime.config.provider.model, "fake");
    }

    fn canonicalize(path: impl Into<PathBuf>) -> PathBuf {
        let path = path.into();
        path.canonicalize().unwrap_or(path)
    }
}
