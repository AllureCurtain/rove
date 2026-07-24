//! Provider profile normalization and model inventory.
//!
//! User-facing profiles select a **type** (`openai`, `openai-responses`,
//! `anthropic`, `ollama`, `fake`). The type maps to an internal
//! [`WireProtocolId`]. Display `name` is optional and defaults from `api_base`
//! when omitted. Advanced clients may still set `wire_protocol` directly.

use std::collections::BTreeMap;

use rove_app_bootstrap::AppConfig;
use rove_app_bootstrap::provider::{ProviderAuthConfig, ProviderProfileConfig, SecretSource};
use rove_models::provider::WireProtocolId;

use super::{ApiError, ProviderProfileRequest};

const DEFAULT_PROVIDER_KEY_ENV: &str = "OPENAI_API_KEY";
const DEFAULT_ANTHROPIC_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const JOB_PROVIDER_PROFILE: &str = "__api_request__";

/// Normalized provider identity used by job assembly and inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedProviderProfile {
    /// Stable open wire protocol id (`openai-chat`, `anthropic-messages`, …).
    pub(super) wire_protocol: String,
    /// User-facing channel id when known.
    pub(super) channel: String,
    /// Display name (custom label or derived from `api_base`).
    pub(super) name: String,
    pub(super) api_base: String,
    pub(super) api_key_env: Option<String>,
    pub(super) inventory_family: InventoryFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InventoryFamily {
    OpenAiCompatible,
    Anthropic,
    Ollama,
    Fake,
}

pub(super) fn apply_provider_profile(
    config: &mut AppConfig,
    profile: &ProviderProfileRequest,
    model: Option<&str>,
) -> Result<(), ApiError> {
    let profile = normalize_provider_profile(profile)?;
    let model = model
        .filter(|model| !model.trim().is_empty())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let model = if model.is_empty() {
        let fallback = config.provider.model.trim();
        if fallback.is_empty() {
            return Err(ApiError::bad_request(
                "provider profile requests require an explicit model id",
            ));
        }
        fallback.to_string()
    } else {
        model
    };

    let wire_protocol = WireProtocolId::new(profile.wire_protocol.clone()).map_err(|error| {
        ApiError::bad_request(format!("provider wire_protocol is invalid: {error}"))
    })?;
    let key_env = provider_key_env(&profile);
    // Resolve once at request time so missing secrets fail before the job
    // starts, and so request-scoped env vars can be cleared by callers/tests
    // before the async job builds its model client.
    let api_key = provider_api_key(&profile.inventory_family, &key_env)?;
    let auth = match profile.inventory_family {
        InventoryFamily::OpenAiCompatible => {
            if api_key.is_empty() {
                ProviderAuthConfig::None
            } else {
                ProviderAuthConfig::Bearer {
                    secret: SecretSource::Literal(api_key),
                }
            }
        }
        InventoryFamily::Anthropic => {
            if api_key.is_empty() {
                ProviderAuthConfig::None
            } else {
                ProviderAuthConfig::Header {
                    header: "x-api-key".to_string(),
                    secret: SecretSource::Literal(api_key),
                }
            }
        }
        InventoryFamily::Ollama | InventoryFamily::Fake => ProviderAuthConfig::None,
    };

    let mut profiles = BTreeMap::new();
    profiles.insert(
        JOB_PROVIDER_PROFILE.to_string(),
        ProviderProfileConfig {
            wire_protocol,
            base_url: if profile.inventory_family == InventoryFamily::Fake {
                String::new()
            } else {
                profile.api_base.clone()
            },
            model: model.clone(),
            auth,
            headers: BTreeMap::new(),
            options: config.provider.options,
            protocol_options: serde_json::json!({}),
        },
    );

    config.provider.active = Some(JOB_PROVIDER_PROFILE.to_string());
    config.provider.profiles = profiles;
    config.provider.fallback_profiles.clear();
    config.provider.fallback_models.clear();
    config.provider.fallback_providers.clear();
    config.provider.model = model;
    // Keep legacy fields coherent for dump-config and older diagnostics without
    // driving assembly (named profiles take precedence when present).
    config.provider.name = profile.name;
    config.provider.api_base = profile.api_base;
    config.provider.api_key.clear();
    config.provider.anthropic_api_key.clear();
    Ok(())
}

