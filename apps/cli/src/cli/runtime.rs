use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::approval::stdin_approval_provider;
use crate::cli::args::CliApprovalPolicy;
use crate::cli::input::stdin_input_provider;
use rove_app_bootstrap::build_model_client;
use rove_app_bootstrap::{AppConfig, AppConfigOverrides};
use rove_app_bootstrap::{EngineOptions, build_engine};
use rove_models::ModelClient;
use rove_models::fake::FakeModelClient;
use rove_runtime::engine::Engine;
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{ApprovalPolicy, ToolApprovalProvider, UserInputProvider};
use rove_runtime::workspace::Workspace;

pub struct CliRuntimeOptions {
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub max_steps: Option<u32>,
    pub agent: Option<String>,
    pub trust_project: bool,
    pub approval: CliApprovalPolicy,
    pub task_workspace: Option<String>,
    pub task_base: Option<PathBuf>,
    pub initial_fake_response: Option<String>,
    pub interaction: CliRuntimeInteraction,
}

#[derive(Default)]
pub enum CliRuntimeInteraction {
    #[default]
    Stdin,
    Providers {
        input_provider: Option<Arc<dyn UserInputProvider>>,
        approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    },
}

pub struct CliRuntimeProviders {
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
}

impl CliRuntimeInteraction {
    fn into_providers(self, approval_policy: ApprovalPolicy) -> CliRuntimeProviders {
        match self {
            Self::Stdin => CliRuntimeProviders {
                input_provider: Some(stdin_input_provider()),
                approval_provider: (approval_policy == ApprovalPolicy::Ask)
                    .then(stdin_approval_provider),
            },
            Self::Providers {
                input_provider,
                approval_provider,
            } => CliRuntimeProviders {
                input_provider,
                approval_provider,
            },
        }
    }
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
            agent_selector: options.agent.clone(),
            api_bind_addr: None,
            trust_project: options.trust_project,
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

    if !config.project_activation_allowed() {
        tracing::warn!(
            code = "project_trust_required",
            workspace_root = %workspace.root.display(),
            project_config_present = config.source_summary.project_config_present,
            "Project activation is restricted; workspace config and MCP servers are disabled. Pass --trust-project to activate this workspace explicitly."
        );
    }

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

    let approval_policy = cli_approval_policy(options.approval);
    let providers = options.interaction.into_providers(approval_policy);
    let engine = build_engine(EngineOptions {
        model,
        workspace: &workspace,
        config: &config,
        max_steps: config.runtime.max_steps,
        agent_selector: None,
        approval_policy,
        input_provider: providers.input_provider,
        approval_provider: providers.approval_provider,
        environment: None,
    })
    .await?;

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

    use crate::cli::args::CliApprovalPolicy;
    use rove_runtime::agents::AgentActivationError;
    use rove_runtime::types::ApprovalPolicy;

    use super::{CliRuntimeInteraction, CliRuntimeOptions, build_cli_runtime};

    #[test]
    fn custom_interaction_does_not_fall_back_to_stdin() {
        let providers = CliRuntimeInteraction::Providers {
            input_provider: None,
            approval_provider: None,
        }
        .into_providers(ApprovalPolicy::Ask);

        assert!(providers.input_provider.is_none());
        assert!(providers.approval_provider.is_none());
    }

    #[test]
    fn stdin_interaction_installs_approval_only_for_ask_policy() {
        let ask = CliRuntimeInteraction::Stdin.into_providers(ApprovalPolicy::Ask);
        let never = CliRuntimeInteraction::Stdin.into_providers(ApprovalPolicy::Never);

        assert!(ask.input_provider.is_some());
        assert!(ask.approval_provider.is_some());
        assert!(never.input_provider.is_some());
        assert!(never.approval_provider.is_none());
    }

    #[tokio::test]
    async fn runtime_builder_rebases_configured_state_dir_into_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();

        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: Some(2),
            agent: None,
            trust_project: false,
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("ready".to_string()),
            interaction: Default::default(),
        })
        .await
        .unwrap();

        assert_eq!(runtime.workspace.root, canonicalize(tmp.path()));
        assert!(runtime.workspace.state_dir.ends_with(".rove"));
        assert_eq!(runtime.config.provider.model, "fake");
    }

    #[tokio::test]
    async fn cli_agent_selector_activates_only_with_explicit_workspace_trust() {
        let tmp = tempfile::TempDir::new().unwrap();
        write_agent_definition(tmp.path());

        let denied = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: Some(2),
            agent: Some("workspace:ops".to_string()),
            trust_project: false,
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("unused".to_string()),
            interaction: Default::default(),
        })
        .await;
        let denied = match denied {
            Ok(_) => panic!("workspace Agent must require explicit Project Trust"),
            Err(error) => error,
        };
        assert!(matches!(
            denied.downcast_ref::<AgentActivationError>(),
            Some(AgentActivationError::WorkspaceSourceNotAuthorized)
        ));

        let runtime = build_cli_runtime(CliRuntimeOptions {
            cwd: Some(tmp.path().to_path_buf()),
            model: Some("fake".to_string()),
            max_steps: Some(2),
            agent: Some("workspace:ops".to_string()),
            trust_project: true,
            approval: CliApprovalPolicy::Never,
            task_workspace: None,
            task_base: None,
            initial_fake_response: Some("ready".to_string()),
            interaction: Default::default(),
        })
        .await
        .unwrap();
        let stream = runtime.engine.ask("diagnose rollback".to_string(), None);
        assert_eq!(
            stream
                .agent_profile()
                .expect("CLI-selected Agent profile")
                .selector
                .to_string(),
            "workspace:ops"
        );
    }

    fn write_agent_definition(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("agents/ops")).unwrap();
        std::fs::write(
            root.join("agents/ops/agent.toml"),
            r#"
schema_version = 1
id = "ops"
definition_version = "1.0.0"
display_name = "Operations"
default_instructions_path = "instructions.md"
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("agents/ops/instructions.md"),
            "Inspect before changing anything.",
        )
        .unwrap();
    }

    fn canonicalize(path: impl Into<PathBuf>) -> PathBuf {
        let path = path.into();
        path.canonicalize().unwrap_or(path)
    }
}
