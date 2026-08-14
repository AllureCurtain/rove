use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use figment::Figment;
use figment::providers::{Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

use rove_runtime::agents::AgentSelector;
use rove_runtime::execution::{
    EvaluatorMode, ExecutionPolicy, FinalizerPolicy, StrategySelectionSource,
};
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::workspace::WorkspaceKind;

use crate::project_trust::{
    ProjectActivation, ProjectActivationSource, ProjectActivationState, ProjectTrustRepository,
    TRUSTED_WORKSPACES_ENV, capability_digest_map,
};
use crate::provider::ProviderProfileConfig;

pub use rove_models::ProviderOptions;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub runtime: RuntimeConfig,
    pub provider: ProviderConfig,
    pub tool: ToolConfig,
    pub memory: MemoryConfig,
    pub state: StateConfig,
    pub api: ApiConfig,
    pub web: WebConfig,
    pub routing: RoutingConfig,
    #[serde(skip)]
    pub source_summary: ConfigSourceSummary,
    /// Workspace-owned environment values scoped to this configuration load.
    /// Values are never serialized or exposed through `Debug`.
    #[serde(skip)]
    pub project_environment: ProjectEnvironment,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut config = Self {
            runtime: RuntimeConfig::default(),
            provider: ProviderConfig::default(),
            tool: ToolConfig::default(),
            memory: MemoryConfig::default(),
            state: StateConfig::default(),
            api: ApiConfig::default(),
            web: WebConfig::default(),
            routing: RoutingConfig::default(),
            source_summary: ConfigSourceSummary::default(),
            project_environment: ProjectEnvironment::default(),
        };
        // In-memory defaults used by tests and ad-hoc construction include a
        // usable fake profile. Figment loading uses empty profiles so project
        // TOML maps do not merge with built-in defaults.
        ensure_default_provider_profile(&mut config.provider);
        config
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProjectEnvironment {
    values: BTreeMap<String, String>,
}

impl ProjectEnvironment {
    pub(crate) fn values(&self) -> &BTreeMap<String, String> {
        &self.values
    }
}

impl fmt::Debug for ProjectEnvironment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectEnvironment")
            .field("keys", &self.values.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RuntimeConfig {
    pub max_steps: u32,
    pub system_prompt_path: PathBuf,
    pub planner_prompt_path: PathBuf,
    pub model_compaction_enabled: bool,
    pub compaction_failure_threshold: u32,
    pub context_soft_limit_tokens: usize,
    pub context_hard_limit_tokens: usize,
    pub context_reserved_tokens: usize,
    /// Optional multidimensional execution limits. Absent dimensions keep the
    /// deterministic `max_steps` projection, so an existing config file behaves
    /// exactly as before.
    pub execution: ExecutionConfig,
    /// Runtime-owned Agent selection and bounded procedural context settings.
    pub agent: AgentConfig,
}

/// Operator-facing Agent activation settings.
///
/// Workspace content still requires the independent Project Trust
/// `workspace_instructions` capability. Setting these fields cannot grant that
/// capability; an unauthorized request fails during Engine assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AgentConfig {
    /// Fully qualified `<source>:<agent-id>` selector.
    pub selector: String,
    /// Discover root and nested `AGENTS.md` files for the selected workspace.
    pub workspace_instructions: bool,
    /// Admit remediation-mode procedures after all trust/risk checks.
    pub allow_remediation_procedures: bool,
    /// Operator ceiling on selected procedures for one run.
    pub max_procedure_selections: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            selector: AgentSelector::legacy().to_string(),
            workspace_instructions: false,
            allow_remediation_procedures: false,
            max_procedure_selections: 3,
        }
    }
}

/// Operator-facing execution lifecycle configuration.
///
/// Every field is optional and defaults to "not configured", which preserves the
/// deterministic `max_steps`-derived policy. This is a bounded projection into
/// `rove_runtime::execution::ExecutionPolicy`, which remains the sole execution
/// config truth at runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionConfig {
    /// Bounded model evaluation for typed plan ambiguity. Rule-only by default.
    pub evaluator_mode: Option<ExecutionEvaluatorMode>,
    /// Independent finalization authority. Deterministic by default.
    pub finalizer_policy: Option<ExecutionFinalizerPolicy>,
    pub max_plan_steps: Option<u32>,
    pub max_step_attempts: Option<u32>,
    pub max_model_turns: Option<u32>,
    pub max_model_turns_per_step: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub max_tool_calls_per_step: Option<u32>,
    pub max_plan_revisions: Option<u32>,
    pub max_model_repairs: Option<u32>,
    pub max_finalization_turns: Option<u32>,
    pub max_wall_time_ms: Option<u64>,
    pub max_total_tokens: Option<u64>,
    /// Cost enforcement applies only when the active provider supplies priced
    /// usage. Configuring it never fabricates enforcement.
    pub max_cost_microunits: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEvaluatorMode {
    RuleOnly,
    RuleFirstModelOnAmbiguity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionFinalizerPolicy {
    Deterministic,
    ModelPreferred,
}

impl ExecutionConfig {
    /// True when no dimension has been configured, so the deterministic
    /// `max_steps` projection is used unchanged.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Apply configured dimensions on top of the deterministic policy derived
    /// from `max_steps` / `plan_enabled`.
    ///
    /// An unset field leaves the derived value in place rather than clearing it,
    /// so partial configuration cannot silently remove a budget the strategy
    /// projection established.
    pub fn apply_to(&self, mut policy: ExecutionPolicy) -> ExecutionPolicy {
        if let Some(mode) = self.evaluator_mode {
            policy.evaluator_mode = match mode {
                ExecutionEvaluatorMode::RuleOnly => EvaluatorMode::RuleOnly,
                ExecutionEvaluatorMode::RuleFirstModelOnAmbiguity => {
                    EvaluatorMode::RuleFirstModelOnAmbiguity
                }
            };
        }
        if let Some(finalizer) = self.finalizer_policy {
            policy.finalizer_policy = match finalizer {
                ExecutionFinalizerPolicy::Deterministic => FinalizerPolicy::Deterministic,
                ExecutionFinalizerPolicy::ModelPreferred => FinalizerPolicy::ModelPreferred,
            };
        }
        if !self.is_empty() {
            policy.selection_source = StrategySelectionSource::Config;
        }

        let budgets = &mut policy.budgets;
        overlay(&mut budgets.max_plan_steps, self.max_plan_steps);
        overlay(&mut budgets.max_step_attempts, self.max_step_attempts);
        overlay(&mut budgets.max_model_turns, self.max_model_turns);
        overlay(
            &mut budgets.max_model_turns_per_step,
            self.max_model_turns_per_step,
        );
        overlay(&mut budgets.max_tool_calls, self.max_tool_calls);
        overlay(
            &mut budgets.max_tool_calls_per_step,
            self.max_tool_calls_per_step,
        );
        overlay(&mut budgets.max_plan_revisions, self.max_plan_revisions);
        overlay(&mut budgets.max_model_repairs, self.max_model_repairs);
        overlay(
            &mut budgets.max_finalization_turns,
            self.max_finalization_turns,
        );
        overlay(&mut budgets.max_wall_time_ms, self.max_wall_time_ms);
        overlay(&mut budgets.max_total_tokens, self.max_total_tokens);
        overlay(&mut budgets.max_cost_microunits, self.max_cost_microunits);
        policy
    }
}

fn overlay<T: Copy>(slot: &mut Option<T>, configured: Option<T>) {
    if let Some(value) = configured {
        *slot = Some(value);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProviderConfig {
    pub active: Option<String>,
    pub profiles: BTreeMap<String, ProviderProfileConfig>,
    pub fallback_profiles: Vec<String>,
    /// Default/current model id (may be overridden by active profile or CLI/env).
    pub model: String,
    pub fallback_models: Vec<String>,
    pub options: ProviderOptions,
}

fn validate_provider_options(options: &ProviderOptions, prefix: &str) -> anyhow::Result<()> {
    if options.max_tokens == Some(0) {
        anyhow::bail!("{prefix}.max_tokens must be greater than 0");
    }
    validate_finite_option(prefix, "temperature", options.temperature)?;
    validate_finite_option(prefix, "top_p", options.top_p)?;
    validate_finite_option(prefix, "frequency_penalty", options.frequency_penalty)?;
    validate_finite_option(prefix, "presence_penalty", options.presence_penalty)?;
    Ok(())
}

fn validate_finite_option(prefix: &str, field: &str, value: Option<f64>) -> anyhow::Result<()> {
    if let Some(value) = value
        && !value.is_finite()
    {
        anyhow::bail!("{prefix}.{field} must be finite");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ToolConfig {
    pub mcp_config_path: PathBuf,
    pub shell: ShellConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ShellConfig {
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub inherit_environment: bool,
    pub denylist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemoryConfig {
    pub session_dir: PathBuf,
    pub durable_dir: PathBuf,
    pub recall_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StateConfig {
    pub state_dir: PathBuf,
    pub sqlite_path: PathBuf,
    pub lazy_migration: bool,
    pub sqlite_busy_timeout_ms: u64,
    pub allow_external_paths: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ApiConfig {
    pub bind_addr: String,
    pub token_auth: Option<String>,
    pub unsafe_remote_without_auth: bool,
    pub cors_origins: Vec<String>,
    pub rate_limit_per_minute: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WebConfig {
    pub api_base: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RoutingConfig {
    pub failure_threshold: u32,
    pub open_cooldown_ms: u64,
    pub retry_max_attempts: u32,
    pub retry_backoff_base_ms: u64,
    pub retry_backoff_max_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSourceSummary {
    pub workspace_root: PathBuf,
    pub user_config_path: PathBuf,
    pub user_config_present: bool,
    pub user_config_loaded: bool,
    pub user_config_revision: Option<String>,
    pub project_config_path: PathBuf,
    pub project_config_present: bool,
    pub project_config_loaded: bool,
    pub project_activation: ProjectActivationState,
    pub project_activation_source: Option<ProjectActivationSource>,
    pub trusted_workspace_roots: Vec<PathBuf>,
    pub project_trust_identity_digest: Option<String>,
    pub project_trust_invalidated_capabilities: Vec<String>,
    pub project_trust_granted_capabilities: Vec<String>,
    pub env_keys: Vec<String>,
    pub cli_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppConfigOverrides {
    pub model: Option<String>,
    pub max_steps: Option<u32>,
    pub agent_selector: Option<String>,
    pub api_bind_addr: Option<String>,
    pub trust_project: bool,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_steps: 20,
            system_prompt_path: PathBuf::from("prompts/system.md"),
            planner_prompt_path: PathBuf::from("prompts/planner.md"),
            model_compaction_enabled: false,
            compaction_failure_threshold: 3,
            context_soft_limit_tokens: 24_000,
            context_hard_limit_tokens: 30_000,
            context_reserved_tokens: 4_000,
            execution: ExecutionConfig::default(),
            agent: AgentConfig::default(),
        }
    }
}

fn default_fake_provider_profile() -> ProviderProfileConfig {
    ProviderProfileConfig {
        label: Some("Local deterministic Fake".to_string()),
        provider_type: "fake".to_string(),
        base_url: String::new(),
        model: "fake".to_string(),
        auth: crate::provider::ProviderAuthConfig::None,
        headers: BTreeMap::new(),
        options: ProviderOptions::default(),
        protocol_options: serde_json::json!({}),
    }
}

fn ensure_default_provider_profile(provider: &mut ProviderConfig) {
    if !provider.profiles.is_empty() {
        return;
    }
    provider
        .profiles
        .insert("default".to_string(), default_fake_provider_profile());
    if provider.active.is_none() {
        provider.active = Some("default".to_string());
    }
    if provider.model.trim().is_empty() {
        provider.model = "fake".to_string();
    }
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            mcp_config_path: PathBuf::from(".rove/mcp_servers.json"),
            shell: ShellConfig::default(),
        }
    }
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30_000,
            max_output_bytes: 64 * 1024,
            inherit_environment: true,
            denylist: Vec::new(),
        }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            session_dir: PathBuf::from(".rove/memory/sessions"),
            durable_dir: PathBuf::from(".rove/memory"),
            recall_limit: 8,
        }
    }
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            state_dir: PathBuf::from(".rove"),
            sqlite_path: PathBuf::from(".rove/state.sqlite"),
            lazy_migration: true,
            sqlite_busy_timeout_ms: 5_000,
            allow_external_paths: false,
        }
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8787".to_string(),
            token_auth: None,
            unsafe_remote_without_auth: false,
            cors_origins: Vec::new(),
            rate_limit_per_minute: None,
        }
    }
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            api_base: "http://127.0.0.1:8787".to_string(),
        }
    }
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            open_cooldown_ms: 30_000,
            retry_max_attempts: 1,
            retry_backoff_base_ms: 250,
            retry_backoff_max_ms: 5_000,
        }
    }
}