pub(super) fn normalize_provider_profile(
    profile: &ProviderProfileRequest,
) -> Result<NormalizedProviderProfile, ApiError> {
    let explicit_protocol = profile
        .wire_protocol
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let channel = profile
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let raw_name = profile.name.trim();

    let (wire_protocol, channel_id, inventory_family) = if let Some(protocol) = explicit_protocol {
        let (wire, channel_id, family) = resolve_protocol_identity(protocol)?;
        // Optional channel must agree when both are set.
        if let Some(ref ch) = channel {
            let from_channel = resolve_channel_identity(ch)?;
            if from_channel.0 != wire {
                return Err(ApiError::bad_request(format!(
                    "provider.channel `{ch}` does not match wire_protocol `{wire}`"
                )));
            }
        }
        (wire, channel_id, family)
    } else if let Some(ref ch) = channel {
        resolve_channel_identity(ch)?
    } else {
        return Err(ApiError::bad_request(
            "provider profile requires a type (openai, openai-responses, anthropic, ollama, or fake); advanced clients may set wire_protocol instead",
        ));
    };

    let api_base = profile.api_base.trim().trim_end_matches('/').to_string();
    match inventory_family {
        InventoryFamily::OpenAiCompatible | InventoryFamily::Anthropic => {
            if api_base.is_empty() {
                return Err(ApiError::bad_request("provider.api_base must not be empty"));
            }
        }
        InventoryFamily::Ollama => {
            if api_base.is_empty() {
                return Err(ApiError::bad_request(
                    "provider.api_base must not be empty for ollama providers",
                ));
            }
        }
        InventoryFamily::Fake => {}
    }

    WireProtocolId::new(&wire_protocol).map_err(|error| {
        ApiError::bad_request(format!("provider wire_protocol is invalid: {error}"))
    })?;

    let display_name = display_name_for_profile(raw_name, &api_base, &channel_id);

    Ok(NormalizedProviderProfile {
        wire_protocol,
        channel: channel_id,
        name: display_name,
        api_base,
        api_key_env: profile.api_key_env.clone(),
        inventory_family,
    })
}

fn display_name_for_profile(raw_name: &str, api_base: &str, channel_id: &str) -> String {
    let trimmed = raw_name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    name_from_base_url(api_base).unwrap_or_else(|| channel_id.to_string())
}

fn name_from_base_url(api_base: &str) -> Option<String> {
    let trimmed = api_base.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("local") {
        return None;
    }
    // Prefer host[:port] without depending on the url crate.
    let without_scheme = trimmed
        .split("://")
        .nth(1)
        .unwrap_or(trimmed)
        .split('/')
        .next()
        .unwrap_or(trimmed)
        .trim()
        .trim_end_matches('/');
    // Drop userinfo if present (user:pass@host).
    let host_port = without_scheme
        .rsplit('@')
        .next()
        .unwrap_or(without_scheme)
        .trim();
    if host_port.is_empty() {
        None
    } else {
        Some(host_port.to_string())
    }
}

/// User-facing type → (wire_protocol, channel_id, inventory family).
///
/// Types are product labels (`openai`, `anthropic`, …), not "official vs
/// relay". Official and gateway endpoints share the same type; only base URL,
/// key, and model differ. Gateways that speak Gemini's OpenAI-compatible API
/// are reached with the `openai` type.
fn resolve_channel_identity(channel: &str) -> Result<(String, String, InventoryFamily), ApiError> {
    match channel.trim().to_ascii_lowercase().as_str() {
        // Chat Completions wire: official OpenAI, relays, vLLM, DeepSeek, ZenMux, …
        "openai" => Ok((
            "openai-chat".to_string(),
            "openai".to_string(),
            InventoryFamily::OpenAiCompatible,
        )),
        "openai-responses" => Ok((
            "openai-responses".to_string(),
            "openai-responses".to_string(),
            InventoryFamily::OpenAiCompatible,
        )),
        "anthropic" => Ok((
            "anthropic-messages".to_string(),
            "anthropic".to_string(),
            InventoryFamily::Anthropic,
        )),
        "ollama" => Ok((
            "ollama-chat".to_string(),
            "ollama".to_string(),
            InventoryFamily::Ollama,
        )),
        "fake" => Ok((
            "fake".to_string(),
            "fake".to_string(),
            InventoryFamily::Fake,
        )),
        other => Err(ApiError::bad_request(format!(
            "unsupported provider type `{other}`; choose openai, openai-responses, anthropic, ollama, or fake"
        ))),
    }
}

