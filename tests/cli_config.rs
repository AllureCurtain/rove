use std::path::PathBuf;

use rove::config::{AppConfig, FallbackProviderConfig};
use rove::interfaces::cli::config::format_effective_config;

#[test]
fn format_effective_config_prints_json_without_secret_value() {
    let config = AppConfig {
        provider: "openai".to_string(),
        api_base: "https://example.test/v1".to_string(),
        api_key: "secret-token".to_string(),
        anthropic_api_key: "anthropic-secret".to_string(),
        model: "model-a".to_string(),
        fallback_models: vec!["model-b".to_string(), "model-c".to_string()],
        fallback_providers: vec![FallbackProviderConfig {
            api_base: "https://fallback.test/v1".to_string(),
            api_key: "fallback-secret".to_string(),
            model: "fallback-provider-model".to_string(),
        }],
        routing_failure_threshold: 5,
        routing_open_cooldown_ms: 45_000,
        max_steps: 42,
        system_prompt_path: PathBuf::from("prompts/custom.md"),
        mcp_config_path: PathBuf::from(".rove/custom-mcp.json"),
    };

    let output = format_effective_config(&config);
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["provider"], "openai");
    assert_eq!(json["api_base"], "https://example.test/v1");
    assert_eq!(json["api_key_set"], true);
    assert_eq!(json["anthropic_api_key_set"], true);
    assert_eq!(json["model"], "model-a");
    assert_eq!(json["fallback_models"][0], "model-b");
    assert_eq!(json["fallback_models"][1], "model-c");
    assert_eq!(
        json["fallback_providers"][0]["api_base"],
        "https://fallback.test/v1"
    );
    assert_eq!(json["fallback_providers"][0]["api_key_set"], true);
    assert_eq!(
        json["fallback_providers"][0]["model"],
        "fallback-provider-model"
    );
    assert_eq!(json["routing_failure_threshold"], 5);
    assert_eq!(json["routing_open_cooldown_ms"], 45_000);
    assert_eq!(json["max_steps"], 42);
    assert_eq!(json["system_prompt_path"], "prompts/custom.md");
    assert_eq!(json["mcp_config_path"], ".rove/custom-mcp.json");
    assert!(!output.contains("secret-token"));
    assert!(!output.contains("fallback-secret"));
    assert!(!output.contains("anthropic-secret"));
}