impl Default for ConfigSourceSummary {
    fn default() -> Self {
        let activation = ProjectActivation::programmatic();
        Self {
            workspace_root: PathBuf::from("."),
            user_config_path: crate::user_config::UserConfigPaths::discover().config_file,
            user_config_present: false,
            user_config_loaded: false,
            user_config_revision: None,
            project_config_path: PathBuf::from(".rove/config.toml"),
            project_config_present: false,
            project_config_loaded: false,
            project_activation: activation.state,
            project_activation_source: activation.source,
            trusted_workspace_roots: activation.trusted_workspace_roots,
            project_trust_identity_digest: None,
            project_trust_invalidated_capabilities: Vec::new(),
            project_trust_granted_capabilities: activation
                .granted_capabilities
                .into_iter()
                .collect(),
            env_keys: Vec::new(),
            cli_keys: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn defaults() -> Self {
        // Empty provider profiles so layered TOML maps replace cleanly rather
        // than merging with a built-in fake profile entry.
        Self {
            runtime: RuntimeConfig::default(),
            provider: ProviderConfig::default(),
            tool: ToolConfig::default(),
            memory: MemoryConfig::default(),
            state: StateConfig::default(),
            api: ApiConfig::default(),
            web: WebConfig::default(),
            routing: RoutingConfig::default(),
            source_summary: ConfigSourceSummary::default(),
            project_environment: ProjectEnvironment::default(),
        }
    }

    pub fn load(
        workspace_root: impl AsRef<Path>,
        overrides: AppConfigOverrides,
    ) -> anyhow::Result<Self> {
        let repository = ProjectTrustRepository::operator_default().ok();
        Self::load_with_optional_project_trust_repository(
            workspace_root,
            overrides,
            repository.as_ref(),
            crate::user_config::UserConfigPaths::discover(),
        )
    }

    /// Load configuration from an explicit user catalog location.
    ///
    /// This is useful for embedders and concurrent tests that must not depend
    /// on process-global home-directory overrides.
    pub fn load_with_user_config_paths(
        workspace_root: impl AsRef<Path>,
        overrides: AppConfigOverrides,
        user_paths: crate::user_config::UserConfigPaths,
    ) -> anyhow::Result<Self> {
        let repository = ProjectTrustRepository::operator_default().ok();
        Self::load_with_optional_project_trust_repository(
            workspace_root,
            overrides,
            repository.as_ref(),
            user_paths,
        )
    }

    /// Load configuration against an explicit canonical trust authority.
    /// First-party processes normally use [`Self::load`]; this entry point lets
    /// embedders share the exact repository instance with an API coordinator.
    pub fn load_with_project_trust_repository(
        workspace_root: impl AsRef<Path>,
        overrides: AppConfigOverrides,
        repository: &ProjectTrustRepository,
    ) -> anyhow::Result<Self> {
        Self::load_with_optional_project_trust_repository(
            workspace_root,
            overrides,
            Some(repository),
            crate::user_config::UserConfigPaths::discover(),
        )
    }

    /// Load against explicit Provider and Project Trust authorities.
    pub fn load_with_authorities(
        workspace_root: impl AsRef<Path>,
        overrides: AppConfigOverrides,
        repository: &ProjectTrustRepository,
        user_paths: crate::user_config::UserConfigPaths,
    ) -> anyhow::Result<Self> {
        Self::load_with_optional_project_trust_repository(
            workspace_root,
            overrides,
            Some(repository),
            user_paths,
        )
    }

    fn load_with_optional_project_trust_repository(
        workspace_root: impl AsRef<Path>,
        overrides: AppConfigOverrides,
        repository: Option<&ProjectTrustRepository>,
        user_paths: crate::user_config::UserConfigPaths,
    ) -> anyhow::Result<Self> {
        let explicit_fake = overrides.model.as_deref() == Some("fake")
            || std::env::var("ROVE_MODEL").ok().as_deref() == Some("fake");
        let workspace_root = workspace_root
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.as_ref().to_path_buf());
        // Resolve temporary grants and durable operator trust before reading a
        // workspace-owned .env file so the repository cannot grant trust to
        // itself. Temporary grants remain process-scoped and are never written
        // to the durable repository implicitly.
        let trusted_workspaces = std::env::var_os(TRUSTED_WORKSPACES_ENV);
        let temporary_activation = ProjectActivation::resolve(
            &workspace_root,
            overrides.trust_project,
            trusted_workspaces,
        )?;
        let capability_digests = capability_digest_map(&workspace_root, None, None);
        let durable_resolution = repository
            .and_then(|repository| {
                repository
                    .resolve(
                        &workspace_root,
                        workspace_kind_for_root(&workspace_root),
                        &capability_digests,
                    )
                    .ok()
            })
            .unwrap_or_else(|| crate::project_trust::ProjectTrustResolution {
                state: ProjectActivationState::Restricted,
                identity_digest: crate::project_trust::workspace_identity_digest(
                    &workspace_root,
                    workspace_kind_for_root(&workspace_root),
                ),
                invalidated_capabilities: Vec::new(),
                granted_capabilities: Default::default(),
            });
        let activation = if temporary_activation.state == ProjectActivationState::Trusted {
            temporary_activation
        } else {
            ProjectActivation::durable(durable_resolution.clone(), Vec::new())
        };
        let capability_allowed = |capability: &str| {
            activation.state == ProjectActivationState::Trusted
                && (activation.source != Some(ProjectActivationSource::Durable)
                    || activation.granted_capabilities.contains(capability))
        };
        let project_config_granted =
            capability_allowed(crate::project_trust::CAP_PROJECT_CONFIGURATION);
        let workspace_instructions_granted =
            capability_allowed(crate::project_trust::CAP_WORKSPACE_INSTRUCTIONS);
        let provider_credentials_granted =
            capability_allowed(crate::project_trust::CAP_PROVIDER_CREDENTIALS);
        let mcp_processes_granted = capability_allowed(crate::project_trust::CAP_MCP_PROCESSES);
        let external_paths_granted = capability_allowed(crate::project_trust::CAP_EXTERNAL_PATHS);
        let project_environment_values = load_project_environment(
            &workspace_root,
            project_config_granted
                || provider_credentials_granted
                || mcp_processes_granted
                || external_paths_granted,
        )?;
        let project_config_path = workspace_root.join(".rove/config.toml");
        let project_config_present = project_config_path.exists();
        let safe_project_config_path =
            bounded_workspace_config_path(&workspace_root, ".rove/config.toml");
        let project_config_loaded = project_config_granted && safe_project_config_path.is_some();

        let env_layer = env_layer(
            &project_environment_values,
            ProjectEnvironmentCapabilities {
                project_configuration: project_config_granted,
                workspace_instructions: workspace_instructions_granted,
                provider_credentials: provider_credentials_granted,
                mcp_processes: mcp_processes_granted,
                external_paths: external_paths_granted,
            },
        )?;
        let env_keys = env_layer.keys.clone();
        let cli_layer = overrides.into_layer();
        let cli_keys = cli_layer.keys.clone();

        let user_config_present = user_paths.config_file.is_file();
        let user_document = crate::user_config::UserConfigLoader::new(user_paths.clone())
            .load_or_default()
            .map_err(anyhow::Error::from)?;
        let user_config_loaded = user_config_present;
        let user_config_revision = user_config_loaded.then(|| user_document.revision());
        let user_layer = user_document_to_app_layer(&user_document);

        let mut figment = Figment::from(Serialized::defaults(AppConfig::defaults()));
        figment = figment.merge(Serialized::defaults(user_layer));
        if let Some(path) = safe_project_config_path.filter(|_| project_config_loaded) {
            let project_layer = filtered_project_config(
                &path,
                workspace_instructions_granted,
                provider_credentials_granted,
                mcp_processes_granted,
                external_paths_granted,
            )?;
            figment = figment.merge(Serialized::defaults(project_layer));
        }
        figment = figment.merge(Serialized::defaults(env_layer.config));
        figment = figment.merge(Serialized::defaults(cli_layer.config));

        let mut config: AppConfig = figment.extract()?;
        // Programmatic AppConfig::default remains deterministic. Product loads
        // get Fake only when the invocation explicitly selected it.
        if explicit_fake && config.provider.profiles.is_empty() {
            ensure_default_provider_profile(&mut config.provider);
        }
        config.source_summary = ConfigSourceSummary {
            workspace_root,
            user_config_path: user_paths.config_file,
            user_config_present,
            user_config_loaded,
            user_config_revision,
            project_config_path,
            project_config_present,
            project_config_loaded,
            project_activation: activation.state,
            project_activation_source: activation.source,
            trusted_workspace_roots: activation.trusted_workspace_roots,
            project_trust_identity_digest: Some(durable_resolution.identity_digest),
            project_trust_invalidated_capabilities: durable_resolution.invalidated_capabilities,
            project_trust_granted_capabilities: activation
                .granted_capabilities
                .into_iter()
                .collect(),
            env_keys,
            cli_keys,
        };
        config.project_environment = ProjectEnvironment {
            values: if provider_credentials_granted {
                project_environment_values
            } else {
                BTreeMap::new()
            },
        };
        config.normalize_active_profile_model();
        config.validate()?;
        Ok(config)
    }

    pub fn from_env() -> anyhow::Result<Self> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::load(cwd, AppConfigOverrides::default())
    }

    pub fn load_system_prompt(&self) -> String {
        let path = self.resolve_path(&self.runtime.system_prompt_path);
        std::fs::read_to_string(path).unwrap_or_else(|_| {
            "You are rove, a helpful assistant that can use tools to accomplish tasks.".to_string()
        })
    }

    pub fn load_planner_prompt(&self) -> String {
        let path = self.resolve_path(&self.runtime.planner_prompt_path);
        std::fs::read_to_string(path)
            .unwrap_or_else(|_| rove_runtime::planner::DEFAULT_PLANNER_PROMPT.to_string())
    }

