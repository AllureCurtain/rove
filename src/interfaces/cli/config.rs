use crate::config::AppConfig;

pub fn run() -> anyhow::Result<()> {
    let config = AppConfig::from_env()?;
    println!("{}", format_effective_config(&config));
    Ok(())
}

pub fn format_effective_config(config: &AppConfig) -> String {
    let fallback_providers: Vec<_> = config
        .fallback_providers
        .iter()
        .map(|provider| {
            serde_json::json!({
                "api_base": provider.api_base,
                "api_key_set": !provider.api_key.is_empty(),
                "model": provider.model,
            })
        })
        .collect();
    let value = serde_json::json!({
        "api_base": config.api_base,
        "api_key_set": !config.api_key.is_empty(),
        "model": config.model,
        "fallback_models": config.fallback_models,
        "fallback_providers": fallback_providers,
        "routing_failure_threshold": config.routing_failure_threshold,
        "routing_open_cooldown_ms": config.routing_open_cooldown_ms,
        "max_steps": config.max_steps,
        "system_prompt_path": config.system_prompt_path.to_string_lossy(),
        "mcp_config_path": config.mcp_config_path.to_string_lossy(),
    });
    serde_json::to_string_pretty(&value).expect("effective config snapshot should serialize")
}
