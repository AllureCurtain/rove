use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use figment::Figment;
use figment::providers::{Format, Serialized, Toml};
use serde::{Deserialize, Serialize};

use crate::memory::paths::MemoryPaths;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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
    pub rag: RagConfig,
    #[serde(skip)]
    pub source_summary: ConfigSourceSummary,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProviderConfig {
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub anthropic_api_key: String,
    pub model: String,
    pub responses_prompt_cache: bool,
    pub responses_prompt_cache_retention: Option<String>,
    pub fallback_models: Vec<String>,
    pub fallback_providers: Vec<FallbackProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FallbackProviderConfig {
    #[serde(default = "default_fallback_provider_name")]
    pub name: String,
    pub api_base: String,
    pub api_key: String,
    pub model: String,
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
#[serde(default)]
pub struct RagConfig {
    pub deterministic: bool,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_api_base: String,
    pub embedding_api_key: String,
    pub rerank_provider: Option<String>,
    pub rerank_model: Option<String>,
    pub rerank_api_key: Option<String>,
    pub timeout_ms: u64,
    pub fallback_to_deterministic: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSourceSummary {
    pub workspace_root: PathBuf,
    pub project_config_path: PathBuf,
    pub project_config_loaded: bool,
    pub env_keys: Vec<String>,
    pub cli_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppConfigOverrides {
    pub model: Option<String>,
    pub max_steps: Option<u32>,
    pub api_bind_addr: Option<String>,
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
            name: "openai".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            anthropic_api_key: String::new(),
            model: "gpt-4o".to_string(),
            responses_prompt_cache: false,
            responses_prompt_cache_retention: None,
            fallback_models: Vec::new(),
            fallback_providers: Vec::new(),
        }
    }
}

fn default_fallback_provider_name() -> String {
    "openai-compatible".to_string()
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

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            deterministic: true,
            embedding_provider: "deterministic".to_string(),
            embedding_model: "deterministic-64".to_string(),
            embedding_api_base: "https://api.openai.com/v1".to_string(),
            embedding_api_key: String::new(),
            rerank_provider: None,
            rerank_model: None,
            rerank_api_key: None,
            timeout_ms: 30_000,
            fallback_to_deterministic: true,
        }
    }
}

impl Default for ConfigSourceSummary {
    fn default() -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            project_config_path: PathBuf::from(".rove/config.toml"),
            project_config_loaded: false,
            env_keys: Vec::new(),
            cli_keys: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn defaults() -> Self {
        Self::default()
    }

    pub fn load(
        workspace_root: impl AsRef<Path>,
        overrides: AppConfigOverrides,
    ) -> anyhow::Result<Self> {
        let _ = dotenvy::dotenv();

        let workspace_root = workspace_root
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.as_ref().to_path_buf());
        let project_config_path = workspace_root.join(".rove/config.toml");
        let project_config_loaded = project_config_path.exists();

        let env_layer = env_layer()?;
        let env_keys = env_layer.keys.clone();
        let cli_layer = overrides.into_layer();
        let cli_keys = cli_layer.keys.clone();

        let mut figment = Figment::from(Serialized::defaults(AppConfig::defaults()));
        if project_config_loaded {
            figment = figment.merge(Toml::file(&project_config_path));
        }
        figment = figment.merge(Serialized::defaults(env_layer.config));
        figment = figment.merge(Serialized::defaults(cli_layer.config));