    pub fn shell_policy(&self) -> rove_runtime::tools::shell::ShellPolicy {
        rove_runtime::tools::shell::ShellPolicy {
            timeout_ms: self.tool.shell.timeout_ms,
            max_output_bytes: self.tool.shell.max_output_bytes,
            inherit_environment: self.tool.shell.inherit_environment,
            denylist: self.tool.shell.denylist.clone(),
        }
    }

    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.source_summary.workspace_root.join(path)
        }
    }

    pub fn state_dir(&self) -> PathBuf {
        self.resolve_path(&self.state.state_dir)
    }

    pub fn sqlite_path(&self) -> PathBuf {
        self.resolve_path(&self.state.sqlite_path)
    }

    pub fn memory_paths(&self) -> MemoryPaths {
        MemoryPaths {
            session_dir: self.resolve_path(&self.memory.session_dir),
            durable_dir: self.resolve_path(&self.memory.durable_dir),
            recall_limit: self.memory.recall_limit,
        }
    }

    /// Resolve durable memory only when it remains inside the current workspace.
    ///
    /// Product control surfaces use this stricter path even when the process is
    /// configured to allow other explicitly external runtime paths.
    pub fn workspace_bounded_durable_memory_dir(&self) -> anyhow::Result<PathBuf> {
        let resolved = self.normalized_workspace_path(&self.memory.durable_dir);
        if !resolved.starts_with(&self.source_summary.workspace_root) {
            anyhow::bail!("memory.durable_dir resolves outside the selected workspace");
        }
        Ok(resolved)
    }

    /// Resolve the product-managed MCP catalog only inside the selected workspace.
    pub fn workspace_bounded_mcp_config_path(&self) -> anyhow::Result<PathBuf> {
        let resolved = self.normalized_workspace_path(&self.tool.mcp_config_path);
        let canonical_workspace_root = self.source_summary.workspace_root.canonicalize()?;
        let mut ancestor = resolved.parent().ok_or_else(|| {
            anyhow::anyhow!("tool.mcp_config_path has no workspace-bounded parent")
        })?;
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .ok_or_else(|| anyhow::anyhow!("tool.mcp_config_path has no existing ancestor"))?;
        }
        let canonical_ancestor = ancestor.canonicalize()?;
        if !canonical_ancestor.starts_with(canonical_workspace_root) {
            anyhow::bail!("tool.mcp_config_path resolves outside the selected workspace");
        }
        Ok(resolved)
    }

    pub fn rebase_to_workspace(&mut self, workspace_root: impl AsRef<Path>) {
        let workspace_root = workspace_root
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.as_ref().to_path_buf());
        self.source_summary.workspace_root = workspace_root.clone();
        self.source_summary.project_config_path = workspace_root.join(".rove/config.toml");
        self.source_summary.project_config_present =
            self.source_summary.project_config_path.exists();
        self.source_summary.project_config_loaded = false;
        let temporary = ProjectActivation {
            state: self.source_summary.project_activation,
            source: self.source_summary.project_activation_source,
            trusted_workspace_roots: self.source_summary.trusted_workspace_roots.clone(),
            granted_capabilities: crate::project_trust::all_capability_names()
                .into_iter()
                .collect(),
        }
        .for_workspace(&workspace_root);
        let durable = ProjectTrustRepository::operator_default()
            .and_then(|repository| {
                repository.resolve(
                    &workspace_root,
                    workspace_kind_for_root(&workspace_root),
                    &capability_digest_map(&workspace_root, None, None),
                )
            })
            .ok();
        let activation = if temporary.state == ProjectActivationState::Trusted {
            temporary
        } else if let Some(resolution) = durable.clone() {
            ProjectActivation::durable(resolution, Vec::new())
        } else {
            temporary
        };
        self.source_summary.project_activation = activation.state;
        self.source_summary.project_activation_source = activation.source;
        self.source_summary.trusted_workspace_roots = activation.trusted_workspace_roots;
        self.source_summary.project_trust_identity_digest = durable
            .as_ref()
            .map(|resolution| resolution.identity_digest.clone());
        self.source_summary.project_trust_invalidated_capabilities = durable
            .as_ref()
            .map(|resolution| resolution.invalidated_capabilities.clone())
            .unwrap_or_default();
        self.source_summary.project_trust_granted_capabilities =
            activation.granted_capabilities.into_iter().collect();
    }

    pub fn project_activation_state(&self) -> ProjectActivationState {
        self.source_summary.project_activation
    }

    pub fn project_activation_allowed(&self) -> bool {
        self.project_activation_state() == ProjectActivationState::Trusted
    }

    pub fn project_capability_allowed(&self, capability: &str) -> bool {
        self.project_activation_allowed()
            && (self.source_summary.project_activation_source
                != Some(ProjectActivationSource::Durable)
                || self
                    .source_summary
                    .project_trust_granted_capabilities
                    .iter()
                    .any(|granted| granted == capability))
    }

    pub fn apply_project_trust_resolution(
        &mut self,
        resolution: crate::project_trust::ProjectTrustResolution,
    ) {
        let temporary_grant = self.project_activation_allowed()
            && self.source_summary.project_activation_source
                != Some(ProjectActivationSource::Durable);
        if temporary_grant
            && matches!(
                resolution.state,
                ProjectActivationState::Unknown | ProjectActivationState::Restricted
            )
        {
            self.source_summary.project_trust_identity_digest = Some(resolution.identity_digest);
            self.source_summary.project_trust_invalidated_capabilities =
                resolution.invalidated_capabilities;
            return;
        }
        let activation_state = match resolution.state {
            // `unknown` is the operator-store truth exposed by the trust API;
            // execution remains fail-closed until an explicit grant exists.
            ProjectActivationState::Unknown => ProjectActivationState::Restricted,
            state => state,
        };
        self.source_summary.project_activation = activation_state;
        self.source_summary.project_activation_source = (activation_state
            == ProjectActivationState::Trusted)
            .then_some(ProjectActivationSource::Durable);
        self.source_summary.project_trust_identity_digest = Some(resolution.identity_digest);
        self.source_summary.project_trust_invalidated_capabilities =
            resolution.invalidated_capabilities;
        self.source_summary.project_trust_granted_capabilities =
            resolution.granted_capabilities.into_iter().collect();
    }

    fn normalize_active_profile_model(&mut self) {
        let model_overridden = self
            .source_summary
            .env_keys
            .iter()
            .any(|key| key == "ROVE_MODEL")
            || self
                .source_summary
                .cli_keys
                .iter()
                .any(|key| key == "provider.model");
        if model_overridden || !self.provider.model.trim().is_empty() {
            return;
        }
        let Some(active) = self.provider.active.as_deref() else {
            return;
        };
        if let Some(profile) = self.provider.profiles.get(active) {
            self.provider.model = profile.model.clone();
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        self.validate_provider_config()?;
        if !self.provider.profiles.is_empty() && self.provider.model.trim().is_empty() {
            anyhow::bail!("provider.model must not be empty");
        }
        if self
            .provider
            .fallback_models
            .iter()
            .any(|model| model.trim().is_empty())
        {
            anyhow::bail!("provider.fallback_models must not contain empty model names");
        }
        validate_provider_options(&self.provider.options, "provider.options")?;
        if self.runtime.max_steps == 0 {
            anyhow::bail!("runtime.max_steps must be greater than 0");
        }
        AgentSelector::parse(&self.runtime.agent.selector)
            .map_err(|error| anyhow::anyhow!("runtime.agent.selector is invalid: {error}"))?;
        if self.runtime.agent.max_procedure_selections == 0 {
            anyhow::bail!("runtime.agent.max_procedure_selections must be greater than 0");
        }
        if self.runtime.compaction_failure_threshold == 0 {
            anyhow::bail!("runtime.compaction_failure_threshold must be greater than 0");
        }
        self.validate_execution_config()?;
        if self.runtime.context_reserved_tokens >= self.runtime.context_hard_limit_tokens {
            anyhow::bail!(
                "runtime.context_reserved_tokens must be less than context_hard_limit_tokens"
            );
        }
        if self.runtime.context_soft_limit_tokens >= self.runtime.context_hard_limit_tokens {
            anyhow::bail!(
                "runtime.context_soft_limit_tokens must be less than context_hard_limit_tokens"
            );
        }
        if self.routing.failure_threshold == 0 {
            anyhow::bail!("routing.failure_threshold must be greater than 0");
        }
        if self.routing.open_cooldown_ms == 0 {
            anyhow::bail!("routing.open_cooldown_ms must be greater than 0");
        }
        if self.routing.retry_max_attempts == 0 {
            anyhow::bail!("routing.retry_max_attempts must be greater than 0");
        }
        if self.routing.retry_backoff_max_ms < self.routing.retry_backoff_base_ms {
            anyhow::bail!(
                "routing.retry_backoff_max_ms must be greater than or equal to retry_backoff_base_ms"
            );
        }
        if self.state.sqlite_busy_timeout_ms == 0 {
            anyhow::bail!("state.sqlite_busy_timeout_ms must be greater than 0");
        }
        if self.memory.recall_limit == 0 {
            anyhow::bail!("memory.recall_limit must be greater than 0");
        }
        self.validate_api_remote_mode()?;
        self.validate_workspace_paths()?;
        Ok(())
    }

    fn validate_provider_config(&self) -> anyhow::Result<()> {
        if self.provider.profiles.is_empty() {
            if self.provider.active.is_some()
                || !self.provider.fallback_profiles.is_empty()
                || !self.provider.fallback_models.is_empty()
            {
                anyhow::bail!(
                    "provider.profiles is required when provider selection or fallback fields are configured; flat provider.name/api_base/api_key config is no longer supported"
                );
            }
            return Ok(());
        }
        let active =
            self.provider.active.as_deref().ok_or_else(|| {
                anyhow::anyhow!("provider.active is required when profiles exist")
            })?;
        validate_profile_name(active, "provider.active")?;
        if !self.provider.profiles.contains_key(active) {
            anyhow::bail!("provider.active references unknown profile `{active}`");
        }
        let mut seen_fallbacks = HashSet::new();
        for fallback in &self.provider.fallback_profiles {
            validate_profile_name(fallback, "provider.fallback_profiles")?;
            if fallback == active {
                anyhow::bail!("provider.fallback_profiles must not contain the active profile");
            }
            if !seen_fallbacks.insert(fallback) {
                anyhow::bail!("provider.fallback_profiles must not contain duplicates");
            }
            if !self.provider.profiles.contains_key(fallback) {
                anyhow::bail!("provider.fallback_profiles references unknown profile `{fallback}`");
            }
        }
        for (name, profile) in &self.provider.profiles {
            validate_profile_name(name, "provider.profiles")?;
            profile
                .validate(
                    &self.source_summary.workspace_root,
                    self.state.allow_external_paths,
                )
                .map_err(|error| {
                    anyhow::anyhow!("provider profile `{name}` is invalid: {error}")
                })?;
        }
        Ok(())
    }

    /// Reject an execution configuration that the runtime policy would refuse.
    ///
    /// Validating here keeps a bad budget a startup error with a config-shaped
    /// message instead of a mid-run refusal.
    fn validate_execution_config(&self) -> anyhow::Result<()> {
        let execution = &self.runtime.execution;
        for (field, value) in [
            ("max_plan_steps", execution.max_plan_steps),
            ("max_step_attempts", execution.max_step_attempts),
            ("max_model_turns", execution.max_model_turns),
            (
                "max_model_turns_per_step",
                execution.max_model_turns_per_step,
            ),
            ("max_tool_calls", execution.max_tool_calls),
            ("max_tool_calls_per_step", execution.max_tool_calls_per_step),
            ("max_plan_revisions", execution.max_plan_revisions),
            ("max_model_repairs", execution.max_model_repairs),
            ("max_finalization_turns", execution.max_finalization_turns),
        ] {
            if value == Some(0) {
                anyhow::bail!("runtime.execution.{field} must be greater than 0");
            }
        }
        for (field, value) in [
            ("max_wall_time_ms", execution.max_wall_time_ms),
            ("max_total_tokens", execution.max_total_tokens),
            ("max_cost_microunits", execution.max_cost_microunits),
        ] {
            if value == Some(0) {
                anyhow::bail!("runtime.execution.{field} must be greater than 0");
            }
        }

        // The resolved policy is the authority, so validate the exact value the
        // runtime will receive rather than the config fragment alone.
        let policy = execution.apply_to(ExecutionPolicy::from_max_steps_and_plan_flag(
            self.runtime.max_steps,
            true,
        ));
        policy
            .validate()
            .map_err(|error| anyhow::anyhow!("runtime.execution is invalid: {error}"))
    }

    fn validate_api_remote_mode(&self) -> anyhow::Result<()> {
        let addr: SocketAddr = self
            .api
            .bind_addr
            .parse()
            .map_err(|err| anyhow::anyhow!("api.bind_addr is invalid: {err}"))?;
        if !addr.ip().is_loopback()
            && self
                .api
                .token_auth
                .as_deref()
                .unwrap_or_default()
                .is_empty()
            && !self.api.unsafe_remote_without_auth
        {
            anyhow::bail!(
                "api.token_auth is required when api.bind_addr is remote; set api.unsafe_remote_without_auth=true to override"
            );
        }
        Ok(())
    }

    fn validate_workspace_paths(&self) -> anyhow::Result<()> {
        if self.state.allow_external_paths {
            return Ok(());
        }
        for (name, path) in [
            (
                "runtime.system_prompt_path",
                &self.runtime.system_prompt_path,
            ),
            (
                "runtime.planner_prompt_path",
                &self.runtime.planner_prompt_path,
            ),
            ("tool.mcp_config_path", &self.tool.mcp_config_path),
            ("state.state_dir", &self.state.state_dir),
            ("state.sqlite_path", &self.state.sqlite_path),
            ("memory.session_dir", &self.memory.session_dir),
            ("memory.durable_dir", &self.memory.durable_dir),
        ] {
            self.validate_workspace_path(name, path)?;
        }
        Ok(())
    }

    fn validate_workspace_path(&self, name: &str, path: &Path) -> anyhow::Result<()> {
        let resolved = self.normalized_workspace_path(path);
        if !resolved.starts_with(&self.source_summary.workspace_root) {
            anyhow::bail!(
                "{name} resolves outside the workspace; set state.allow_external_paths=true to allow it"
            );
        }
        Ok(())
    }

    fn normalized_workspace_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            normalize_path(path)
        } else {
            normalize_path(self.source_summary.workspace_root.join(path))
        }
    }
}

