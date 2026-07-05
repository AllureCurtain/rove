use std::path::PathBuf;

use crate::config::{AppConfig, AppConfigOverrides};
use crate::core::engine::Engine;
use crate::core::types::ApprovalPolicy;
use crate::core::workspace::Workspace;
use crate::interfaces::cli::approval::stdin_approval_provider;
use crate::interfaces::cli::args::CliApprovalPolicy;
use crate::interfaces::cli::input::stdin_input_provider;
use crate::interfaces::runtime::{EngineAssemblyOptions, build_interface_engine};
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

    let approval_policy = cli_approval_policy(options.approval);
    let engine = build_interface_engine(EngineAssemblyOptions {
        model,
        workspace: &workspace,
        config: &config,
        max_steps: config.runtime.max_steps,
        approval_policy,
        input_provider: Some(stdin_input_provider()),
        approval_provider: (approval_policy == ApprovalPolicy::Ask).then(stdin_approval_provider),
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
