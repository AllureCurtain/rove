use crate::config::AppConfig;
use crate::config::ProviderOptions;
use crate::models::anthropic::AnthropicClient;
use crate::models::fake::FakeModelClient;
use crate::models::health::{HealthConfig, ModelHealthStore};
use crate::models::ollama::OllamaClient;
use crate::models::openai::OpenAiClient;
use crate::models::openai_responses::OpenAiResponsesClient;
use crate::models::routing::{RetryPolicy, RoutingModelClient};
use crate::models::traits::ModelClient;
use std::sync::Arc;
use std::time::Duration;

pub fn build_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_routed_model_client(config, ProviderSpec::primary(config, model_id), None)
}

pub fn build_model_client_with_health(
    config: &AppConfig,
    model_id: String,
    health: Arc<ModelHealthStore>,
) -> Box<dyn ModelClient> {
    build_routed_model_client(
        config,
        ProviderSpec::primary(config, model_id),
        Some(health),
    )
}

pub fn build_openai_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_routed_model_client(
        config,
        ProviderSpec::openai_compatible(config, model_id),
        None,
    )
}

pub fn build_anthropic_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_routed_model_client(config, ProviderSpec::anthropic(config, model_id), None)
}

pub fn build_ollama_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_routed_model_client(config, ProviderSpec::ollama(config, model_id), None)
}

fn build_routed_model_client(
    config: &AppConfig,
    primary: ProviderSpec,
    health: Option<Arc<ModelHealthStore>>,
) -> Box<dyn ModelClient> {
    let fallback_specs = fallback_specs(config, &primary);
    let primary = build_provider_client(primary);
    if fallback_specs.is_empty() {
        return primary;
    }

    let fallbacks = fallback_specs
        .into_iter()
        .map(build_provider_client)
        .collect::<Vec<_>>();
    let routed = match health {
        Some(health) => RoutingModelClient::with_health_store(primary, fallbacks, health),
        None => RoutingModelClient::new(primary, fallbacks).with_health_config(HealthConfig {
            failure_threshold: config.routing.failure_threshold,
            open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
        }),
    };
    Box::new(routed.with_retry_policy(RetryPolicy {
        max_attempts: config.routing.retry_max_attempts,
        backoff_base: Duration::from_millis(config.routing.retry_backoff_base_ms),
        backoff_max: Duration::from_millis(config.routing.retry_backoff_max_ms),
    }))
}

fn fallback_specs(config: &AppConfig, primary: &ProviderSpec) -> Vec<ProviderSpec> {
    let mut fallbacks = config
        .provider
        .fallback_models
        .iter()
        .cloned()
        .map(|model| primary.with_model(model))
        .collect::<Vec<_>>();

    fallbacks.extend(
        config
            .provider
            .fallback_providers
            .iter()
            .map(ProviderSpec::fallback),
    );
    fallbacks
}

fn build_provider_client(spec: ProviderSpec) -> Box<dyn ModelClient> {
    match spec.kind {
        ProviderKind::OpenAiCompatible => {
            let mut client = OpenAiClient::new(spec.api_base, spec.api_key, spec.model);
            client.apply_options(&spec.options);
            Box::new(client)
        }
        ProviderKind::OpenAiResponses => {
            let mut client = OpenAiResponsesClient::new(spec.api_base, spec.api_key, spec.model)
                .with_prompt_cache(
                    spec.responses_prompt_cache,
                    spec.responses_prompt_cache_retention,
                );
            client.apply_options(&spec.options);
            Box::new(client)
        }
        ProviderKind::Anthropic => {
            let mut client =
                AnthropicClient::new(anthropic_base(spec.api_base), spec.api_key, spec.model);
            client.apply_options(&spec.options);
            Box::new(client)
        }
        ProviderKind::Ollama => {
            let mut client = OllamaClient::new(ollama_base(spec.api_base), spec.model);
            client.apply_options(&spec.options);
            Box::new(client)
        }
        ProviderKind::Fake => Box::new(FakeModelClient::new(format!(
            "fake response from {}",
            spec.model
        ))),
    }
}