fn validate_profile_name(name: &str, field: &str) -> anyhow::Result<()> {
    crate::provider_catalog::ProviderProfileId::new(name.to_string())
        .map(|_| ())
        .map_err(|_| {
            anyhow::anyhow!("{field} must use 1-128 ASCII letters, digits, '-', '_', or '.'")
        })
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    match path.canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => normalize_lexical_path(path),
    }
}

fn workspace_kind_for_root(root: &Path) -> WorkspaceKind {
    if root.join(".git").exists() {
        WorkspaceKind::Repo
    } else {
        WorkspaceKind::Folder
    }
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[derive(Debug, Default, Serialize)]
struct AppConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<RuntimeConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<ToolConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory: Option<MemoryConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<StateConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api: Option<ApiConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    web: Option<WebConfigLayer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    routing: Option<RoutingConfigLayer>,
}

#[derive(Debug, Default, Serialize)]
struct RuntimeConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_prompt_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    planner_prompt_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_compaction_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    compaction_failure_threshold: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_soft_limit_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_hard_limit_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_reserved_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<AgentConfigLayer>,
}

#[derive(Debug, Default, Serialize)]
struct AgentConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    selector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_instructions: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allow_remediation_procedures: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_procedure_selections: Option<u32>,
}

#[derive(Debug, Default, Serialize)]
struct ProviderConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profiles: Option<BTreeMap<String, ProviderProfileConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_profiles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<ProviderOptions>,
}

#[derive(Debug, Default, Serialize)]
struct ToolConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_config_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell: Option<ShellConfigLayer>,
}

#[derive(Debug, Default, Serialize)]
struct ShellConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inherit_environment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    denylist: Option<Vec<String>>,
}

#[derive(Debug, Default, Serialize)]
struct MemoryConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    durable_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recall_limit: Option<usize>,
}

#[derive(Debug, Default, Serialize)]
struct StateConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    state_dir: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sqlite_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lazy_migration: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sqlite_busy_timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allow_external_paths: Option<bool>,
}

#[derive(Debug, Default, Serialize)]
struct ApiConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    bind_addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_auth: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unsafe_remote_without_auth: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cors_origins: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rate_limit_per_minute: Option<u32>,
}

#[derive(Debug, Default, Serialize)]
struct WebConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    api_base: Option<String>,
}

#[derive(Debug, Default, Serialize)]
struct RoutingConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_threshold: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    open_cooldown_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_max_attempts: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_backoff_base_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry_backoff_max_ms: Option<u64>,
}

