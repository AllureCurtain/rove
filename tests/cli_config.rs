use rove::config::{AppConfig, FallbackProviderConfig};
use rove::interfaces::cli::config::format_effective_config;

#[test]
fn format_effective_config_prints_json_without_secret_value() {
    let mut config = AppConfig::default();
    config.provider.name = "openai".to_string();
    config.provider.api_base = "https://example.test/v1".to_string();
    config.provider.api_key = "secret-token".to_string();
    config.provider.anthropic_api_key = "anthropic-secret".to_string();
    config.provider.model = "model-a".to_string();
    config.provider.fallback_models = vec!["model-b".to_string(), "model-c".to_string()];
    config.provider.fallback_providers = vec![FallbackProviderConfig {
        name: "anthropic".to_string(),
        api_base: "https://fallback.test/v1".to_string(),
        api_key: "fallback-secret".to_string(),
        model: "fallback-provider-model".to_string(),
    }];
    config.routing.failure_threshold = 5;
    config.routing.open_cooldown_ms = 45_000;
    config.runtime.max_steps = 42;
    config.runtime.system_prompt_path = "prompts/custom.md".into();
    config.tool.mcp_config_path = ".rove/custom-mcp.json".into();

    let output = format_effective_config(&config);
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(json["provider"]["name"], "openai");
    assert_eq!(json["provider"]["api_base"], "https://example.test/v1");
    assert_eq!(json["provider"]["api_key_set"], true);
    assert_eq!(json["provider"]["anthropic_api_key_set"], true);
    assert_eq!(json["provider"]["model"], "model-a");
    assert_eq!(json["provider"]["fallback_models"][0], "model-b");
    assert_eq!(json["provider"]["fallback_models"][1], "model-c");
    assert_eq!(
        json["provider"]["fallback_providers"][0]["name"],
        "anthropic"
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["api_base"],
        "https://fallback.test/v1"
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["api_key_set"],
        true
    );
    assert_eq!(
        json["provider"]["fallback_providers"][0]["model"],
        "fallback-provider-model"
    );
    assert_eq!(json["routing"]["failure_threshold"], 5);
    assert_eq!(json["routing"]["open_cooldown_ms"], 45_000);
    assert_eq!(json["runtime"]["max_steps"], 42);
    assert_eq!(json["runtime"]["system_prompt_path"], "prompts/custom.md");
    assert_eq!(json["tool"]["mcp_config_path"], ".rove/custom-mcp.json");
    assert!(!output.contains("secret-token"));
    assert!(!output.contains("fallback-secret"));
    assert!(!output.contains("anthropic-secret"));
}
