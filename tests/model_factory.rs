use std::path::PathBuf;

use rove::config::{AppConfig, FallbackProviderConfig};
use rove::models::factory::build_openai_model_client;

#[test]
fn build_openai_model_client_uses_configured_fallback_models() {
    let config = AppConfig {
        provider: "openai".to_string(),
        api_base: "https://example.test/v1".to_string(),
        api_key: "secret-token".to_string(),
        anthropic_api_key: String::new(),
        model: "primary-model".to_string(),
        fallback_models: vec!["fallback-a".to_string(), "fallback-b".to_string()],
        fallback_providers: Vec::new(),
        routing_failure_threshold: 3,
        routing_open_cooldown_ms: 30_000,
        max_steps: 20,
        system_prompt_path: PathBuf::from("prompts/system.md"),
        mcp_config_path: PathBuf::from(".rove/mcp_servers.json"),
    };

    let model = build_openai_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(primary-model,fallback-a,fallback-b)"
    );
}

#[test]
fn build_openai_model_client_uses_configured_fallback_providers() {
    let config = AppConfig {
        provider: "openai".to_string(),
        api_base: "https://example.test/v1".to_string(),
        api_key: "secret-token".to_string(),
        anthropic_api_key: String::new(),
        model: "primary-model".to_string(),
        fallback_models: Vec::new(),
        fallback_providers: vec![
            FallbackProviderConfig {
                api_base: "https://fallback-a.test/v1".to_string(),
                api_key: "fallback-a-secret".to_string(),
                model: "provider-a".to_string(),
            },
            FallbackProviderConfig {
                api_base: "https://fallback-b.test/v1".to_string(),
                api_key: "fallback-b-secret".to_string(),
                model: "provider-b".to_string(),
            },
        ],
        routing_failure_threshold: 3,
        routing_open_cooldown_ms: 30_000,
        max_steps: 20,
        system_prompt_path: PathBuf::from("prompts/system.md"),
        mcp_config_path: PathBuf::from(".rove/mcp_servers.json"),
    };

    let model = build_openai_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(primary-model,provider-a,provider-b)"
    );
}