#[derive(Debug, Default)]
struct NamedConfigLayer {
    config: AppConfigLayer,
    keys: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct ProjectEnvironmentCapabilities {
    project_configuration: bool,
    workspace_instructions: bool,
    provider_credentials: bool,
    mcp_processes: bool,
    external_paths: bool,
}

fn user_document_to_app_layer(document: &crate::user_config::UserConfigDocument) -> AppConfigLayer {
    AppConfigLayer {
        provider: Some(ProviderConfigLayer {
            active: document.model.default_profile.clone(),
            profiles: (!document.provider.profiles.is_empty())
                .then(|| document.provider.profiles.clone()),
            fallback_profiles: (!document.provider.fallback_profiles.is_empty())
                .then(|| document.provider.fallback_profiles.clone()),
            model: document.model.default_model.clone(),
            ..ProviderConfigLayer::default()
        }),
        ..AppConfigLayer::default()
    }
}

impl AppConfigOverrides {
    fn into_layer(self) -> NamedConfigLayer {
        let mut keys = Vec::new();
        if self.trust_project {
            keys.push("project.trust".to_string());
        }
        let mut runtime = RuntimeConfigLayer::default();
        if let Some(max_steps) = self.max_steps {
            runtime.max_steps = Some(max_steps);
            keys.push("runtime.max_steps".to_string());
        }
        if let Some(selector) = self.agent_selector {
            runtime.agent = Some(AgentConfigLayer {
                selector: Some(selector),
                ..AgentConfigLayer::default()
            });
            keys.push("runtime.agent.selector".to_string());
        }

        let mut provider = ProviderConfigLayer::default();
        if let Some(model) = self.model {
            provider.model = Some(model);
            keys.push("provider.model".to_string());
        }

        let mut api = ApiConfigLayer::default();
        if let Some(bind_addr) = self.api_bind_addr {
            api.bind_addr = Some(bind_addr);
            keys.push("api.bind_addr".to_string());
        }

        NamedConfigLayer {
            config: AppConfigLayer {
                runtime: Some(runtime).filter(has_runtime_values),
                provider: Some(provider).filter(has_provider_values),
                api: Some(api).filter(has_api_values),
                ..AppConfigLayer::default()
            },
            keys,
        }
    }
}

fn env_layer(
    project_environment: &BTreeMap<String, String>,
    capabilities: ProjectEnvironmentCapabilities,
) -> anyhow::Result<NamedConfigLayer> {
    let env_string = |name: &str| {
        process_env_string(name).or_else(|| {
            project_environment_value_allowed(name, capabilities)
                .then(|| project_environment.get(name).cloned())
                .flatten()
                .filter(|value| !value.trim().is_empty())
        })
    };
    let mut keys = Vec::new();
    let mut runtime = RuntimeConfigLayer::default();
    if let Some(value) = env_string("ROVE_MAX_STEPS") {
        runtime.max_steps = Some(parse_env("ROVE_MAX_STEPS", &value)?);
        keys.push("ROVE_MAX_STEPS".to_string());
    }
    if let Some(value) = env_string("ROVE_SYSTEM_PROMPT") {
        runtime.system_prompt_path = Some(PathBuf::from(value));
        keys.push("ROVE_SYSTEM_PROMPT".to_string());
    }
    if let Some(value) = env_string("ROVE_PLANNER_PROMPT") {
        runtime.planner_prompt_path = Some(PathBuf::from(value));
        keys.push("ROVE_PLANNER_PROMPT".to_string());
    }
    if let Some(value) = env_string("ROVE_MODEL_COMPACTION_ENABLED") {
        runtime.model_compaction_enabled =
            Some(parse_env_bool("ROVE_MODEL_COMPACTION_ENABLED", &value)?);
        keys.push("ROVE_MODEL_COMPACTION_ENABLED".to_string());
    }
    if let Some(value) = env_string("ROVE_COMPACTION_FAILURE_THRESHOLD") {
        runtime.compaction_failure_threshold =
            Some(parse_env("ROVE_COMPACTION_FAILURE_THRESHOLD", &value)?);
        keys.push("ROVE_COMPACTION_FAILURE_THRESHOLD".to_string());
    }
    if let Some(value) = env_string("ROVE_CONTEXT_SOFT_LIMIT_TOKENS") {
        runtime.context_soft_limit_tokens =
            Some(parse_env("ROVE_CONTEXT_SOFT_LIMIT_TOKENS", &value)?);
        keys.push("ROVE_CONTEXT_SOFT_LIMIT_TOKENS".to_string());
    }
    if let Some(value) = env_string("ROVE_CONTEXT_HARD_LIMIT_TOKENS") {
        runtime.context_hard_limit_tokens =
            Some(parse_env("ROVE_CONTEXT_HARD_LIMIT_TOKENS", &value)?);
        keys.push("ROVE_CONTEXT_HARD_LIMIT_TOKENS".to_string());
    }
    if let Some(value) = env_string("ROVE_CONTEXT_RESERVED_TOKENS") {
        runtime.context_reserved_tokens = Some(parse_env("ROVE_CONTEXT_RESERVED_TOKENS", &value)?);
        keys.push("ROVE_CONTEXT_RESERVED_TOKENS".to_string());
    }
    let mut agent = AgentConfigLayer::default();
    if let Some(value) = env_string("ROVE_AGENT") {
        agent.selector = Some(value);
        keys.push("ROVE_AGENT".to_string());
    }
    if let Some(value) = env_string("ROVE_WORKSPACE_INSTRUCTIONS") {
        agent.workspace_instructions = Some(parse_env_bool("ROVE_WORKSPACE_INSTRUCTIONS", &value)?);
        keys.push("ROVE_WORKSPACE_INSTRUCTIONS".to_string());
    }
    if let Some(value) = env_string("ROVE_ALLOW_REMEDIATION_PROCEDURES") {
        agent.allow_remediation_procedures =
            Some(parse_env_bool("ROVE_ALLOW_REMEDIATION_PROCEDURES", &value)?);
        keys.push("ROVE_ALLOW_REMEDIATION_PROCEDURES".to_string());
    }
    if let Some(value) = env_string("ROVE_MAX_PROCEDURE_SELECTIONS") {
        agent.max_procedure_selections = Some(parse_env("ROVE_MAX_PROCEDURE_SELECTIONS", &value)?);
        keys.push("ROVE_MAX_PROCEDURE_SELECTIONS".to_string());
    }
    runtime.agent = Some(agent).filter(has_agent_values);

    let mut provider = ProviderConfigLayer::default();
    if let Some(value) = env_string("ROVE_PROVIDER_ACTIVE") {
        provider.active = Some(value);
        keys.push("ROVE_PROVIDER_ACTIVE".to_string());
    }
    if let Some(value) = env_string("ROVE_PROVIDER_PROFILES") {
        provider.profiles = Some(serde_json::from_str(&value)?);
        keys.push("ROVE_PROVIDER_PROFILES".to_string());
    }
    if let Some(value) = env_string("ROVE_PROVIDER_FALLBACK_PROFILES") {
        provider.fallback_profiles = Some(parse_csv(&value));
        keys.push("ROVE_PROVIDER_FALLBACK_PROFILES".to_string());
    }
    if let Some(value) = env_string("ROVE_MODEL") {
        provider.model = Some(value);
        keys.push("ROVE_MODEL".to_string());
    }
    if let Some(value) = env_string("ROVE_FALLBACK_MODELS") {
        provider.fallback_models = Some(parse_csv(&value));
        keys.push("ROVE_FALLBACK_MODELS".to_string());
    }

    let mut routing = RoutingConfigLayer::default();
    if let Some(value) = env_string("ROVE_ROUTING_FAILURE_THRESHOLD") {
        routing.failure_threshold = Some(parse_env("ROVE_ROUTING_FAILURE_THRESHOLD", &value)?);
        keys.push("ROVE_ROUTING_FAILURE_THRESHOLD".to_string());
    }
    if let Some(value) = env_string("ROVE_ROUTING_OPEN_COOLDOWN_MS") {
        routing.open_cooldown_ms = Some(parse_env("ROVE_ROUTING_OPEN_COOLDOWN_MS", &value)?);
        keys.push("ROVE_ROUTING_OPEN_COOLDOWN_MS".to_string());
    }
    if let Some(value) = env_string("ROVE_ROUTING_RETRY_MAX_ATTEMPTS") {
        routing.retry_max_attempts = Some(parse_env("ROVE_ROUTING_RETRY_MAX_ATTEMPTS", &value)?);
        keys.push("ROVE_ROUTING_RETRY_MAX_ATTEMPTS".to_string());
    }
    if let Some(value) = env_string("ROVE_ROUTING_RETRY_BACKOFF_BASE_MS") {
        routing.retry_backoff_base_ms =
            Some(parse_env("ROVE_ROUTING_RETRY_BACKOFF_BASE_MS", &value)?);
        keys.push("ROVE_ROUTING_RETRY_BACKOFF_BASE_MS".to_string());
    }
    if let Some(value) = env_string("ROVE_ROUTING_RETRY_BACKOFF_MAX_MS") {
        routing.retry_backoff_max_ms =
            Some(parse_env("ROVE_ROUTING_RETRY_BACKOFF_MAX_MS", &value)?);
        keys.push("ROVE_ROUTING_RETRY_BACKOFF_MAX_MS".to_string());
    }

    let mut tool = ToolConfigLayer::default();
    if let Some(value) = env_string("ROVE_MCP_CONFIG") {
        tool.mcp_config_path = Some(PathBuf::from(value));
        keys.push("ROVE_MCP_CONFIG".to_string());
    }
    let mut shell = ShellConfigLayer::default();
    if let Some(value) = env_string("ROVE_SHELL_TIMEOUT_MS") {
        shell.timeout_ms = Some(parse_env("ROVE_SHELL_TIMEOUT_MS", &value)?);
        keys.push("ROVE_SHELL_TIMEOUT_MS".to_string());
    }
    if let Some(value) = env_string("ROVE_SHELL_MAX_OUTPUT_BYTES") {
        shell.max_output_bytes = Some(parse_env("ROVE_SHELL_MAX_OUTPUT_BYTES", &value)?);
        keys.push("ROVE_SHELL_MAX_OUTPUT_BYTES".to_string());
    }
    if let Some(value) = env_string("ROVE_SHELL_INHERIT_ENVIRONMENT") {
        shell.inherit_environment = Some(parse_env_bool("ROVE_SHELL_INHERIT_ENVIRONMENT", &value)?);
        keys.push("ROVE_SHELL_INHERIT_ENVIRONMENT".to_string());
    }
    if let Some(value) = env_string("ROVE_SHELL_DENYLIST") {
        shell.denylist = Some(parse_csv(&value));
        keys.push("ROVE_SHELL_DENYLIST".to_string());
    }
    tool.shell = Some(shell).filter(has_shell_values);

    let mut memory = MemoryConfigLayer::default();
    if let Some(value) = env_string("ROVE_MEMORY_SESSION_DIR") {
        memory.session_dir = Some(PathBuf::from(value));
        keys.push("ROVE_MEMORY_SESSION_DIR".to_string());
    }
    if let Some(value) = env_string("ROVE_MEMORY_DURABLE_DIR") {
        memory.durable_dir = Some(PathBuf::from(value));
        keys.push("ROVE_MEMORY_DURABLE_DIR".to_string());
    }
    if let Some(value) = env_string("ROVE_MEMORY_RECALL_LIMIT") {
        memory.recall_limit = Some(parse_env("ROVE_MEMORY_RECALL_LIMIT", &value)?);
        keys.push("ROVE_MEMORY_RECALL_LIMIT".to_string());
    }

    let mut state = StateConfigLayer::default();
    if let Some(value) = env_string("ROVE_STATE_DIR") {
        state.state_dir = Some(PathBuf::from(value));
        keys.push("ROVE_STATE_DIR".to_string());
    }
    if let Some(value) = env_string("ROVE_STATE_SQLITE") {
        state.sqlite_path = Some(PathBuf::from(value));
        keys.push("ROVE_STATE_SQLITE".to_string());
    }
    if let Some(value) = env_string("ROVE_STATE_LAZY_MIGRATION") {
        state.lazy_migration = Some(parse_env_bool("ROVE_STATE_LAZY_MIGRATION", &value)?);
        keys.push("ROVE_STATE_LAZY_MIGRATION".to_string());
    }
    if let Some(value) = env_string("ROVE_STATE_SQLITE_BUSY_TIMEOUT_MS") {
        state.sqlite_busy_timeout_ms =
            Some(parse_env("ROVE_STATE_SQLITE_BUSY_TIMEOUT_MS", &value)?);
        keys.push("ROVE_STATE_SQLITE_BUSY_TIMEOUT_MS".to_string());
    }
    if let Some(value) = env_string("ROVE_STATE_ALLOW_EXTERNAL_PATHS") {
        state.allow_external_paths =
            Some(parse_env_bool("ROVE_STATE_ALLOW_EXTERNAL_PATHS", &value)?);
        keys.push("ROVE_STATE_ALLOW_EXTERNAL_PATHS".to_string());
    }

    let mut api = ApiConfigLayer::default();
    if let Some(value) = env_string("ROVE_API_BIND_ADDR") {
        api.bind_addr = Some(value);
        keys.push("ROVE_API_BIND_ADDR".to_string());
    }
    if let Some(value) = env_string("ROVE_API_TOKEN") {
        api.token_auth = Some(value);
        keys.push("ROVE_API_TOKEN".to_string());
    }
    if let Some(value) = env_string("ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH") {
        api.unsafe_remote_without_auth = Some(parse_env_bool(
            "ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH",
            &value,
        )?);
        keys.push("ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH".to_string());
    }
    if let Some(value) = env_string("ROVE_API_CORS_ORIGINS") {
        api.cors_origins = Some(parse_csv(&value));
        keys.push("ROVE_API_CORS_ORIGINS".to_string());
    }
    if let Some(value) = env_string("ROVE_API_RATE_LIMIT_PER_MINUTE") {
        api.rate_limit_per_minute = Some(parse_env("ROVE_API_RATE_LIMIT_PER_MINUTE", &value)?);
        keys.push("ROVE_API_RATE_LIMIT_PER_MINUTE".to_string());
    }

    let mut web = WebConfigLayer::default();
    if let Some(value) = env_string("ROVE_WEB_API_BASE") {
        web.api_base = Some(value);
        keys.push("ROVE_WEB_API_BASE".to_string());
    }

    Ok(NamedConfigLayer {
        config: AppConfigLayer {
            runtime: Some(runtime).filter(has_runtime_values),
            provider: Some(provider).filter(has_provider_values),
            tool: Some(tool).filter(has_tool_values),
            memory: Some(memory).filter(has_memory_values),
            state: Some(state).filter(has_state_values),
            api: Some(api).filter(has_api_values),
            web: Some(web).filter(has_web_values),
            routing: Some(routing).filter(has_routing_values),
        },
        keys,
    })
}

fn process_env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn project_environment_value_allowed(
    name: &str,
    capabilities: ProjectEnvironmentCapabilities,
) -> bool {
    match name {
        "ROVE_AGENT"
        | "ROVE_WORKSPACE_INSTRUCTIONS"
        | "ROVE_ALLOW_REMEDIATION_PROCEDURES"
        | "ROVE_MAX_PROCEDURE_SELECTIONS" => capabilities.workspace_instructions,
        "ROVE_PROVIDER_ACTIVE"
        | "ROVE_PROVIDER_PROFILES"
        | "ROVE_PROVIDER_FALLBACK_PROFILES"
        | "ROVE_MODEL"
        | "ROVE_FALLBACK_MODELS" => capabilities.provider_credentials,
        "ROVE_MCP_CONFIG" => capabilities.mcp_processes,
        "ROVE_SYSTEM_PROMPT"
        | "ROVE_PLANNER_PROMPT"
        | "ROVE_MEMORY_SESSION_DIR"
        | "ROVE_MEMORY_DURABLE_DIR"
        | "ROVE_STATE_DIR"
        | "ROVE_STATE_SQLITE"
        | "ROVE_STATE_ALLOW_EXTERNAL_PATHS" => capabilities.external_paths,
        _ => capabilities.project_configuration,
    }
}

fn load_project_environment(
    workspace_root: &Path,
    enabled: bool,
) -> anyhow::Result<BTreeMap<String, String>> {
    if !enabled {
        return Ok(BTreeMap::new());
    }
    let Some(path) = bounded_workspace_config_path(workspace_root, ".env") else {
        return Ok(BTreeMap::new());
    };
    dotenvy::from_path_iter(&path)?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(anyhow::Error::from)
}

fn filtered_project_config(
    path: &Path,
    workspace_instructions_granted: bool,
    _provider_credentials_granted: bool,
    mcp_processes_granted: bool,
    external_paths_granted: bool,
) -> anyhow::Result<serde_json::Value> {
    let mut value = Figment::new()
        .merge(Toml::file(path))
        .extract::<serde_json::Value>()?;
    reject_project_provider_definitions(&value)?;
    if let Some(provider) = value
        .get_mut("provider")
        .and_then(serde_json::Value::as_object_mut)
        && provider.contains_key("active")
        && !provider.contains_key("model")
    {
        provider.insert(
            "model".to_string(),
            serde_json::Value::String(String::new()),
        );
    }
    if !workspace_instructions_granted {
        remove_project_config_value(&mut value, &["runtime", "agent"]);
    }
    if !mcp_processes_granted {
        remove_project_config_value(&mut value, &["tool", "mcp_config_path"]);
    }
    if !external_paths_granted {
        for path in [
            &["runtime", "system_prompt_path"][..],
            &["runtime", "planner_prompt_path"][..],
            &["memory", "session_dir"][..],
            &["memory", "durable_dir"][..],
            &["state", "state_dir"][..],
            &["state", "sqlite_path"][..],
            &["state", "allow_external_paths"][..],
        ] {
            remove_project_config_value(&mut value, path);
        }
    }
    Ok(value)
}

fn reject_project_provider_definitions(value: &serde_json::Value) -> anyhow::Result<()> {
    let Some(provider) = value.get("provider").and_then(serde_json::Value::as_object) else {
        return Ok(());
    };
    let forbidden = [
        "profiles",
        "fallback_profiles",
        "fallback_models",
        "options",
        "base_url",
        "auth",
        "headers",
        "protocol_options",
        "wire_protocol",
    ];
    if let Some(field) = forbidden
        .iter()
        .find(|field| provider.contains_key(**field))
    {
        anyhow::bail!(
            "project_provider_authority_violation: workspace provider.{field} cannot define provider endpoints, credentials, headers, fallbacks, or protocol options; select a user profile with provider.active and provider.model"
        );
    }
    Ok(())
}

fn remove_project_config_value(value: &mut serde_json::Value, path: &[&str]) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut current = value;
    for parent in parents {
        let Some(next) = current.get_mut(*parent) else {
            return;
        };
        current = next;
    }
    if let Some(object) = current.as_object_mut() {
        object.remove(*last);
    }
}

