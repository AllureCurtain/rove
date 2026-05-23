use std::path::PathBuf;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct FallbackProviderConfig {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
}

/// Application configuration.
///
/// Loaded from environment variables and `.rove/config.toml` (when it exists).
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Provider type: "openai", "anthropic", or "ollama".
    pub provider: String,
    /// OpenAI-compatible API base URL.
    pub api_base: String,
    /// API key for the model provider.
    pub api_key: String,
    /// Anthropic API key (used when provider is "anthropic").
    pub anthropic_api_key: String,
    /// Model identifier to use.
    pub model: String,
    /// Fallback model identifiers to try if the primary model fails before streaming.
    pub fallback_models: Vec<String>,
    /// Full fallback provider configs with independent base URL, API key, and model.
    pub fallback_providers: Vec<FallbackProviderConfig>,
    /// Number of pre-commit failures before a routing candidate circuit opens.
    pub routing_failure_threshold: u32,
    /// Cooldown before an open routing circuit allows a half-open probe.
    pub routing_open_cooldown_ms: u64,
    /// Maximum steps per run.
    pub max_steps: u32,
    /// Path to the system prompt file.
    pub system_prompt_path: PathBuf,
    /// Optional MCP server configuration file.
    pub mcp_config_path: PathBuf,
}

impl AppConfig {
    /// Load configuration from environment variables.
    ///
    /// Supports:
    /// - ROVE_PROVIDER (default: openai) — "openai", "anthropic", or "ollama"
    /// - OPENAI_API_BASE / OPENAI_BASE_URL
    /// - OPENAI_API_KEY
    /// - ANTHROPIC_API_KEY
    /// - ROVE_MODEL (default: gpt-4o)
    /// - ROVE_FALLBACK_MODELS (comma-separated, default: empty)
    /// - ROVE_FALLBACK_PROVIDERS (JSON array, default: empty)
    /// - ROVE_ROUTING_FAILURE_THRESHOLD (default: 3)
    /// - ROVE_ROUTING_OPEN_COOLDOWN_MS (default: 30000)
    /// - ROVE_MAX_STEPS (default: 20)
    /// - ROVE_SYSTEM_PROMPT (default: prompts/system.md)
    /// - ROVE_MCP_CONFIG (default: .rove/mcp_servers.json)
    pub fn from_env() -> anyhow::Result<Self> {
        // Load .env if present
        let _ = dotenvy::dotenv();

        let provider = std::env::var("ROVE_PROVIDER").unwrap_or_else(|_| "openai".to_string());

        let api_base = std::env::var("OPENAI_API_BASE")
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();

        let anthropic_api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_default();

        let model = std::env::var("ROVE_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());

        let fallback_models = std::env::var("ROVE_FALLBACK_MODELS")
            .map(|raw| parse_fallback_models(&raw))
            .unwrap_or_default();

        let fallback_providers = match std::env::var("ROVE_FALLBACK_PROVIDERS") {
            Ok(raw) if !raw.trim().is_empty() => serde_json::from_str(&raw)?,
            _ => Vec::new(),
        };

        let routing_failure_threshold = std::env::var("ROVE_ROUTING_FAILURE_THRESHOLD")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3);

        let routing_open_cooldown_ms = std::env::var("ROVE_ROUTING_OPEN_COOLDOWN_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30_000);

        let max_steps: u32 = std::env::var("ROVE_MAX_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);

        let system_prompt_path = std::env::var("ROVE_SYSTEM_PROMPT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("prompts/system.md"));

        let mcp_config_path = std::env::var("ROVE_MCP_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(".rove/mcp_servers.json"));

        Ok(Self {
            provider,
            api_base,
            api_key,
            anthropic_api_key,
            model,
            fallback_models,
            fallback_providers,
            routing_failure_threshold,
            routing_open_cooldown_ms,
            max_steps,
            system_prompt_path,
            mcp_config_path,
        })
    }

    /// Load the system prompt content from file.
    pub fn load_system_prompt(&self) -> String {
        std::fs::read_to_string(&self.system_prompt_path).unwrap_or_else(|_| {
            "You are rove, a helpful assistant that can use tools to accomplish tasks.".to_string()
        })
    }
}

fn parse_fallback_models(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_fallback_models;

    #[test]
    fn parse_fallback_models_trims_empty_entries() {
        assert_eq!(
            parse_fallback_models(" fallback-a, ,fallback-b,, fallback-c "),
            vec!["fallback-a", "fallback-b", "fallback-c"]
        );
    }
}
