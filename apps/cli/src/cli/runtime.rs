use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::cli::approval::stdin_approval_provider;
use crate::cli::args::CliApprovalPolicy;
use crate::cli::input::stdin_input_provider;
use rove_app_bootstrap::{
    AppConfig, AppConfigOverrides, EngineOptions, ModelSelection, PersistedSessionSelection,
    ProviderCatalog, ProviderCatalogService, ProviderProfileId, RunModelSnapshot,
    SessionSelectionStore, build_engine_with_registry, tool_registry_for_config_with_environment,
    try_build_model_client_with_health,
};
use rove_core::ToolRegistry;
use rove_models::fake::FakeModelClient;
use rove_models::health::{HealthConfig, ModelHealthStore};
use rove_runtime::conversation::{MessageDomainService, SqliteMessageRepository};
use rove_runtime::engine::Engine;
use rove_runtime::environment::{ExecutionEnvironment, local_environment};
use rove_runtime::state::store::StateStore;
use rove_runtime::types::{ApprovalPolicy, TaskState, ToolApprovalProvider, UserInputProvider};
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
    /// Explicit user data root override (tests, embedders). `None` uses
    /// `ROVE_DATA_ROOT` or the platform convention.
    pub data_root: Option<PathBuf>,
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
    pub state_store: StateStore,
    pub provider_catalog: ProviderCatalogService,
    pub session_selections: SessionSelectionStore,
    pub message_service: MessageDomainService,
    tool_registry: ToolRegistry,
    environment: Arc<dyn ExecutionEnvironment>,
    model_health: Arc<ModelHealthStore>,
    providers: CliRuntimeProviders,
    approval_policy: ApprovalPolicy,
    initial_fake_response: Option<String>,
}

pub struct RunAssembly {
    pub engine: Engine,
    pub selection: ModelSelection,
    pub run_model_snapshot: RunModelSnapshot,
}

impl CliRuntime {
    pub fn default_selection(&self) -> anyhow::Result<ModelSelection> {
        let catalog = self.provider_catalog.load()?;
        if self.config.provider.model == "fake"
            && self
                .config
                .provider
                .profiles
                .values()
                .any(|profile| profile.provider_type == "fake")
            && catalog.profiles().is_empty()
        {
            return Ok(ModelSelection {
                profile_id: ProviderProfileId::new("programmatic-fake")?,
                model: "fake".to_string(),
                reasoning: "default".to_string(),
                revision: "programmatic".to_string(),
            });
        }
        selection_from_config(&self.config, &catalog)
    }

    pub fn catalog(&self) -> anyhow::Result<ProviderCatalog> {
        self.provider_catalog.load().map_err(anyhow::Error::from)
    }

    pub fn selection_for_session(
        &self,
        session_id: rove_runtime::types::SessionId,
    ) -> anyhow::Result<(ModelSelection, u64)> {
        if let Some(persisted) = self.session_selections.load(&session_id.to_string())? {
            return Ok((persisted.selection, persisted.revision));
        }
        Ok((self.default_selection()?, 0))
    }