fn parse_env<T>(name: &str, value: &str) -> anyhow::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|err| anyhow::anyhow!("{name} is invalid: {err}"))
}

fn parse_env_bool(name: &str, value: &str) -> anyhow::Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => anyhow::bail!("{name} is invalid: expected true or false"),
    }
}

fn parse_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn has_runtime_values(layer: &RuntimeConfigLayer) -> bool {
    layer.max_steps.is_some()
        || layer.system_prompt_path.is_some()
        || layer.planner_prompt_path.is_some()
        || layer.model_compaction_enabled.is_some()
        || layer.compaction_failure_threshold.is_some()
        || layer.context_soft_limit_tokens.is_some()
        || layer.context_hard_limit_tokens.is_some()
        || layer.context_reserved_tokens.is_some()
        || layer.agent.is_some()
}

fn has_agent_values(layer: &AgentConfigLayer) -> bool {
    layer.selector.is_some()
        || layer.workspace_instructions.is_some()
        || layer.allow_remediation_procedures.is_some()
        || layer.max_procedure_selections.is_some()
}

fn has_provider_values(layer: &ProviderConfigLayer) -> bool {
    layer.active.is_some()
        || layer.profiles.is_some()
        || layer.fallback_profiles.is_some()
        || layer.model.is_some()
        || layer.fallback_models.is_some()
        || layer.options.is_some()
}

fn has_tool_values(layer: &ToolConfigLayer) -> bool {
    layer.mcp_config_path.is_some() || layer.shell.is_some()
}

fn has_shell_values(layer: &ShellConfigLayer) -> bool {
    layer.timeout_ms.is_some()
        || layer.max_output_bytes.is_some()
        || layer.inherit_environment.is_some()
        || layer.denylist.is_some()
}

fn has_memory_values(layer: &MemoryConfigLayer) -> bool {
    layer.session_dir.is_some() || layer.durable_dir.is_some() || layer.recall_limit.is_some()
}

fn has_state_values(layer: &StateConfigLayer) -> bool {
    layer.state_dir.is_some()
        || layer.sqlite_path.is_some()
        || layer.lazy_migration.is_some()
        || layer.sqlite_busy_timeout_ms.is_some()
        || layer.allow_external_paths.is_some()
}

fn has_api_values(layer: &ApiConfigLayer) -> bool {
    layer.bind_addr.is_some()
        || layer.token_auth.is_some()
        || layer.unsafe_remote_without_auth.is_some()
        || layer.cors_origins.is_some()
        || layer.rate_limit_per_minute.is_some()
}

fn has_web_values(layer: &WebConfigLayer) -> bool {
    layer.api_base.is_some()
}

fn has_routing_values(layer: &RoutingConfigLayer) -> bool {
    layer.failure_threshold.is_some()
        || layer.open_cooldown_ms.is_some()
        || layer.retry_max_attempts.is_some()
        || layer.retry_backoff_base_ms.is_some()
        || layer.retry_backoff_max_ms.is_some()
}

fn bounded_workspace_config_path(workspace_root: &Path, raw_path: &str) -> Option<PathBuf> {
    const MAX_BOOTSTRAP_CONFIG_BYTES: u64 = 256 * 1024;

    let path =
        rove_runtime::workspace::boundary::resolve_workspace_read_path(workspace_root, raw_path)
            .ok()?;
    (std::fs::metadata(&path).ok()?.len() <= MAX_BOOTSTRAP_CONFIG_BYTES).then_some(path)
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn trusted_overrides() -> AppConfigOverrides {
        AppConfigOverrides {
            trust_project: true,
            ..AppConfigOverrides::default()
        }
    }

    fn clear_config_env() {
        for key in [
            "ROVE_PROVIDER_ACTIVE",
            "ROVE_PROVIDER_PROFILES",
            "ROVE_PROVIDER_FALLBACK_PROFILES",
            "ROVE_PROVIDER",
            "ROVE_PROVIDER_API_BASE",
            "OPENAI_API_BASE",
            "OPENAI_BASE_URL",
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "PROJECT_PROVIDER_SECRET",
            "ROVE_MODEL",
            "ROVE_FALLBACK_MODELS",
            "ROVE_FALLBACK_PROVIDERS",
            "ROVE_ROUTING_FAILURE_THRESHOLD",
            "ROVE_ROUTING_OPEN_COOLDOWN_MS",
            "ROVE_ROUTING_RETRY_MAX_ATTEMPTS",
            "ROVE_ROUTING_RETRY_BACKOFF_BASE_MS",
            "ROVE_ROUTING_RETRY_BACKOFF_MAX_MS",
            "ROVE_MAX_STEPS",
            "ROVE_SYSTEM_PROMPT",
            "ROVE_PLANNER_PROMPT",
            "ROVE_MODEL_COMPACTION_ENABLED",
            "ROVE_COMPACTION_FAILURE_THRESHOLD",
            "ROVE_CONTEXT_SOFT_LIMIT_TOKENS",
            "ROVE_CONTEXT_HARD_LIMIT_TOKENS",
            "ROVE_CONTEXT_RESERVED_TOKENS",
            "ROVE_AGENT",
            "ROVE_WORKSPACE_INSTRUCTIONS",
            "ROVE_ALLOW_REMEDIATION_PROCEDURES",
            "ROVE_MAX_PROCEDURE_SELECTIONS",
            "ROVE_OPENAI_RESPONSES_PROMPT_CACHE",
            "ROVE_OPENAI_RESPONSES_PROMPT_CACHE_RETENTION",
            "ROVE_MCP_CONFIG",
            "ROVE_SHELL_TIMEOUT_MS",
            "ROVE_SHELL_MAX_OUTPUT_BYTES",
            "ROVE_SHELL_INHERIT_ENVIRONMENT",
            "ROVE_SHELL_DENYLIST",
            "ROVE_MEMORY_SESSION_DIR",
            "ROVE_MEMORY_DURABLE_DIR",
            "ROVE_MEMORY_RECALL_LIMIT",
            "ROVE_STATE_DIR",
            "ROVE_STATE_SQLITE",
            "ROVE_STATE_LAZY_MIGRATION",
            "ROVE_STATE_SQLITE_BUSY_TIMEOUT_MS",
            "ROVE_STATE_ALLOW_EXTERNAL_PATHS",
            "ROVE_API_BIND_ADDR",
            "ROVE_API_TOKEN",
            "ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH",
            "ROVE_API_CORS_ORIGINS",
            "ROVE_API_RATE_LIMIT_PER_MINUTE",
            "ROVE_WEB_API_BASE",
            crate::user_config::USER_CONFIG_ROOT_ENV,
            crate::project_trust::PROJECT_TRUST_STORE_ENV,
            TRUSTED_WORKSPACES_ENV,
        ] {
            unsafe {
                std::env::remove_var(key);
            }
        }
    }

    fn write_user_config(temp: &tempfile::TempDir, text: &str) {
        let root = temp.path().join("user-config");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("config.toml"), text).unwrap();
        unsafe { std::env::set_var(crate::user_config::USER_CONFIG_ROOT_ENV, root) };
    }

    #[test]
    fn parse_csv_trims_empty_entries() {
        assert_eq!(
            parse_csv(" fallback-a, ,fallback-b,, fallback-c "),
            vec!["fallback-a", "fallback-b", "fallback-c"]
        );
    }

    #[test]
    fn agent_config_defaults_to_the_legacy_compatibility_profile() {
        let config = AppConfig::default();

        assert_eq!(config.runtime.agent.selector, "builtin:legacy");
        assert!(!config.runtime.agent.workspace_instructions);
        assert!(!config.runtime.agent.allow_remediation_procedures);
        assert_eq!(config.runtime.agent.max_procedure_selections, 3);
    }

    #[test]
    fn invalid_agent_selector_is_rejected_during_config_load() {
        let _guard = env_lock();
        clear_config_env();
        let temp = tempfile::TempDir::new().unwrap();

        let error = AppConfig::load(
            temp.path(),
            AppConfigOverrides {
                agent_selector: Some("unqualified".to_string()),
                ..AppConfigOverrides::default()
            },
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("runtime.agent.selector is invalid")
        );
    }

    #[test]
    fn layered_config_uses_default_project_env_cli_order() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        write_user_config(
            &tmp,
            r#"
schema_version = 1
[model]
default_profile = "default"
default_model = "user-model"
[provider.profiles.default]
provider_type = "ollama"
base_url = "http://localhost:11434"
model = "user-model"
"#,
        );
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[runtime]
max_steps = 9

[provider]
active = "default"
model = "project-model"