#[derive(Clone)]
struct ProviderSpec {
    kind: ProviderKind,
    api_base: String,
    api_key: String,
    model: String,
    options: ProviderOptions,
    responses_prompt_cache: bool,
    responses_prompt_cache_retention: Option<String>,
}

impl ProviderSpec {
    fn primary(config: &AppConfig, model: String) -> Self {
        match ProviderKind::from_name(&config.provider.name) {
            ProviderKind::OpenAiCompatible => Self::openai_compatible(config, model),
            ProviderKind::OpenAiResponses => Self::openai_responses(config, model),
            ProviderKind::Anthropic => Self::anthropic(config, model),
            ProviderKind::Ollama => Self::ollama(config, model),
            ProviderKind::Fake => Self::fake(model),
        }
    }

    fn openai_compatible(config: &AppConfig, model: String) -> Self {
        Self {
            kind: ProviderKind::OpenAiCompatible,
            api_base: config.provider.api_base.clone(),
            api_key: config.provider.api_key.clone(),
            model,
            options: config.provider.options,
            responses_prompt_cache: false,
            responses_prompt_cache_retention: None,
        }
    }

    fn openai_responses(config: &AppConfig, model: String) -> Self {
        Self {
            kind: ProviderKind::OpenAiResponses,
            api_base: config.provider.api_base.clone(),
            api_key: config.provider.api_key.clone(),
            model,
            options: config.provider.options,
            responses_prompt_cache: config.provider.responses_prompt_cache,
            responses_prompt_cache_retention: config
                .provider
                .responses_prompt_cache_retention
                .clone(),
        }
    }

    fn anthropic(config: &AppConfig, model: String) -> Self {
        let api_key = if config.provider.anthropic_api_key.is_empty() {
            config.provider.api_key.clone()
        } else {
            config.provider.anthropic_api_key.clone()
        };
        Self {
            kind: ProviderKind::Anthropic,
            api_base: config.provider.api_base.clone(),
            api_key,
            model,
            options: config.provider.options,
            responses_prompt_cache: false,
            responses_prompt_cache_retention: None,
        }
    }

    fn ollama(config: &AppConfig, model: String) -> Self {
        Self {
            kind: ProviderKind::Ollama,
            api_base: config.provider.api_base.clone(),
            api_key: String::new(),
            model,
            options: config.provider.options,
            responses_prompt_cache: false,
            responses_prompt_cache_retention: None,
        }
    }

    fn fake(model: String) -> Self {
        Self {
            kind: ProviderKind::Fake,
            api_base: String::new(),
            api_key: String::new(),
            model,
            options: ProviderOptions::default(),
            responses_prompt_cache: false,
            responses_prompt_cache_retention: None,
        }
    }

    fn fallback(provider: &crate::config::FallbackProviderConfig) -> Self {
        Self {
            kind: ProviderKind::from_name(&provider.name),
            api_base: provider.api_base.clone(),
            api_key: provider.api_key.clone(),
            model: provider.model.clone(),
            options: provider.options.unwrap_or_default(),
            responses_prompt_cache: false,
            responses_prompt_cache_retention: None,
        }
    }

    fn with_model(&self, model: String) -> Self {
        Self {
            model,
            ..self.clone()
        }
    }
}

#[derive(Clone, Copy)]
enum ProviderKind {
    OpenAiCompatible,
    OpenAiResponses,
    Anthropic,
    Ollama,
    Fake,
}

impl ProviderKind {
    fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "openai-responses" | "responses" => Self::OpenAiResponses,
            "anthropic" => Self::Anthropic,
            "ollama" => Self::Ollama,
            "fake" => Self::Fake,
            _ => Self::OpenAiCompatible,
        }
    }
}

fn anthropic_base(api_base: String) -> String {
    let trimmed = api_base.trim();
    if trimmed.is_empty() || trimmed.contains("api.openai.com") {
        "https://api.anthropic.com".to_string()
    } else {
        api_base
    }
}

fn ollama_base(api_base: String) -> String {
    if api_base.contains("openai") {
        String::new()
    } else {
        api_base
    }
}