    pub fn persist_session_selection(
        &self,
        session_id: rove_runtime::types::SessionId,
        expected_revision: u64,
        selection: ModelSelection,
    ) -> anyhow::Result<PersistedSessionSelection> {
        let catalog = self.provider_catalog.load()?;
        let profile = catalog.profile_config(&selection.profile_id)?;
        catalog.resolve(&selection, &self.workspace.root)?;
        profile
            .resolve(
                &self.provider_catalog.paths().root,
                true,
                Some(&selection.model),
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "provider_unavailable: the selected Provider credential is unavailable"
                )
            })?;
        self.session_selections
            .update(&session_id.to_string(), expected_revision, selection)
            .map_err(anyhow::Error::from)
    }

    pub async fn assemble_run(
        &self,
        message: &str,
        selection: Option<&ModelSelection>,
        resume_state: Option<&TaskState>,
        allow_selection_change: bool,
    ) -> anyhow::Result<RunAssembly> {
        let catalog = self.provider_catalog.load().map_err(|error| {
            anyhow::anyhow!("provider_unavailable: {error}; configure ~/.rove/config.toml")
        })?;
        let programmatic_fake_selected = selection
            .is_some_and(|selection| selection.profile_id.to_string() == "programmatic-fake");
        if (selection.is_none() || programmatic_fake_selected)
            && self.config.provider.model == "fake"
            && self
                .config
                .provider
                .profiles
                .values()
                .any(|profile| profile.provider_type == "fake")
            && catalog.profiles().is_empty()
        {
            return self.assemble_programmatic_fake(message, resume_state).await;
        }
        let mut selection = selection
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| selection_from_config(&self.config, &catalog))?;
        selection.revision = catalog.revision().to_string();
        let snapshot = catalog
            .snapshot(&selection, &self.workspace.root)
            .map_err(|error| anyhow::anyhow!("provider_unavailable: {error}"))?;
        validate_resume_model(resume_state, &snapshot, allow_selection_change)?;

        let mut config = self.config.clone();
        config.provider.active = Some(selection.profile_id.to_string());
        config.provider.model = selection.model.clone();
        config.provider.profiles = catalog.document().provider.profiles.clone();
        for profile in config.provider.profiles.values_mut() {
            profile.rebase_secret_paths(&self.provider_catalog.paths().root);
        }
        config.source_summary.user_config_loaded = true;
        config.source_summary.user_config_path = self.provider_catalog.paths().config_file.clone();
        config.provider.fallback_profiles = catalog.document().provider.fallback_profiles.clone();
        let profile = catalog.profile_config(&selection.profile_id)?;
        let model = if profile.provider_type == "fake" {
            Box::new(FakeModelClient::new(
                self.initial_fake_response
                    .clone()
                    .unwrap_or_else(|| format!("fake response: {message}")),
            )) as Box<dyn rove_models::ModelClient>
        } else {
            try_build_model_client_with_health(
                &config,
                selection.model.clone(),
                Arc::clone(&self.model_health),
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "provider_unavailable: the selected Provider credential or endpoint configuration is unavailable"
                )
            })?
        };
        let engine = build_engine_with_registry(
            EngineOptions {
                model,
                workspace: &self.workspace,
                config: &config,
                max_steps: config.runtime.max_steps,
                agent_selector: None,
                approval_policy: self.approval_policy,
                input_provider: self.providers.input_provider.clone(),
                approval_provider: self.providers.approval_provider.clone(),
                environment: Some(Arc::clone(&self.environment)),
                run_model_snapshot: Some(snapshot.clone()),
            },
            self.tool_registry.clone(),
        )?;
        Ok(RunAssembly {
            engine,
            selection,
            run_model_snapshot: snapshot,
        })
    }

    async fn assemble_programmatic_fake(
        &self,
        message: &str,
        resume_state: Option<&TaskState>,
    ) -> anyhow::Result<RunAssembly> {
        let selection = ModelSelection {
            profile_id: ProviderProfileId::new("programmatic-fake")?,
            model: "fake".to_string(),
            reasoning: "default".to_string(),
            revision: "programmatic".to_string(),
        };
        let snapshot = RunModelSnapshot {
            profile_id: selection.profile_id.to_string(),
            provider_type: "fake".to_string(),
            wire_protocol: "fake".to_string(),
            endpoint: String::new(),
            model: selection.model.clone(),
            reasoning: selection.reasoning.clone(),
            catalog_revision: selection.revision.clone(),
            safe_config_digest: rove_runtime::context::stable_hash("programmatic-fake"),
        };
        validate_resume_model(resume_state, &snapshot, false)?;
        let model = Box::new(FakeModelClient::new(
            self.initial_fake_response
                .clone()
                .unwrap_or_else(|| format!("fake response: {message}")),
        ));
        let engine = build_engine_with_registry(
            EngineOptions {
                model,
                workspace: &self.workspace,
                config: &self.config,
                max_steps: self.config.runtime.max_steps,
                agent_selector: None,
                approval_policy: self.approval_policy,
                input_provider: self.providers.input_provider.clone(),
                approval_provider: self.providers.approval_provider.clone(),
                environment: Some(Arc::clone(&self.environment)),
                run_model_snapshot: Some(snapshot.clone()),
            },
            self.tool_registry.clone(),
        )?;
        Ok(RunAssembly {
            engine,
            selection,
            run_model_snapshot: snapshot,
        })
    }
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
            data_root: options.data_root.clone(),
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
    if config.state_dir_is_contract_managed() {
        config.ensure_contract_layout().map_err(|error| {
            anyhow::anyhow!("user state workspace directory is unavailable: {error}")
        })?;
    }

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

    let approval_policy = cli_approval_policy(options.approval);
    let providers = options.interaction.into_providers(approval_policy);
    let environment = local_environment(&workspace);
    let tool_registry =
        tool_registry_for_config_with_environment(&workspace, &config, Arc::clone(&environment))
            .await?;

    let state_store = StateStore::with_index_path(
        &workspace.state_dir,
        config.sqlite_path(),
        config.state.sqlite_busy_timeout_ms,
    );
    state_store.index.initialize()?;
    let message_service = MessageDomainService::new(Arc::new(SqliteMessageRepository::new(
        state_store.index.path(),
        state_store.index.busy_timeout_ms(),
    )));
    let model_health = Arc::new(ModelHealthStore::with_persistence(
        HealthConfig {
            failure_threshold: config.routing.failure_threshold,
            open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
        },
        &workspace.state_dir,
    ));

    let session_selections = SessionSelectionStore::new(&workspace.state_dir);
    Ok(CliRuntime {
        workspace,
        config,
        state_store,
        provider_catalog: ProviderCatalogService::discover(),
        session_selections,
        message_service,
        tool_registry,
        environment,
        model_health,
        providers,
        approval_policy,
        initial_fake_response: options.initial_fake_response,
    })
}

