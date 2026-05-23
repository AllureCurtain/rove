use crate::config::AppConfig;
use crate::models::anthropic::AnthropicClient;
use crate::models::ollama::OllamaClient;
use crate::models::openai::OpenAiClient;
use crate::models::routing::{HealthConfig, RoutingModelClient};
use crate::models::traits::ModelClient;
use std::time::Duration;

pub fn build_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    match config.provider.as_str() {
        "anthropic" => build_anthropic_model_client(config, model_id),
        "ollama" => build_ollama_model_client(config, model_id),
        _ => build_openai_model_client(config, model_id),
    }
}

pub fn build_openai_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    let primary = openai_client(config.api_base.clone(), config.api_key.clone(), model_id);
    if config.fallback_models.is_empty() && config.fallback_providers.is_empty() {
        return primary;
    }

    let mut fallbacks: Vec<Box<dyn ModelClient>> = config
        .fallback_models
        .iter()
        .cloned()
        .map(|model| openai_client(config.api_base.clone(), config.api_key.clone(), model))
        .collect();
    fallbacks.extend(config.fallback_providers.iter().map(|provider| {
        openai_client(
            provider.api_base.clone(),
            provider.api_key.clone(),
            provider.model.clone(),
        )
    }));
    Box::new(
        RoutingModelClient::new(primary, fallbacks).with_health_config(HealthConfig {
            failure_threshold: config.routing_failure_threshold,
            open_cooldown: Duration::from_millis(config.routing_open_cooldown_ms),
        }),
    )
}

pub fn build_anthropic_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    let api_base = if config.api_base.contains("anthropic") {
        config.api_base.clone()
    } else {
        "https://api.anthropic.com".to_string()
    };
    let api_key = if config.anthropic_api_key.is_empty() {
        config.api_key.clone()
    } else {
        config.anthropic_api_key.clone()
    };
    Box::new(AnthropicClient::new(api_base, api_key, model_id))
}

pub fn build_ollama_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    let api_base = if config.api_base.contains("openai") {
        String::new()
    } else {
        config.api_base.clone()
    };
    Box::new(OllamaClient::new(api_base, model_id))
}

fn openai_client(api_base: String, api_key: String, model_id: String) -> Box<dyn ModelClient> {
    Box::new(OpenAiClient::new(api_base, api_key, model_id))
}