[routing]
failure_threshold = 2
retry_max_attempts = 4
retry_backoff_base_ms = 123
retry_backoff_max_ms = 456
"#,
        )
        .unwrap();
        unsafe {
            std::env::set_var("ROVE_MODEL", "env-model");
            std::env::set_var("ROVE_MAX_STEPS", "11");
        }

        let config = AppConfig::load(
            tmp.path(),
            AppConfigOverrides {
                model: Some("cli-model".to_string()),
                max_steps: Some(13),
                agent_selector: None,
                api_bind_addr: None,
                trust_project: true,
            },
        )
        .unwrap();

        assert_eq!(config.provider.active.as_deref(), Some("default"));
        assert_eq!(config.provider.model, "cli-model");
        assert_eq!(config.runtime.max_steps, 13);
        assert_eq!(config.routing.failure_threshold, 2);
        assert_eq!(config.routing.retry_max_attempts, 4);
        assert_eq!(config.routing.retry_backoff_base_ms, 123);
        assert_eq!(config.routing.retry_backoff_max_ms, 456);
        assert!(config.source_summary.project_config_loaded);
        assert!(
            config
                .source_summary
                .env_keys
                .contains(&"ROVE_MODEL".to_string())
        );
        assert!(
            config
                .source_summary
                .cli_keys
                .contains(&"provider.model".to_string())
        );
        clear_config_env();
    }

    #[test]
    fn project_config_is_deferred_until_the_workspace_is_explicitly_trusted() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        write_user_config(
            &tmp,
            r#"
schema_version = 1
[model]
default_profile = "external"
default_model = "user-model"
[provider.profiles.external]
provider_type = "openai"
base_url = "https://user.example.test/v1"
model = "user-model"
auth = { style = "bearer", secret = { env = "OPENAI_API_KEY" } }
"#,
        );
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[runtime]
max_steps = 9

[provider]
active = "external"
model = "project-model"
"#,
        )
        .unwrap();

        let restricted = AppConfig::load(tmp.path(), AppConfigOverrides::default()).unwrap();
        assert_eq!(
            restricted.project_activation_state(),
            ProjectActivationState::Restricted
        );
        assert!(restricted.source_summary.project_config_present);
        assert!(!restricted.source_summary.project_config_loaded);
        assert_eq!(
            restricted.runtime.max_steps,
            RuntimeConfig::default().max_steps
        );
        assert_eq!(restricted.provider.model, "user-model");

        let trusted = AppConfig::load(tmp.path(), trusted_overrides()).unwrap();
        assert_eq!(
            trusted.project_activation_state(),
            ProjectActivationState::Trusted
        );
        assert_eq!(
            trusted.source_summary.project_activation_source,
            Some(ProjectActivationSource::CommandLine)
        );
        assert!(trusted.source_summary.project_config_loaded);
        assert_eq!(trusted.runtime.max_steps, 9);
        assert_eq!(trusted.provider.model, "project-model");
        clear_config_env();
    }

    #[test]
    fn durable_capabilities_filter_project_toml_and_scoped_environment_independently() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        write_user_config(
            &tmp,
            r#"
schema_version = 1
[model]
default_profile = "external"
default_model = "user-model"
[provider.profiles.external]
provider_type = "openai"
base_url = "https://user.example.test/v1"
model = "user-model"
auth = { style = "bearer", secret = { env = "PROJECT_PROVIDER_SECRET" } }
"#,
        );
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[runtime]
max_steps = 9
system_prompt_path = "../outside-system.md"

[provider]
active = "external"

[tool]
mcp_config_path = "custom-mcp.json"

[state]
allow_external_paths = true
"#,
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".env"),
            "ROVE_MODEL=project-env-model\nPROJECT_PROVIDER_SECRET=project-secret\n",
        )
        .unwrap();
        let repository = ProjectTrustRepository::new(tmp.path().join("operator-trust.sqlite"));
        let digests = capability_digest_map(tmp.path(), None, None);
        let selected = |names: &[&str]| {
            names
                .iter()
                .map(|name| ((*name).to_string(), digests.get(*name).unwrap().clone()))
                .collect::<BTreeMap<_, _>>()
        };

        repository
            .decide(
                tmp.path(),
                WorkspaceKind::Folder,
                crate::project_trust::ProjectTrustDecision::Grant,
                selected(&[crate::project_trust::CAP_PROJECT_CONFIGURATION]),
            )
            .unwrap();
        let project_only = AppConfig::load_with_project_trust_repository(
            tmp.path(),
            AppConfigOverrides::default(),
            &repository,
        )
        .unwrap();

        assert_eq!(project_only.runtime.max_steps, 9);
        assert_eq!(project_only.provider.model, "user-model");
        assert_eq!(
            project_only.runtime.system_prompt_path,
            RuntimeConfig::default().system_prompt_path
        );
        assert_eq!(
            project_only.tool.mcp_config_path,
            ToolConfig::default().mcp_config_path
        );
        assert!(!project_only.state.allow_external_paths);
        assert!(project_only.project_environment.values().is_empty());
        assert!(std::env::var_os("PROJECT_PROVIDER_SECRET").is_none());
        assert!(std::env::var_os("ROVE_MODEL").is_none());

        repository
            .decide(
                tmp.path(),
                WorkspaceKind::Folder,
                crate::project_trust::ProjectTrustDecision::Grant,
                selected(&[crate::project_trust::CAP_PROVIDER_CREDENTIALS]),
            )
            .unwrap();
        let provider_enabled = AppConfig::load_with_project_trust_repository(
            tmp.path(),
            AppConfigOverrides::default(),
            &repository,
        )
        .unwrap();

        assert_eq!(provider_enabled.provider.model, "project-env-model");
        assert_eq!(
            provider_enabled
                .project_environment
                .values()
                .get("PROJECT_PROVIDER_SECRET")
                .map(String::as_str),
            Some("project-secret")
        );
        assert!(std::env::var_os("PROJECT_PROVIDER_SECRET").is_none());
        assert!(!format!("{provider_enabled:?}").contains("project-secret"));
        let model = crate::factory::try_build_model_client(
            &provider_enabled,
            provider_enabled.provider.model.clone(),
        )
        .unwrap();
        assert!(model.client_id().as_str().contains("user.example.test"));

        repository
            .decide(
                tmp.path(),
                WorkspaceKind::Folder,
                crate::project_trust::ProjectTrustDecision::Grant,
                selected(&[
                    crate::project_trust::CAP_MCP_PROCESSES,
                    crate::project_trust::CAP_EXTERNAL_PATHS,
                ]),
            )
            .unwrap();
        let all_enabled = AppConfig::load_with_project_trust_repository(
            tmp.path(),
            AppConfigOverrides::default(),
            &repository,
        )
        .unwrap();
        assert_eq!(
            all_enabled.runtime.system_prompt_path,
            PathBuf::from("../outside-system.md")
        );
        assert_eq!(
            all_enabled.tool.mcp_config_path,
            PathBuf::from("custom-mcp.json")
        );
        assert!(all_enabled.state.allow_external_paths);
        clear_config_env();
    }

    #[test]
    fn trusted_bootstrap_rejects_external_project_config_and_env_symlinks_when_supported() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let config_dir = workspace.join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        let outside_config = tmp.path().join("outside-config.toml");
        let outside_env = tmp.path().join("outside.env");
        std::fs::write(
            &outside_config,
            "[runtime]\nmax_steps = 99\n[provider]\nmodel = \"poisoned\"\n",
        )
        .unwrap();
        std::fs::write(&outside_env, "ROVE_MODEL=poisoned-from-env\n").unwrap();
        if !create_file_symlink(&outside_config, &config_dir.join("config.toml"))
            || !create_file_symlink(&outside_env, &workspace.join(".env"))
        {
            clear_config_env();
            return;
        }

        let config = AppConfig::load(&workspace, trusted_overrides()).unwrap();

        assert!(config.source_summary.project_config_present);
        assert!(!config.source_summary.project_config_loaded);
        assert_eq!(config.runtime.max_steps, RuntimeConfig::default().max_steps);
        assert!(config.provider.model.is_empty());
        assert!(std::env::var_os("ROVE_MODEL").is_none());
        clear_config_env();
    }

    #[test]
    fn environment_grant_is_exact_and_rebase_does_not_inherit_it() {
        let _guard = env_lock();
        clear_config_env();
        let selected = tempfile::TempDir::new().unwrap();
        let other = tempfile::TempDir::new().unwrap();
        let trusted = std::env::join_paths([selected.path()]).unwrap();
        unsafe { std::env::set_var(TRUSTED_WORKSPACES_ENV, trusted) };

        let mut config = AppConfig::load(selected.path(), AppConfigOverrides::default()).unwrap();
        assert_eq!(
            config.source_summary.project_activation_source,
            Some(ProjectActivationSource::Environment)
        );
        assert!(config.project_activation_allowed());

        config.rebase_to_workspace(other.path());
        assert_eq!(
            config.project_activation_state(),
            ProjectActivationState::Restricted
        );
        assert!(!config.project_activation_allowed());
        clear_config_env();
    }

    #[test]
    fn product_trust_resolution_preserves_temporary_grants_but_not_revocation() {
        let mut config = AppConfig::default();
        assert_eq!(
            config.source_summary.project_activation_source,
            Some(ProjectActivationSource::Programmatic)
        );
        config.apply_project_trust_resolution(crate::project_trust::ProjectTrustResolution {
            state: ProjectActivationState::Restricted,
            identity_digest: "sha256:restricted".to_string(),
            invalidated_capabilities: Vec::new(),
            granted_capabilities: Default::default(),
        });
        assert!(config.project_activation_allowed());
        assert_eq!(
            config.source_summary.project_activation_source,
            Some(ProjectActivationSource::Programmatic)
        );

        config.apply_project_trust_resolution(crate::project_trust::ProjectTrustResolution {
            state: ProjectActivationState::Revoked,
            identity_digest: "sha256:revoked".to_string(),
            invalidated_capabilities: Vec::new(),
            granted_capabilities: Default::default(),
        });
        assert_eq!(
            config.project_activation_state(),
            ProjectActivationState::Revoked
        );
        assert!(!config.project_activation_allowed());
    }

    #[test]
    fn unknown_product_trust_resolution_is_restricted_for_runtime_activation() {
        let mut config = AppConfig::default();
        config.source_summary.project_activation = ProjectActivationState::Restricted;
        config.source_summary.project_activation_source = None;
        config.apply_project_trust_resolution(crate::project_trust::ProjectTrustResolution {
            state: ProjectActivationState::Unknown,
            identity_digest: "sha256:unknown".to_string(),
            invalidated_capabilities: Vec::new(),
            granted_capabilities: Default::default(),
        });
        assert_eq!(
            config.project_activation_state(),
            ProjectActivationState::Restricted
        );
        assert!(!config.project_activation_allowed());
    }

    #[test]
    fn user_config_parses_named_profiles_and_protocol_options() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        write_user_config(
            &tmp,
            r#"
schema_version = 1
[model]
default_profile = "team-gateway"
default_model = "project-model"
[provider]
fallback_profiles = ["claude"]

[provider.profiles.team-gateway]
provider_type = "openai-responses"
base_url = "https://gateway.example.test/v1"
model = "project-model"
auth = { style = "bearer", secret = { env = "TEAM_GATEWAY_KEY" } }
headers = { x-tenant = { env = "TEAM_TENANT" } }
protocol_options = { prompt_cache_enabled = true, prompt_cache_retention = "24h" }

[provider.profiles.team-gateway.options]
max_tokens = 1024
temperature = 0.2

[provider.profiles.claude]
provider_type = "anthropic"
base_url = "https://api.anthropic.com"
model = "claude-fallback"
auth = { style = "header", header = "x-api-key", secret = { env = "ANTHROPIC_API_KEY" } }
"#,
        );

        let config = AppConfig::load(tmp.path(), trusted_overrides()).unwrap();

        assert_eq!(config.provider.active.as_deref(), Some("team-gateway"));
        assert_eq!(config.provider.model, "project-model");
        assert_eq!(config.provider.fallback_profiles, ["claude"]);
        let profile = &config.provider.profiles["team-gateway"];
        assert_eq!(profile.provider_type, "openai-responses");
        assert_eq!(profile.options.max_tokens, Some(1024));
        assert_eq!(profile.protocol_options["prompt_cache_enabled"], true);
        assert_eq!(profile.protocol_options["prompt_cache_retention"], "24h");
        clear_config_env();
    }

    #[test]
    fn named_profile_model_uses_project_env_cli_precedence() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        write_user_config(
            &tmp,
            r#"
schema_version = 1
[model]
default_profile = "gateway"
default_model = "user-model"
[provider.profiles.gateway]
provider_type = "openai"
base_url = "https://gateway.example.test/v1"
model = "user-model"
auth = { style = "bearer", secret = { env = "OPENAI_API_KEY" } }
"#,
        );
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[provider]
active = "gateway"
model = "project-model"
"#,
        )
        .unwrap();

        let project = AppConfig::load(tmp.path(), trusted_overrides()).unwrap();
        assert_eq!(project.provider.model, "project-model");

        unsafe { std::env::set_var("ROVE_MODEL", "env-model") };
        let env = AppConfig::load(tmp.path(), trusted_overrides()).unwrap();
        assert_eq!(env.provider.model, "env-model");

        let cli = AppConfig::load(
            tmp.path(),
            AppConfigOverrides {
                model: Some("cli-model".to_string()),
                trust_project: true,
                ..AppConfigOverrides::default()
            },
        )
        .unwrap();
        assert_eq!(cli.provider.model, "cli-model");
        clear_config_env();
    }

    #[test]
    fn named_profile_references_fail_closed() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("user-config");
        std::fs::create_dir_all(&root).unwrap();
        unsafe { std::env::set_var(crate::user_config::USER_CONFIG_ROOT_ENV, &root) };
        let path = root.join("config.toml");
        let profile = r#"