fn selection_from_config(
    config: &AppConfig,
    catalog: &ProviderCatalog,
) -> anyhow::Result<ModelSelection> {
    if let Some(active) = config.provider.active.as_deref() {
        let profile_id = ProviderProfileId::new(active.to_string())?;
        if catalog.profile_config(&profile_id).is_ok() {
            return Ok(ModelSelection {
                profile_id,
                model: config.provider.model.clone(),
                reasoning: "default".to_string(),
                revision: catalog.revision().to_string(),
            });
        }
    }
    if config.provider.model == "fake" {
        let profile_id = ProviderProfileId::new("default")?;
        if catalog.profile_config(&profile_id).is_ok() {
            return Ok(ModelSelection {
                profile_id,
                model: "fake".to_string(),
                reasoning: "default".to_string(),
                revision: catalog.revision().to_string(),
            });
        }
    }
    catalog.default_selection().map_err(|error| {
        anyhow::anyhow!(
            "provider_onboarding_required: {error}; create ~/.rove/config.toml or explicitly pass --model fake"
        )
    })
}

fn validate_resume_model(
    resume_state: Option<&TaskState>,
    current: &RunModelSnapshot,
    allow_selection_change: bool,
) -> anyhow::Result<()> {
    let Some(saved) = resume_state
        .and_then(|state| {
            state
                .checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.runtime_identity.as_ref())
                .or(state.runtime_identity.as_ref())
        })
        .and_then(|identity| identity.run_model.as_ref())
    else {
        return Ok(());
    };
    let selection_changed = saved.profile_id != current.profile_id
        || saved.model != current.model
        || saved.reasoning != current.reasoning;
    if selection_changed && !allow_selection_change {
        anyhow::bail!(
            "provider_changed_for_resume: the saved run uses {}/{}; restore that selection before resuming",
            saved.profile_id,
            saved.model
        );
    }
    if !selection_changed
        && (saved.provider_type != current.provider_type
            || saved.wire_protocol != current.wire_protocol
            || saved.endpoint != current.endpoint
            || saved.safe_config_digest != current.safe_config_digest)
    {
        anyhow::bail!(
            "provider_changed_for_resume: the selected Provider identity changed since the saved run"
        );
    }
    Ok(())
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
        let data_root = tempfile::TempDir::new().unwrap();

        let runtime = build_cli_runtime(CliRuntimeOptions {
            data_root: Some(data_root.path().to_path_buf()),
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

        assert_eq!(
            runtime.workspace.root,
            runtime.config.source_summary.workspace_root
        );
        let layout = rove_app_bootstrap::WorkspaceStateLayout::resolve(
            runtime.config.user_state_roots.as_ref().unwrap().root(),
            &runtime.config.source_summary.workspace_root,
        );
        assert_eq!(runtime.workspace.state_dir, layout.workspace_dir);
        assert!(data_root.path().join("workspaces").is_dir());
        assert!(!tmp.path().join(".rove").exists());
        assert_eq!(runtime.config.provider.model, "fake");
    }

    #[tokio::test]
    async fn cli_agent_selector_activates_only_with_explicit_workspace_trust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_root = tempfile::TempDir::new().unwrap();
        write_agent_definition(tmp.path());

        let denied = build_cli_runtime(CliRuntimeOptions {
            data_root: Some(data_root.path().to_path_buf()),
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
        .await
        .unwrap();
        let denied = match denied
            .assemble_run("diagnose rollback", None, None, false)
            .await
        {
            Ok(_) => panic!("workspace Agent must require explicit Project Trust"),
            Err(error) => error,
        };
        assert!(matches!(
            denied.downcast_ref::<AgentActivationError>(),
            Some(AgentActivationError::WorkspaceSourceNotAuthorized)
        ));

        let runtime = build_cli_runtime(CliRuntimeOptions {
            data_root: Some(data_root.path().to_path_buf()),
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
        let assembly = runtime
            .assemble_run("diagnose rollback", None, None, false)
            .await
            .unwrap();
        let stream = assembly.engine.ask("diagnose rollback".to_string(), None);
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
}
