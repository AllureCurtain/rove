use crate::config::AppConfig;
use crate::models::openai::OpenAiClient;
use crate::models::routing::{HealthConfig, RoutingModelClient};
use crate::models::traits::ModelClient;
use std::time::Duration;

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

fn openai_client(api_base: String, api_key: String, model_id: String) -> Box<dyn ModelClient> {
    Box::new(OpenAiClient::new(api_base, api_key, model_id))
}
