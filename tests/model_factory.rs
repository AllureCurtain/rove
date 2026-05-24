use rove::config::{AppConfig, FallbackProviderConfig};
use rove::models::factory::build_openai_model_client;

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
        "routing(primary-model,fallback-a,fallback-b)"
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
            api_base: "https://fallback-a.test/v1".to_string(),
            api_key: "fallback-a-secret".to_string(),
            model: "provider-a".to_string(),
        },
        FallbackProviderConfig {
            api_base: "https://fallback-b.test/v1".to_string(),
            api_key: "fallback-b-secret".to_string(),
            model: "provider-b".to_string(),
        },
    ];

    let model = build_openai_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(primary-model,provider-a,provider-b)"
    );
}
