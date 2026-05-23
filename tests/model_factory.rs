use std::path::PathBuf;

use rove::config::AppConfig;
use rove::models::factory::build_openai_model_client;

#[test]
fn build_openai_model_client_uses_configured_fallback_models() {
    let config = AppConfig {
        api_base: "https://example.test/v1".to_string(),
        api_key: "secret-token".to_string(),
        model: "primary-model".to_string(),
        fallback_models: vec!["fallback-a".to_string(), "fallback-b".to_string()],
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
