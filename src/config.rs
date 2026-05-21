use std::path::PathBuf;

/// Application configuration.
///
/// Loaded from environment variables and `.rove/config.toml` (when it exists).
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// OpenAI-compatible API base URL.
    pub api_base: String,
    /// API key for the model provider.
    pub api_key: String,
    /// Model identifier to use.
    pub model: String,
    /// Maximum steps per run.
    pub max_steps: u32,
    /// Path to the system prompt file.
    pub system_prompt_path: PathBuf,
}

impl AppConfig {
    /// Load configuration from environment variables.
    ///
    /// Supports:
    /// - OPENAI_API_BASE / OPENAI_BASE_URL
    /// - OPENAI_API_KEY
    /// - ROVE_MODEL (default: gpt-4o)
    /// - ROVE_MAX_STEPS (default: 20)
    /// - ROVE_SYSTEM_PROMPT (default: prompts/system.md)
    pub fn from_env() -> anyhow::Result<Self> {
        // Load .env if present
        let _ = dotenvy::dotenv();

        let api_base = std::env::var("OPENAI_API_BASE")
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());

        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();

        let model = std::env::var("ROVE_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());

        let max_steps: u32 = std::env::var("ROVE_MAX_STEPS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);

        let system_prompt_path = std::env::var("ROVE_SYSTEM_PROMPT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("prompts/system.md"));

        Ok(Self {
            api_base,
            api_key,
            model,
            max_steps,
            system_prompt_path,
        })
    }

    /// Load the system prompt content from file.
    pub fn load_system_prompt(&self) -> String {
        std::fs::read_to_string(&self.system_prompt_path).unwrap_or_else(|_| {
            "You are rove, a helpful assistant that can use tools to accomplish tasks.".to_string()
        })
    }
}
