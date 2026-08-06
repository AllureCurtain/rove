use std::collections::{BTreeMap, HashSet};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use figment::Figment;
use figment::providers::{Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

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
        };
        // In-memory defaults used by tests and ad-hoc construction include a
        // usable fake profile. Figment loading uses empty profiles so project
        // TOML maps do not merge with built-in defaults.
        ensure_default_provider_profile(&mut config.provider);
        config
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            active: None,
            profiles: BTreeMap::new(),
            fallback_profiles: Vec::new(),
            model: "fake".to_string(),
            fallback_models: Vec::new(),
            options: ProviderOptions::default(),
        }
    }
}

fn default_fake_provider_profile() -> ProviderProfileConfig {
    ProviderProfileConfig {
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
        }
    }

    pub fn load(
        workspace_root: impl AsRef<Path>,
        overrides: AppConfigOverrides,
    ) -> anyhow::Result<Self> {
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
        let capability_digests = capability_digest_map(
            &workspace_root,
            Some(&workspace_root.join(".rove/mcp_servers.json")),
            std::env::var("ROVE_PROVIDER_ACTIVE").ok().as_deref(),
        );
        let durable_resolution = ProjectTrustRepository::operator_default()
            .and_then(|repository| {
                repository.resolve(
                    &workspace_root,
                    workspace_kind_for_root(&workspace_root),
                    &capability_digests,
                )
            })
            .unwrap_or_else(|_| crate::project_trust::ProjectTrustResolution {
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
        let project_config_granted = activation.state == ProjectActivationState::Trusted
            && (activation.source != Some(ProjectActivationSource::Durable)
                || activation
                    .granted_capabilities
                    .contains(crate::project_trust::CAP_PROJECT_CONFIGURATION));
        if project_config_granted
            && let Some(path) = bounded_workspace_config_path(&workspace_root, ".env")
        {
            let _ = dotenvy::from_path(path);
        }
        let project_config_path = workspace_root.join(".rove/config.toml");
        let project_config_present = project_config_path.exists();
        let safe_project_config_path =
            bounded_workspace_config_path(&workspace_root, ".rove/config.toml");
        let project_config_loaded = project_config_granted && safe_project_config_path.is_some();

        let env_layer = env_layer()?;
        let env_keys = env_layer.keys.clone();
        let cli_layer = overrides.into_layer();
        let cli_keys = cli_layer.keys.clone();

        let mut figment = Figment::from(Serialized::defaults(AppConfig::defaults()));
        if let Some(path) = safe_project_config_path.filter(|_| project_config_loaded) {
            figment = figment.merge(Toml::file(path));
        }
        figment = figment.merge(Serialized::defaults(env_layer.config));
        figment = figment.merge(Serialized::defaults(cli_layer.config));

        let mut config: AppConfig = figment.extract()?;
        // Only inject the built-in fake profile when no profiles were configured.
        ensure_default_provider_profile(&mut config.provider);
        config.source_summary = ConfigSourceSummary {
            workspace_root,
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
                    &capability_digest_map(
                        &workspace_root,
                        Some(&workspace_root.join(".rove/mcp_servers.json")),
                        None,
                    ),
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
        self.source_summary.project_activation = resolution.state;
        self.source_summary.project_activation_source = (resolution.state
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
        if model_overridden {
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
        if self.provider.model.trim().is_empty() {
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
        if self.runtime.compaction_failure_threshold == 0 {
            anyhow::bail!("runtime.compaction_failure_threshold must be greater than 0");
        }
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
            anyhow::bail!(
                "provider.profiles is required; flat provider.name/api_base/api_key config is no longer supported"
            );
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
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 64
        || (!bytes[0].is_ascii_lowercase() && !bytes[0].is_ascii_digit())
        || !bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        anyhow::bail!("{field} must use 1-64 lowercase ASCII letters, digits, '-', '_', or '.'");
    }
    Ok(())
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

fn env_layer() -> anyhow::Result<NamedConfigLayer> {
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

fn env_string(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
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
            .expect("env lock should not be poisoned")
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
            crate::project_trust::PROJECT_TRUST_STORE_ENV,
            TRUSTED_WORKSPACES_ENV,
        ] {
            unsafe {
                std::env::remove_var(key);
            }
        }
    }

    #[test]
    fn parse_csv_trims_empty_entries() {
        assert_eq!(
            parse_csv(" fallback-a, ,fallback-b,, fallback-c "),
            vec!["fallback-a", "fallback-b", "fallback-c"]
        );
    }

    #[test]
    fn layered_config_uses_default_project_env_cli_order() {
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

[provider]
active = "default"
model = "project-model"

[provider.profiles.default]
provider_type = "ollama"
base_url = "http://localhost:11434"
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
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[runtime]
max_steps = 9

[provider]
active = "external"

[provider.profiles.external]
provider_type = "openai"
base_url = "https://untrusted.example.test/v1"
model = "untrusted-model"
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
        assert_eq!(restricted.provider.model, "fake");

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
        assert_eq!(trusted.provider.model, "untrusted-model");
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
        assert_eq!(config.provider.model, "fake");
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
    fn project_config_parses_named_profiles_and_protocol_options() {
        let _guard = env_lock();
        clear_config_env();
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[provider]
active = "team-gateway"
fallback_profiles = ["claude"]

[provider.profiles.team-gateway]
provider_type = "openai-responses"
base_url = "https://gateway.example.test/v1"
model = "project-model"
auth = { style = "bearer", secret = { env = "TEAM_GATEWAY_KEY" } }
headers = { x-tenant = "tenant-secret" }
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
        )
        .unwrap();

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
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[provider]
active = "gateway"

[provider.profiles.gateway]
provider_type = "openai"
base_url = "https://gateway.example.test/v1"
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
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        let path = config_dir.join("config.toml");
        let profile = r#"
[provider.profiles.gateway]
provider_type = "openai"
base_url = "https://gateway.example.test/v1"
model = "model"
"#;

        std::fs::write(&path, profile).unwrap();
        let error = AppConfig::load(tmp.path(), trusted_overrides())
            .unwrap_err()
            .to_string();
        assert!(error.contains("provider.active is required"));

        std::fs::write(
            &path,
            format!(
                "[provider]\nactive = \"gateway\"\nfallback_profiles = [\"missing\"]\n{profile}"
            ),
        )
        .unwrap();
        let error = AppConfig::load(tmp.path(), trusted_overrides())
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown profile `missing`"));

        std::fs::write(
            &path,
            format!(
                "[provider]\nactive = \"gateway\"\nfallback_profiles = [\"other\", \"other\"]\n{profile}\n[provider.profiles.other]\nprovider_type = \"ollama\"\nbase_url = \"http://localhost:11434\"\nmodel = \"other\"\n"
            ),
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
    fn project_config_parses_provider_options() {
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

        let config = AppConfig::load(tmp.path(), trusted_overrides()).unwrap();

        assert_eq!(config.provider.options.max_tokens, Some(2048));
        assert_eq!(config.provider.options.temperature, Some(0.2));
        assert_eq!(config.provider.options.top_p, Some(0.8));
        assert_eq!(config.provider.options.frequency_penalty, Some(0.3));
        assert_eq!(config.provider.options.presence_penalty, Some(0.4));
    }

    #[test]
    fn validation_rejects_remote_api_without_token_or_unsafe_flag() {
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
}
