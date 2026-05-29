use rove::config::{AppConfig, FallbackProviderConfig};
use rove::models::factory::{build_model_client, build_openai_model_client};

#[test]
fn build_openai_model_client_uses_configured_fallback_models() {
    let mut config = AppConfig::default();
    config.provider.name = "openai".to_string();
    config.provider.api_base = "https://example.test/v1".to_string();
    config.provider.api_key = "secret-token".to_string();
    config.provider.model = "primary-model".to_string();
    config.provider.fallback_models = vec!["fallback-a".to_string(), "fallback-b".to_string()];

    let model = build_openai_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(openai-compatible:https://example.test/v1:primary-model,openai-compatible:https://example.test/v1:fallback-a,openai-compatible:https://example.test/v1:fallback-b)"
    );
}

#[test]
fn build_openai_model_client_uses_configured_fallback_providers() {
    let mut config = AppConfig::default();
    config.provider.name = "openai".to_string();
    config.provider.api_base = "https://example.test/v1".to_string();
    config.provider.api_key = "secret-token".to_string();
    config.provider.model = "primary-model".to_string();
    config.provider.fallback_providers = vec![
        FallbackProviderConfig {
            name: "openai-compatible".to_string(),
            api_base: "https://fallback-a.test/v1".to_string(),
            api_key: "fallback-a-secret".to_string(),
            model: "provider-a".to_string(),
        },
        FallbackProviderConfig {
            name: "openai-compatible".to_string(),
            api_base: "https://fallback-b.test/v1".to_string(),
            api_key: "fallback-b-secret".to_string(),
            model: "provider-b".to_string(),
        },
    ];

    let model = build_openai_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(openai-compatible:https://example.test/v1:primary-model,openai-compatible:https://fallback-a.test/v1:provider-a,openai-compatible:https://fallback-b.test/v1:provider-b)"
    );
}

#[test]
fn build_model_client_routes_mixed_native_fallback_providers() {
    let mut config = AppConfig::default();
    config.provider.name = "openai".to_string();
    config.provider.api_base = "https://example.test/v1".to_string();
    config.provider.api_key = "secret-token".to_string();
    config.provider.model = "primary-model".to_string();
    config.provider.fallback_providers = vec![
        FallbackProviderConfig {
            name: "anthropic".to_string(),
            api_base: String::new(),
            api_key: "anthropic-secret".to_string(),
            model: "claude-fallback".to_string(),
        },
        FallbackProviderConfig {
            name: "ollama".to_string(),
            api_base: String::new(),
            api_key: String::new(),
            model: "llama-fallback".to_string(),
        },
    ];

    let model = build_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(openai-compatible:https://example.test/v1:primary-model,anthropic:https://api.anthropic.com:claude-fallback,ollama:http://localhost:11434:llama-fallback)"
    );
}

#[test]
fn fallback_models_inherit_primary_provider() {
    let mut config = AppConfig::default();
    config.provider.name = "anthropic".to_string();
    config.provider.api_key = "anthropic-secret".to_string();
    config.provider.model = "claude-primary".to_string();
    config.provider.fallback_models = vec!["claude-fallback".to_string()];

    let model = build_model_client(&config, "claude-primary".to_string());

    assert_eq!(
        model.model_id(),
        "routing(anthropic:https://api.anthropic.com:claude-primary,anthropic:https://api.anthropic.com:claude-fallback)"
    );
}

#[test]
fn build_model_client_supports_fake_provider() {
    let mut config = AppConfig::default();
    config.provider.name = "fake".to_string();
    config.provider.model = "fake-model".to_string();

    let model = build_model_client(&config, "fake-model".to_string());

    assert_eq!(model.model_id(), "fake");
}

#[test]
fn routing_retry_config_does_not_change_target_identity() {
    let mut config = AppConfig::default();
    config.provider.name = "openai".to_string();
    config.provider.api_base = "https://example.test/v1".to_string();
    config.provider.api_key = "secret-token".to_string();
    config.provider.model = "primary-model".to_string();
    config.provider.fallback_models = vec!["fallback-a".to_string()];
    config.routing.retry_max_attempts = 3;
    config.routing.retry_backoff_base_ms = 100;
    config.routing.retry_backoff_max_ms = 1_000;

    let model = build_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(openai-compatible:https://example.test/v1:primary-model,openai-compatible:https://example.test/v1:fallback-a)"
    );
}
