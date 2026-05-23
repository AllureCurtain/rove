use crate::config::AppConfig;
use crate::models::openai::OpenAiClient;
use crate::models::routing::RoutingModelClient;
use crate::models::traits::ModelClient;

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
    Box::new(RoutingModelClient::new(primary, fallbacks))
}

fn openai_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    Box::new(OpenAiClient::new(
        config.api_base.clone(),
        config.api_key.clone(),
        model_id,
    ))
}
