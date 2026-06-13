//! Provider profile normalization and model inventory.
//!
//! Turns an inbound [`ProviderProfileRequest`] into a concrete [`AppConfig`]
//! provider section, and lists account-visible models for the supported
//! OpenAI-compatible, Anthropic, and Ollama provider shapes. Kept separate from
//! the request handlers in [`super`] because it is the network-facing,
//! provider-specific surface of the API.

use crate::config::AppConfig;

use super::{ApiError, ProviderProfileRequest};

const DEFAULT_PROVIDER_KEY_ENV: &str = "OPENAI_API_KEY";
const DEFAULT_ANTHROPIC_KEY_ENV: &str = "ANTHROPIC_API_KEY";

pub(super) fn apply_provider_profile(
    config: &mut AppConfig,
    profile: &ProviderProfileRequest,
    model: Option<&str>,
) -> Result<(), ApiError> {
    let profile = normalize_provider_profile(profile)?;
    let key_env = provider_key_env(&profile);
    let api_key = provider_api_key(&profile.name, &key_env)?;
    config.provider.name = profile.name.clone();
    config.provider.api_base = profile.api_base;
    config.provider.api_key = api_key.clone();
    config.provider.anthropic_api_key = if profile.name == "anthropic" {
        api_key
    } else {
        String::new()
    };
    if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
        config.provider.model = model.trim().to_string();
    }
    config.provider.fallback_models.clear();
    config.provider.fallback_providers.clear();
    Ok(())
}

pub(super) fn normalize_provider_profile(
    profile: &ProviderProfileRequest,
) -> Result<ProviderProfileRequest, ApiError> {
    let name = profile.name.trim().to_ascii_lowercase();
    let canonical_name = match name.as_str() {
        "openai" | "openai-compatible" => "openai-compatible",
        "openai-responses" | "responses" => "openai-responses",
        "anthropic" => "anthropic",
        "ollama" => "ollama",
        "fake" => "fake",
        _ => {
            return Err(ApiError::bad_request(
                "provider profile supports openai-compatible, openai-responses, anthropic, ollama, or fake providers",
            ));
        }
    };
    let api_base = profile.api_base.trim().trim_end_matches('/').to_string();
    if matches!(
        canonical_name,
        "openai-compatible" | "openai-responses" | "anthropic"
    ) && api_base.is_empty()
    {
        return Err(ApiError::bad_request("provider.api_base must not be empty"));
    }
    if canonical_name == "ollama" && api_base.is_empty() {
        return Err(ApiError::bad_request(
            "provider.api_base must not be empty for ollama providers",
        ));
    }
    Ok(ProviderProfileRequest {
        name: canonical_name.to_string(),
        api_base,
        api_key_env: profile.api_key_env.clone(),
    })
}

pub(super) fn provider_key_env(profile: &ProviderProfileRequest) -> String {
    let default = match profile.name.as_str() {
        "anthropic" => DEFAULT_ANTHROPIC_KEY_ENV,
        _ => DEFAULT_PROVIDER_KEY_ENV,
    };
    profile
        .api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_string()
}

fn provider_api_key(provider: &str, key_env: &str) -> Result<String, ApiError> {
    if matches!(provider, "ollama" | "fake") {
        return Ok(String::new());
    }
    std::env::var(key_env)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ApiError::bad_request(format!("provider key env '{key_env}' is not set")))
}

pub(super) struct ProviderInventory {
    pub(super) key_present: bool,
    pub(super) models: Vec<String>,
}

pub(super) async fn provider_inventory(
    profile: &ProviderProfileRequest,
    key_env: &str,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    match profile.name.as_str() {
        "openai-compatible" | "openai-responses" => {
            openai_compatible_inventory(profile, key_env, requested_endpoint).await
        }
        "anthropic" => anthropic_inventory(profile, key_env, requested_endpoint).await,
        "ollama" => ollama_inventory(profile, requested_endpoint).await,
        "fake" => Ok(ProviderInventory {
            key_present: false,
            models: vec!["fake".to_string(), "fake-raw".to_string()],
        }),
        _ => Err(ApiError::bad_request("unsupported provider profile")),
    }
}

async fn openai_compatible_inventory(
    profile: &ProviderProfileRequest,
    key_env: &str,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let api_key = provider_api_key(&profile.name, key_env)?;
    let endpoint = requested_endpoint
        .filter(|endpoint| !endpoint.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/models", profile.api_base.trim_end_matches('/')));
    let response = reqwest::Client::new()
        .get(&endpoint)
        .bearer_auth(&api_key)
        .send()
        .await
        .map_err(|err| ApiError::bad_gateway(format!("provider model inventory failed: {err}")))?;
    model_inventory_response(response, "data", "id", true).await
}

async fn anthropic_inventory(
    profile: &ProviderProfileRequest,
    key_env: &str,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let api_key = provider_api_key(&profile.name, key_env)?;
    let endpoint = requested_endpoint
        .filter(|endpoint| !endpoint.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/v1/models", profile.api_base.trim_end_matches('/')));
    let response = reqwest::Client::new()
        .get(&endpoint)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|err| ApiError::bad_gateway(format!("provider model inventory failed: {err}")))?;
    model_inventory_response(response, "data", "id", true).await
}

async fn ollama_inventory(
    profile: &ProviderProfileRequest,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let endpoint = requested_endpoint
        .filter(|endpoint| !endpoint.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}/api/tags", profile.api_base.trim_end_matches('/')));
    let response = reqwest::Client::new()
        .get(&endpoint)
        .send()
        .await
        .map_err(|err| ApiError::bad_gateway(format!("provider model inventory failed: {err}")))?;
    model_inventory_response(response, "models", "name", false).await
}

async fn model_inventory_response(
    response: reqwest::Response,
    array_field: &str,
    id_field: &str,
    key_present: bool,
) -> Result<ProviderInventory, ApiError> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(ApiError::bad_gateway(format!(
            "provider model inventory returned HTTP {status}: {body}"
        )));
    }
    let body: serde_json::Value = response.json().await.map_err(|err| {
        ApiError::bad_gateway(format!("provider model inventory JSON failed: {err}"))
    })?;
    let models = body
        .get(array_field)
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.get(id_field)
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .collect();
    Ok(ProviderInventory {
        key_present,
        models,
    })
}
