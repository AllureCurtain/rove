use std::{collections::BTreeMap, sync::Arc};

use futures::StreamExt;
use reqwest::{Method, StatusCode, header::HeaderMap};
use rove_app_bootstrap::{
    AppConfig, FallbackProviderConfig, ProviderAuthConfig, ProviderProfileConfig,
    build_model_client, build_openai_model_client, default_wire_protocol_registry,
    try_build_model_client, try_build_model_client_with_registry,
};
use rove_models::provider::{
    AuthStyle, Framing, StreamDecoder, WireProtocol, WireProtocolId, WireProtocolRegistry,
    WireRequest, WireRequestInput,
};
use rove_models::{ModelError, ModelEvent, ProviderOptions};

fn profile(protocol: &str, base_url: &str, model: &str) -> ProviderProfileConfig {
    ProviderProfileConfig {
        wire_protocol: WireProtocolId::new(protocol).unwrap(),
        base_url: base_url.to_string(),
        model: model.to_string(),
        auth: ProviderAuthConfig::None,
        headers: BTreeMap::new(),
        options: ProviderOptions::default(),
        protocol_options: serde_json::json!({}),
    }
}

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
        "routing(openai:https://example.test/v1:primary-model,openai:https://example.test/v1:fallback-a,openai:https://example.test/v1:fallback-b)"
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
            name: "openai".to_string(),
            api_base: "https://fallback-a.test/v1".to_string(),
            api_key: "fallback-a-secret".to_string(),
            model: "provider-a".to_string(),
            options: None,
        },
        FallbackProviderConfig {
            name: "openai".to_string(),
            api_base: "https://fallback-b.test/v1".to_string(),
            api_key: "fallback-b-secret".to_string(),
            model: "provider-b".to_string(),
            options: None,
        },
    ];

    let model = build_openai_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(openai:https://example.test/v1:primary-model,openai:https://fallback-a.test/v1:provider-a,openai:https://fallback-b.test/v1:provider-b)"
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
            options: None,
        },
        FallbackProviderConfig {
            name: "ollama".to_string(),
            api_base: String::new(),
            api_key: String::new(),
            model: "llama-fallback".to_string(),
            options: None,
        },
    ];

    let model = build_model_client(&config, "primary-model".to_string());

    assert_eq!(
        model.model_id(),
        "routing(openai:https://example.test/v1:primary-model,anthropic:https://api.anthropic.com:claude-fallback,ollama:http://localhost:11434:llama-fallback)"
    );
}

#[test]
fn build_model_client_routes_openai_responses_provider() {
    let mut config = AppConfig::default();
    config.provider.name = "openai-responses".to_string();
    config.provider.api_base = "https://api.openai.com/v1".to_string();
    config.provider.api_key = "secret-token".to_string();
    config.provider.model = "gpt-4.1-mini".to_string();
    config.provider.fallback_providers = vec![FallbackProviderConfig {
        name: "openai".to_string(),
        api_base: "https://fallback.test/v1".to_string(),
        api_key: "fallback-secret".to_string(),
        model: "chat-fallback".to_string(),
        options: None,
    }];

    let model = build_model_client(&config, "gpt-4.1-mini".to_string());

    assert_eq!(
        model.model_id(),
        "routing(openai-responses:https://api.openai.com/v1:gpt-4.1-mini,openai:https://fallback.test/v1:chat-fallback)"
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
        "routing(openai:https://example.test/v1:primary-model,openai:https://example.test/v1:fallback-a)"
    );
}

#[test]
fn named_profiles_use_active_override_and_profile_fallback_identity() {
    let mut config = AppConfig::default();
    config.provider.active = Some("gateway".to_string());
    config.provider.profiles.insert(
        "gateway".to_string(),
        profile(
            "openai-chat",
            "https://gateway.example.test/v1",
            "configured-primary",
        ),
    );
    config.provider.profiles.insert(
        "claude".to_string(),
        profile(
            "anthropic-messages",
            "https://api.anthropic.com",
            "claude-fallback",
        ),
    );
    config.provider.fallback_profiles = vec!["claude".to_string()];

    let model = try_build_model_client(&config, "cli-override".to_string()).unwrap();

    assert_eq!(
        model.model_id(),
        "routing(openai:https://gateway.example.test/v1:cli-override,anthropic:https://api.anthropic.com:claude-fallback)"
    );
}

#[test]
fn unknown_named_protocol_fails_without_openai_fallback() {
    let mut config = AppConfig::default();
    config.provider.active = Some("custom".to_string());
    config.provider.profiles.insert(
        "custom".to_string(),
        profile(
            "vendor/not-registered",
            "https://provider.example.test/v1",
            "custom-model",
        ),
    );

    let error = try_build_model_client(&config, "custom-model".to_string())
        .err()
        .unwrap()
        .to_string();

    assert!(error.contains("vendor/not-registered"));
    assert!(error.contains("openai-chat"));
    assert!(!error.contains("secret"));
}

#[tokio::test]
async fn infallible_builder_surfaces_typed_configuration_error() {
    let mut config = AppConfig::default();
    config.provider.active = Some("custom".to_string());
    config.provider.profiles.insert(
        "custom".to_string(),
        profile(
            "vendor/not-registered",
            "https://provider.example.test/v1",
            "custom-model",
        ),
    );

    let client = build_model_client(&config, "custom-model".to_string());
    let event = client.stream(&[], &[]).next().await.unwrap();

    assert!(matches!(
        event,
        Err(ModelError::InvalidConfiguration(message))
            if message.contains("vendor/not-registered")
    ));
}

struct NoopDecoder;

impl StreamDecoder for NoopDecoder {
    fn push(&mut self, _frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
        Ok(Vec::new())
    }
}

struct CustomProtocol {
    id: WireProtocolId,
}

impl WireProtocol for CustomProtocol {
    fn id(&self) -> &WireProtocolId {
        &self.id
    }

    fn build_request(&self, _input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
        Ok(WireRequest {
            method: Method::POST,
            path: "stream".to_string(),
            headers: HeaderMap::new(),
            body: serde_json::json!({}),
        })
    }

    fn framing(&self) -> Framing {
        Framing::JsonLines
    }

    fn decoder(&self) -> Box<dyn StreamDecoder> {
        Box::new(NoopDecoder)
    }

    fn classify_error(&self, _status: StatusCode, _headers: &HeaderMap, _body: &str) -> ModelError {
        ModelError::RequestFailed("custom protocol request failed".to_string())
    }

    fn default_auth_style(&self) -> AuthStyle {
        AuthStyle::None
    }
}

#[test]
fn injected_registry_builds_custom_protocol_without_factory_switch() {
    let mut registry = WireProtocolRegistry::new();
    registry
        .register(Arc::new(CustomProtocol {
            id: WireProtocolId::new("vendor/custom").unwrap(),
        }))
        .unwrap();
    let mut config = AppConfig::default();
    config.provider.active = Some("custom".to_string());
    config.provider.profiles.insert(
        "custom".to_string(),
        profile(
            "vendor/custom",
            "https://provider.example.test/v1",
            "custom-model",
        ),
    );

    let model = try_build_model_client_with_registry(
        &config,
        "custom-model".to_string(),
        Arc::new(registry),
    )
    .unwrap();

    assert_eq!(
        model.client_id().as_str(),
        "vendor/custom:https://provider.example.test/v1:custom-model"
    );
    assert_eq!(default_wire_protocol_registry().len(), 4);
}