schema_version = 1
[provider.profiles.gateway]
provider_type = "openai"
base_url = "https://gateway.example.test/v1"
model = "model"
auth = { style = "bearer", secret = { env = "OPENAI_API_KEY" } }
"#;

        std::fs::write(&path, profile).unwrap();
        let error = AppConfig::load(tmp.path(), trusted_overrides())
            .unwrap_err()
            .to_string();
        assert!(error.contains("provider.active is required"));

        std::fs::write(
            &path,
            "schema_version = 1\n[model]\ndefault_profile = \"gateway\"\n[provider]\nfallback_profiles = [\"missing\"]\n[provider.profiles.gateway]\nprovider_type = \"openai\"\nbase_url = \"https://gateway.example.test/v1\"\nmodel = \"model\"\nauth = { style = \"bearer\", secret = { env = \"OPENAI_API_KEY\" } }",
        )
        .unwrap();
        let error = AppConfig::load(tmp.path(), trusted_overrides())
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown profile `missing`"));

        std::fs::write(
            &path,
            "schema_version = 1\n[model]\ndefault_profile = \"gateway\"\n[provider]\nfallback_profiles = [\"other\", \"other\"]\n[provider.profiles.gateway]\nprovider_type = \"openai\"\nbase_url = \"https://gateway.example.test/v1\"\nmodel = \"model\"\nauth = { style = \"bearer\", secret = { env = \"OPENAI_API_KEY\" } }\n[provider.profiles.other]\nprovider_type = \"ollama\"\nbase_url = \"http://localhost:11434\"\nmodel = \"other\"\n",
        )
        .unwrap();
        let error = AppConfig::load(tmp.path(), trusted_overrides())
            .unwrap_err()
            .to_string();
        assert!(error.contains("must not contain duplicates"));
        clear_config_env();
    }

    #[test]
    fn validation_rejects_invalid_profile_options() {
        let mut config = AppConfig::default();
        let profile = config.provider.profiles.get_mut("default").unwrap();
        profile.options.max_tokens = Some(0);
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("profile.options.max_tokens must be greater than 0")
        );

        let mut config = AppConfig::default();
        let profile = config.provider.profiles.get_mut("default").unwrap();
        profile.options.temperature = Some(f64::NAN);
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("profile.options.temperature must be finite")
        );
    }

    #[test]
    fn project_config_rejects_provider_options() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[provider]
active = "default"
model = "primary-model"

[provider.profiles.default]
provider_type = "openai"
base_url = "https://api.openai.com/v1"
model = "primary-model"

[provider.options]
max_tokens = 2048
temperature = 0.2
top_p = 0.8
frequency_penalty = 0.3
presence_penalty = 0.4
"#,
        )
        .unwrap();

        let error = AppConfig::load(tmp.path(), trusted_overrides()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("project_provider_authority_violation")
        );
    }

    #[test]
    fn validation_rejects_remote_api_without_token_or_unsafe_flag() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let err = AppConfig::load(
            tmp.path(),
            AppConfigOverrides {
                api_bind_addr: Some("0.0.0.0:8787".to_string()),
                ..AppConfigOverrides::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("api.token_auth is required"));
    }

    #[test]
    fn validation_rejects_relative_paths_that_escape_workspace() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[runtime]
system_prompt_path = "../outside/prompt.md"
"#,
        )
        .unwrap();

        let err = AppConfig::load(tmp.path(), trusted_overrides()).unwrap_err();

        assert!(
            err.to_string()
                .contains("runtime.system_prompt_path resolves outside the workspace")
        );
    }

    #[test]
    fn product_memory_boundary_remains_strict_when_external_paths_are_enabled() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let outside = tmp.path().join("outside-memory");
        let inside = workspace.join(".rove/memory");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let mut config = AppConfig::default();
        config.rebase_to_workspace(&workspace);
        config.state.allow_external_paths = true;
        config.memory.durable_dir = outside;
        config.validate().unwrap();

        let error = config.workspace_bounded_durable_memory_dir().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("memory.durable_dir resolves outside the selected workspace")
        );

        config.memory.durable_dir = inside.clone();
        assert_eq!(
            config.workspace_bounded_durable_memory_dir().unwrap(),
            inside.canonicalize().unwrap()
        );
    }

    #[test]
    fn product_mcp_boundary_remains_strict_when_external_paths_are_enabled() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        let outside = tmp.path().join("outside-mcp.json");
        let inside = workspace.join(".rove/mcp_servers.json");
        std::fs::create_dir_all(inside.parent().unwrap()).unwrap();

        let mut config = AppConfig::default();
        config.rebase_to_workspace(&workspace);
        config.state.allow_external_paths = true;
        config.tool.mcp_config_path = outside;
        config.validate().unwrap();

        let error = config.workspace_bounded_mcp_config_path().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("tool.mcp_config_path resolves outside the selected workspace")
        );

        config.tool.mcp_config_path = inside.clone();
        assert_eq!(config.workspace_bounded_mcp_config_path().unwrap(), inside);
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    #[cfg(not(any(unix, windows)))]
    fn create_file_symlink(_target: &Path, _link: &Path) -> bool {
        false
    }

    #[test]
    fn an_unconfigured_execution_section_keeps_the_deterministic_projection() {
        let config = AppConfig::default();
        assert!(config.runtime.execution.is_empty());

        let derived = ExecutionPolicy::from_max_steps_and_plan_flag(config.runtime.max_steps, true);
        let resolved = config.runtime.execution.apply_to(derived.clone());

        assert_eq!(resolved, derived, "an empty section changes nothing");
        assert_eq!(
            resolved.selection_source,
            StrategySelectionSource::MaxStepsAndPlanFlag
        );
        config.validate_execution_config().unwrap();
    }

    #[test]
    fn configured_dimensions_overlay_the_derived_policy() {
        let execution = ExecutionConfig {
            evaluator_mode: Some(ExecutionEvaluatorMode::RuleOnly),
            finalizer_policy: Some(ExecutionFinalizerPolicy::ModelPreferred),
            max_tool_calls: Some(40),
            max_tool_calls_per_step: Some(5),
            max_wall_time_ms: Some(120_000),
            ..ExecutionConfig::default()
        };
        let derived = ExecutionPolicy::from_max_steps_and_plan_flag(12, true);
        let resolved = execution.apply_to(derived.clone());

        assert_eq!(resolved.evaluator_mode, EvaluatorMode::RuleOnly);
        assert_eq!(resolved.finalizer_policy, FinalizerPolicy::ModelPreferred);
        assert_eq!(resolved.budgets.max_tool_calls, Some(40));
        assert_eq!(resolved.budgets.max_tool_calls_per_step, Some(5));
        assert_eq!(resolved.budgets.max_wall_time_ms, Some(120_000));
        assert_eq!(
            resolved.selection_source,
            StrategySelectionSource::Config,
            "an explicitly configured policy records its own source"
        );
        // Dimensions the strategy derived and the operator did not set survive.
        assert_eq!(
            resolved.budgets.max_step_attempts,
            derived.budgets.max_step_attempts
        );
        resolved.validate().unwrap();
    }

    #[test]
    fn a_zero_execution_limit_is_rejected_at_startup() {
        let mut config = AppConfig::default();
        config.runtime.execution.max_tool_calls = Some(0);

        let error = config.validate_execution_config().unwrap_err().to_string();

        assert!(
            error.contains("runtime.execution.max_tool_calls must be greater than 0"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_per_step_limit_above_its_global_limit_is_rejected_at_startup() {
        let mut config = AppConfig::default();
        config.runtime.execution.max_tool_calls = Some(4);
        config.runtime.execution.max_tool_calls_per_step = Some(9);

        let error = config.validate_execution_config().unwrap_err().to_string();

        assert!(
            error.contains("runtime.execution is invalid"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_execution_section_round_trips_through_toml() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[runtime]
max_steps = 9

[runtime.execution]
evaluator_mode = "rule_first_model_on_ambiguity"
finalizer_policy = "model_preferred"
max_model_turns = 30
max_model_turns_per_step = 4
max_total_tokens = 500000
"#,
        )
        .unwrap();

        let config = AppConfig::load(tmp.path(), trusted_overrides()).unwrap();

        let execution = &config.runtime.execution;
        assert_eq!(
            execution.evaluator_mode,
            Some(ExecutionEvaluatorMode::RuleFirstModelOnAmbiguity)
        );
        assert_eq!(
            execution.finalizer_policy,
            Some(ExecutionFinalizerPolicy::ModelPreferred)
        );
        assert_eq!(execution.max_model_turns, Some(30));
        assert_eq!(execution.max_model_turns_per_step, Some(4));
        assert_eq!(execution.max_total_tokens, Some(500_000));

        let resolved = execution.apply_to(ExecutionPolicy::from_max_steps_and_plan_flag(
            config.runtime.max_steps,
            true,
        ));
        assert_eq!(resolved.budgets.max_model_turns, Some(30));
        resolved.validate().unwrap();
        clear_config_env();
    }

    #[test]
    fn an_old_config_without_an_execution_section_still_loads() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[runtime]
max_steps = 7
"#,
        )
        .unwrap();

        let config = AppConfig::load(tmp.path(), trusted_overrides()).unwrap();

        assert_eq!(config.runtime.max_steps, 7);
        assert!(
            config.runtime.execution.is_empty(),
            "a missing section defaults to unconfigured"
        );
        config.validate().unwrap();
        clear_config_env();
    }
}