        let mut config: AppConfig = figment.extract()?;
        config.source_summary = ConfigSourceSummary {
            workspace_root,
            project_config_path,
            project_config_loaded,
            env_keys,
            cli_keys,
        };
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
            .unwrap_or_else(|_| crate::core::planner::DEFAULT_PLANNER_PROMPT.to_string())
    }

    pub fn shell_policy(&self) -> crate::tools::shell::ShellPolicy {
        crate::tools::shell::ShellPolicy {
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

    pub fn rebase_to_workspace(&mut self, workspace_root: impl AsRef<Path>) {
        let workspace_root = workspace_root
            .as_ref()
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.as_ref().to_path_buf());
        self.source_summary.workspace_root = workspace_root.clone();
        self.source_summary.project_config_path = workspace_root.join(".rove/config.toml");
        self.source_summary.project_config_loaded = false;
    }

    fn validate(&self) -> anyhow::Result<()> {
        let provider = self.provider.name.as_str();
        if canonical_provider_name(provider).is_none() {
            anyhow::bail!(
                "invalid provider `{provider}`; expected openai, openai-compatible, openai-responses, anthropic, ollama, or fake"
            );
        }
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
        for fallback in &self.provider.fallback_providers {
            let fallback_provider = fallback.name.as_str();
            let Some(fallback_kind) = canonical_provider_name(fallback_provider) else {
                anyhow::bail!(
                    "invalid fallback provider `{fallback_provider}`; expected openai, openai-compatible, openai-responses, anthropic, ollama, or fake"
                );
            };
            if fallback_kind == "openai-compatible" && fallback.api_base.trim().is_empty() {
                anyhow::bail!(
                    "provider.fallback_providers.api_base must not be empty for OpenAI-compatible providers"
                );
            }
            if fallback.model.trim().is_empty() {
                anyhow::bail!("provider.fallback_providers.model must not be empty");
            }
        }
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
        if self.rag.timeout_ms == 0 {
            anyhow::bail!("rag.timeout_ms must be greater than 0");
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
        let resolved = if path.is_absolute() {
            normalize_path(path)
        } else {
            normalize_path(self.source_summary.workspace_root.join(path))
        };
        if !resolved.starts_with(&self.source_summary.workspace_root) {
            anyhow::bail!(
                "{name} resolves outside the workspace; set state.allow_external_paths=true to allow it"
            );
        }
        Ok(())
    }
}

fn canonical_provider_name(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "openai" | "openai-compatible" => Some("openai-compatible"),
        "openai-responses" | "responses" => Some("openai-responses"),
        "anthropic" => Some("anthropic"),
        "ollama" => Some("ollama"),
        "fake" => Some("fake"),
        _ => None,
    }
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    match path.canonicalize() {
        Ok(canonical) => canonical,
        Err(_) => normalize_lexical_path(path),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    rag: Option<RagConfigLayer>,
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
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    anthropic_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    responses_prompt_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    responses_prompt_cache_retention: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_providers: Option<Vec<FallbackProviderConfig>>,
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

#[derive(Debug, Default, Serialize)]
struct RagConfigLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    deterministic: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding_api_base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    embedding_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rerank_provider: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rerank_model: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rerank_api_key: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fallback_to_deterministic: Option<bool>,
}

#[derive(Debug, Default)]
struct NamedConfigLayer {
    config: AppConfigLayer,
    keys: Vec<String>,
}

impl AppConfigOverrides {
    fn into_layer(self) -> NamedConfigLayer {
        let mut keys = Vec::new();
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
    if let Some(value) = env_string("ROVE_PROVIDER") {
        provider.name = Some(value);
        keys.push("ROVE_PROVIDER".to_string());
    }
    if let Some(value) = env_string("OPENAI_API_BASE").or_else(|| env_string("OPENAI_BASE_URL")) {
        provider.api_base = Some(value);
        keys.push("OPENAI_API_BASE".to_string());
    }
    if let Some(value) = env_string("ROVE_PROVIDER_API_BASE") {
        provider.api_base = Some(value);
        keys.push("ROVE_PROVIDER_API_BASE".to_string());
    }
    if let Some(value) = env_string("OPENAI_API_KEY") {
        provider.api_key = Some(value);
        keys.push("OPENAI_API_KEY".to_string());
    }
    if let Some(value) = env_string("ANTHROPIC_API_KEY") {
        provider.anthropic_api_key = Some(value);
        keys.push("ANTHROPIC_API_KEY".to_string());
    }
    if let Some(value) = env_string("ROVE_MODEL") {
        provider.model = Some(value);
        keys.push("ROVE_MODEL".to_string());
    }
    if let Some(value) = env_string("ROVE_OPENAI_RESPONSES_PROMPT_CACHE") {
        provider.responses_prompt_cache = Some(parse_env_bool(
            "ROVE_OPENAI_RESPONSES_PROMPT_CACHE",
            &value,
        )?);
        keys.push("ROVE_OPENAI_RESPONSES_PROMPT_CACHE".to_string());
    }
    if let Some(value) = env_string("ROVE_OPENAI_RESPONSES_PROMPT_CACHE_RETENTION") {
        provider.responses_prompt_cache_retention = Some(Some(value));
        keys.push("ROVE_OPENAI_RESPONSES_PROMPT_CACHE_RETENTION".to_string());
    }
    if let Some(value) = env_string("ROVE_FALLBACK_MODELS") {
        provider.fallback_models = Some(parse_csv(&value));
        keys.push("ROVE_FALLBACK_MODELS".to_string());
    }
    if let Some(value) = env_string("ROVE_FALLBACK_PROVIDERS") {
        provider.fallback_providers = Some(serde_json::from_str(&value)?);
        keys.push("ROVE_FALLBACK_PROVIDERS".to_string());
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

    let mut rag = RagConfigLayer::default();
    if let Some(value) = env_string("ROVE_RAG_DETERMINISTIC") {
        rag.deterministic = Some(parse_env_bool("ROVE_RAG_DETERMINISTIC", &value)?);
        keys.push("ROVE_RAG_DETERMINISTIC".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_EMBEDDING_PROVIDER") {
        rag.embedding_provider = Some(value);
        keys.push("ROVE_RAG_EMBEDDING_PROVIDER".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_EMBEDDING_MODEL") {
        rag.embedding_model = Some(value);
        keys.push("ROVE_RAG_EMBEDDING_MODEL".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_EMBEDDING_API_BASE") {
        rag.embedding_api_base = Some(value);
        keys.push("ROVE_RAG_EMBEDDING_API_BASE".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_EMBEDDING_API_KEY") {
        rag.embedding_api_key = Some(value);
        keys.push("ROVE_RAG_EMBEDDING_API_KEY".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_RERANK_PROVIDER") {
        rag.rerank_provider = Some(Some(value));
        keys.push("ROVE_RAG_RERANK_PROVIDER".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_RERANK_MODEL") {
        rag.rerank_model = Some(Some(value));
        keys.push("ROVE_RAG_RERANK_MODEL".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_RERANK_API_KEY") {
        rag.rerank_api_key = Some(Some(value));
        keys.push("ROVE_RAG_RERANK_API_KEY".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_TIMEOUT_MS") {
        rag.timeout_ms = Some(parse_env("ROVE_RAG_TIMEOUT_MS", &value)?);
        keys.push("ROVE_RAG_TIMEOUT_MS".to_string());
    }
    if let Some(value) = env_string("ROVE_RAG_FALLBACK_TO_DETERMINISTIC") {
        rag.fallback_to_deterministic = Some(parse_env_bool(
            "ROVE_RAG_FALLBACK_TO_DETERMINISTIC",
            &value,
        )?);
        keys.push("ROVE_RAG_FALLBACK_TO_DETERMINISTIC".to_string());
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
            rag: Some(rag).filter(has_rag_values),
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
    layer.name.is_some()
        || layer.api_base.is_some()
        || layer.api_key.is_some()
        || layer.anthropic_api_key.is_some()
        || layer.model.is_some()
        || layer.responses_prompt_cache.is_some()
        || layer.responses_prompt_cache_retention.is_some()
        || layer.fallback_models.is_some()
        || layer.fallback_providers.is_some()
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

fn has_rag_values(layer: &RagConfigLayer) -> bool {
    layer.deterministic.is_some()
        || layer.embedding_provider.is_some()
        || layer.embedding_model.is_some()
        || layer.embedding_api_base.is_some()
        || layer.embedding_api_key.is_some()
        || layer.rerank_provider.is_some()
        || layer.rerank_model.is_some()
        || layer.rerank_api_key.is_some()
        || layer.timeout_ms.is_some()
        || layer.fallback_to_deterministic.is_some()
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

    fn clear_config_env() {
        for key in [
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
            "ROVE_MCP_CONFIG",
            "ROVE_SHELL_TIMEOUT_MS",
            "ROVE_SHELL_MAX_OUTPUT_BYTES",
            "ROVE_SHELL_INHERIT_ENVIRONMENT",
            "ROVE_SHELL_DENYLIST",
            "ROVE_API_BIND_ADDR",
            "ROVE_API_TOKEN",
            "ROVE_API_UNSAFE_REMOTE_WITHOUT_AUTH",
            "ROVE_RAG_DETERMINISTIC",
            "ROVE_RAG_EMBEDDING_PROVIDER",
            "ROVE_RAG_EMBEDDING_MODEL",
            "ROVE_RAG_EMBEDDING_API_BASE",
            "ROVE_RAG_EMBEDDING_API_KEY",
            "ROVE_RAG_RERANK_PROVIDER",
            "ROVE_RAG_RERANK_MODEL",
            "ROVE_RAG_RERANK_API_KEY",
            "ROVE_RAG_TIMEOUT_MS",
            "ROVE_RAG_FALLBACK_TO_DETERMINISTIC",
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
name = "ollama"
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
            },
        )
        .unwrap();

        assert_eq!(config.provider.name, "ollama");
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
    fn fallback_provider_config_defaults_to_openai_compatible() {
        let fallback: FallbackProviderConfig = serde_json::from_str(
            r#"{
                "api_base": "https://fallback.test/v1",
                "api_key": "fallback-secret",
                "model": "fallback-model"
            }"#,
        )
        .unwrap();

        assert_eq!(fallback.name, "openai-compatible");
        assert_eq!(fallback.model, "fallback-model");
    }

    #[test]
    fn validation_accepts_openai_responses_provider_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[provider]
name = "openai-responses"
model = "gpt-4.1-mini"
api_base = "https://api.openai.com/v1"
"#,
        )
        .unwrap();

        let config = AppConfig::load(tmp.path(), AppConfigOverrides::default()).unwrap();

        assert_eq!(config.provider.name, "openai-responses");
    }

    #[test]
    fn validation_accepts_openai_responses_fallback_provider() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[provider]
name = "openai-compatible"
model = "primary-model"
fallback_providers = [
  { name = "openai-responses", api_base = "https://api.openai.com/v1", api_key = "secret", model = "gpt-4.1-mini" }
]
"#,
        )
        .unwrap();

        let config = AppConfig::load(tmp.path(), AppConfigOverrides::default()).unwrap();

        assert_eq!(
            config.provider.fallback_providers[0].name,
            "openai-responses"
        );
    }

    #[test]
    fn validation_rejects_unknown_fallback_provider_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_dir = tmp.path().join(".rove");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::write(
            config_dir.join("config.toml"),
            r#"
[provider]
fallback_providers = [
  { name = "unknown", api_base = "https://fallback.test/v1", api_key = "secret", model = "fallback-model" }
]
"#,
        )
        .unwrap();

        let err = AppConfig::load(tmp.path(), AppConfigOverrides::default()).unwrap_err();

        assert!(
            err.to_string()
                .contains("invalid fallback provider `unknown`")
        );
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

        let err = AppConfig::load(tmp.path(), AppConfigOverrides::default()).unwrap_err();

        assert!(
            err.to_string()
                .contains("runtime.system_prompt_path resolves outside the workspace")
        );
    }
}