fn resolve_protocol_identity(
    protocol: &str,
) -> Result<(String, String, InventoryFamily), ApiError> {
    let protocol = protocol.trim().to_ascii_lowercase();
    match protocol.as_str() {
        "openai-chat" => Ok((
            "openai-chat".to_string(),
            "openai".to_string(),
            InventoryFamily::OpenAiCompatible,
        )),
        "openai-responses" => Ok((
            "openai-responses".to_string(),
            "openai-responses".to_string(),
            InventoryFamily::OpenAiCompatible,
        )),
        "anthropic-messages" => Ok((
            "anthropic-messages".to_string(),
            "anthropic".to_string(),
            InventoryFamily::Anthropic,
        )),
        "ollama-chat" => Ok((
            "ollama-chat".to_string(),
            "ollama".to_string(),
            InventoryFamily::Ollama,
        )),
        "fake" => Ok((
            "fake".to_string(),
            "fake".to_string(),
            InventoryFamily::Fake,
        )),
        other => Err(ApiError::bad_request(format!(
            "unsupported provider wire_protocol `{other}`; supported built-ins: openai-chat, openai-responses, anthropic-messages, ollama-chat, fake"
        ))),
    }
}

pub(super) fn provider_key_env(profile: &NormalizedProviderProfile) -> String {
    let default = match profile.inventory_family {
        InventoryFamily::Anthropic => DEFAULT_ANTHROPIC_KEY_ENV,
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

fn provider_api_key(family: &InventoryFamily, key_env: &str) -> Result<String, ApiError> {
    if matches!(family, InventoryFamily::Ollama | InventoryFamily::Fake) {
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
    profile: &NormalizedProviderProfile,
    key_env: &str,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    match profile.inventory_family {
        InventoryFamily::OpenAiCompatible => {
            openai_compatible_inventory(profile, key_env, requested_endpoint).await
        }
        InventoryFamily::Anthropic => {
            anthropic_inventory(profile, key_env, requested_endpoint).await
        }
        InventoryFamily::Ollama => ollama_inventory(profile, requested_endpoint).await,
        InventoryFamily::Fake => Ok(ProviderInventory {
            key_present: false,
            models: vec!["fake".to_string(), "fake-raw".to_string()],
        }),
    }
}

async fn openai_compatible_inventory(
    profile: &NormalizedProviderProfile,
    key_env: &str,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let api_key = provider_api_key(&profile.inventory_family, key_env)?;
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
    profile: &NormalizedProviderProfile,
    key_env: &str,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let api_key = provider_api_key(&profile.inventory_family, key_env)?;
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
    profile: &NormalizedProviderProfile,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderProfileRequest;

    #[test]
    fn channel_maps_to_wire_protocol_and_name_defaults_from_base_url() {
        let profile = normalize_provider_profile(&ProviderProfileRequest {
            channel: Some("openai".to_string()),
            name: String::new(),
            wire_protocol: None,
            api_base: "https://relay.example.com/v1".to_string(),
            api_key_env: None,
        })
        .unwrap();
        assert_eq!(profile.wire_protocol, "openai-chat");
        assert_eq!(profile.channel, "openai");
        assert_eq!(profile.name, "relay.example.com");
    }

    #[test]
    fn custom_display_name_is_preserved() {
        let profile = normalize_provider_profile(&ProviderProfileRequest {
            channel: Some("anthropic".to_string()),
            name: "team-claude".to_string(),
            wire_protocol: None,
            api_base: "https://api.anthropic.com".to_string(),
            api_key_env: None,
        })
        .unwrap();
        assert_eq!(profile.wire_protocol, "anthropic-messages");
        assert_eq!(profile.name, "team-claude");
    }

    #[test]
    fn name_is_treated_as_label_not_a_type() {
        // A free-form name is a display label; it never selects the type.
        // Without an explicit channel/wire_protocol the request is rejected.
        let error = normalize_provider_profile(&ProviderProfileRequest {
            channel: None,
            name: "my gateway".to_string(),
            wire_protocol: None,
            api_base: "https://gateway.test/v1".to_string(),
            api_key_env: None,
        })
        .unwrap_err();
        assert!(
            error.message.contains("requires a type"),
            "unexpected error: {}",
            error.message
        );
    }

    #[test]
    fn gemini_relay_uses_openai_type() {
        // Gemini's OpenAI-compatible gateways are reached with the `openai` type;
        // there is no separate gemini type.
        let profile = normalize_provider_profile(&ProviderProfileRequest {
            channel: Some("openai".to_string()),
            name: String::new(),
            wire_protocol: None,
            api_base: "https://zenmux.ai/api/v1".to_string(),
            api_key_env: None,
        })
        .unwrap();
        assert_eq!(profile.wire_protocol, "openai-chat");
        assert_eq!(profile.channel, "openai");
    }

    #[test]
    fn unsupported_type_is_rejected() {
        let error = normalize_provider_profile(&ProviderProfileRequest {
            channel: Some("gemini-openai-compat".to_string()),
            name: String::new(),
            wire_protocol: None,
            api_base: "https://zenmux.ai/api/v1".to_string(),
            api_key_env: None,
        })
        .unwrap_err();
        assert!(
            error.message.contains("unsupported provider type"),
            "unexpected error: {}",
            error.message
        );
    }
}
