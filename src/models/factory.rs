use crate::config::AppConfig;
use crate::models::openai::OpenAiClient;
use crate::models::routing::{HealthConfig, RoutingModelClient};
use crate::models::traits::ModelClient;
use std::time::Duration;

pub fn build_openai_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    let primary = openai_client(config, model_id);
    if config.fallback_models.is_empty() {
        return primary;
    }

    let fallbacks = config
        .fallback_models
        .iter()
        .cloned()
        .map(|model| openai_client(config, model))
        .collect();
    Box::new(
        RoutingModelClient::new(primary, fallbacks).with_health_config(HealthConfig {
            failure_threshold: config.routing_failure_threshold,
            open_cooldown: Duration::from_millis(config.routing_open_cooldown_ms),
        }),
    )
}

fn openai_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    Box::new(OpenAiClient::new(
        config.api_base.clone(),
        config.api_key.clone(),
        model_id,
    ))
}
