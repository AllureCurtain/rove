//! Provider profile normalization and model inventory.
//!
//! User-facing profiles select a **type** (`openai`, `openai-responses`,
//! `anthropic`, `ollama`, `fake`). The type maps to an internal
//! [`WireProtocolId`]. Display `name` is optional and defaults from `api_base`
//! when omitted. Clients must not supply `wire_protocol`; responses may echo it
//! as a read-only diagnostic.

use std::collections::BTreeMap;
use std::time::Duration;

use futures::StreamExt;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use rove_app_bootstrap::AppConfig;
use rove_app_bootstrap::provider::{ProviderAuthConfig, ProviderProfileConfig, SecretSource};
use rove_models::provider::WireProtocolId;

use super::{ApiError, ProviderProfileRequest};

const DEFAULT_PROVIDER_KEY_ENV: &str = "OPENAI_API_KEY";
const DEFAULT_ANTHROPIC_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const JOB_PROVIDER_PROFILE: &str = "__api_request__";
const PROVIDER_INVENTORY_TIMEOUT_MS: u64 = 5_000;
const PROVIDER_INVENTORY_CONNECT_TIMEOUT_MS: u64 = 2_000;
const MAX_PROVIDER_INVENTORY_ENDPOINT_BYTES: usize = 2_048;
const MAX_PROVIDER_INVENTORY_RESPONSE_BYTES: usize = 1_048_576;

/// Normalized provider identity used by job assembly and inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NormalizedProviderProfile {
    /// Stable open wire protocol id (`openai-completions`, `anthropic-messages`, …).
    /// System-mapped from `provider_type`; never accepted from request bodies.
    pub(super) wire_protocol: String,
    /// User-facing provider type (`openai`, `anthropic`, …).
    pub(super) provider_type: String,
    /// Display name (custom label or derived from `api_base`).
    pub(super) name: String,
    pub(super) api_base: String,
    pub(super) api_key_env: Option<String>,
    pub(super) inventory_family: InventoryFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InventoryFamily {
    OpenAi,
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

    // Validate the mapped wire id early so bad configs fail before job start.
    WireProtocolId::new(profile.wire_protocol.clone()).map_err(|error| {
        ApiError::bad_request(format!("provider wire_protocol is invalid: {error}"))
    })?;
    let key_env = provider_key_env(&profile);
    // Resolve once at request time so missing secrets fail before the job
    // starts, and so request-scoped env vars can be cleared by callers/tests
    // before the async job builds its model client.
    let api_key = provider_api_key(&profile.inventory_family, &key_env)?;
    let auth = match profile.inventory_family {
        InventoryFamily::OpenAi => {
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
            label: Some(profile.name.clone()),
            provider_type: profile.provider_type.clone(),
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
    config.provider.model = model;
    Ok(())
}

pub(super) fn normalize_provider_profile(
    profile: &ProviderProfileRequest,
) -> Result<NormalizedProviderProfile, ApiError> {
    let provider_type = profile
        .provider_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| {
            ApiError::bad_request(
                "provider profile requires provider_type (openai, openai-responses, anthropic, ollama, or fake)",
            )
        })?;
    let raw_name = profile.name.trim();

    let (wire_protocol, provider_type_id, inventory_family) =
        resolve_provider_type_identity(&provider_type)?;

    let api_base = profile.api_base.trim().trim_end_matches('/').to_string();
    match inventory_family {
        InventoryFamily::OpenAi | InventoryFamily::Anthropic => {
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

    let display_name = display_name_for_profile(raw_name, &api_base, &provider_type_id);

    Ok(NormalizedProviderProfile {
        wire_protocol,
        provider_type: provider_type_id,
        name: display_name,
        api_base,
        api_key_env: profile.api_key_env.clone(),
        inventory_family,
    })
}

fn display_name_for_profile(raw_name: &str, api_base: &str, provider_type: &str) -> String {
    let trimmed = raw_name.trim();
    if !trimmed.is_empty() {
        return trimmed.to_string();
    }
    name_from_base_url(api_base).unwrap_or_else(|| provider_type.to_string())
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

/// User-facing type → (wire_protocol, provider_type, inventory family).
///
/// Types are product labels (`openai`, `anthropic`, …), not "official vs
/// relay". Official and gateway endpoints share the same type; only base URL,
/// key, and model differ. Gateways that speak OpenAI Chat Completions are
/// reached with the `openai` type.
fn resolve_provider_type_identity(
    provider_type: &str,
) -> Result<(String, String, InventoryFamily), ApiError> {
    match provider_type.trim().to_ascii_lowercase().as_str() {
        // Chat Completions wire: official OpenAI, relays, vLLM, DeepSeek, ZenMux, …
        "openai" => Ok((
            "openai-completions".to_string(),
            "openai".to_string(),
            InventoryFamily::OpenAi,
        )),
        "openai-responses" => Ok((
            "openai-responses".to_string(),
            "openai-responses".to_string(),
            InventoryFamily::OpenAi,
        )),
        "anthropic" => Ok((
            "anthropic-messages".to_string(),
            "anthropic".to_string(),
            InventoryFamily::Anthropic,
        )),
        "ollama" => Ok((
            "ollama".to_string(),
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
    let mut headers = HeaderMap::new();
    match profile.inventory_family {
        InventoryFamily::OpenAi => {
            let api_key = provider_api_key(&profile.inventory_family, key_env)?;
            headers.insert(
                AUTHORIZATION,
                sensitive_provider_header(format!("Bearer {api_key}"))?,
            );
        }
        InventoryFamily::Anthropic => {
            let api_key = provider_api_key(&profile.inventory_family, key_env)?;
            headers.insert(
                reqwest::header::HeaderName::from_static("x-api-key"),
                sensitive_provider_header(api_key)?,
            );
        }
        InventoryFamily::Ollama | InventoryFamily::Fake => {}
    }
    provider_inventory_with_headers(profile, headers, true, requested_endpoint).await
}

fn sensitive_provider_header(value: String) -> Result<HeaderValue, ApiError> {
    let mut value = HeaderValue::try_from(value)
        .map_err(|_| ApiError::bad_request("provider credential is invalid"))?;
    value.set_sensitive(true);
    Ok(value)
}

pub(super) async fn provider_inventory_with_headers(
    profile: &NormalizedProviderProfile,
    headers: HeaderMap,
    key_present: bool,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    match profile.inventory_family {
        InventoryFamily::OpenAi => {
            openai_inventory(profile, headers, key_present, requested_endpoint).await
        }
        InventoryFamily::Anthropic => {
            anthropic_inventory(profile, headers, key_present, requested_endpoint).await
        }
        InventoryFamily::Ollama => ollama_inventory(profile, headers, requested_endpoint).await,
        InventoryFamily::Fake => Ok(ProviderInventory {
            key_present: false,
            models: vec!["fake".to_string(), "fake-raw".to_string()],
        }),
    }
}

async fn openai_inventory(
    profile: &NormalizedProviderProfile,
    headers: HeaderMap,
    key_present: bool,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let endpoint = inventory_endpoint(
        requested_endpoint,
        &format!("{}/models", profile.api_base.trim_end_matches('/')),
    )?;
    let response = provider_inventory_client()?
        .get(&endpoint)
        .headers(headers)
        .send()
        .await
        .map_err(classify_inventory_transport_error)?;
    model_inventory_response(response, "data", "id", key_present).await
}

async fn anthropic_inventory(
    profile: &NormalizedProviderProfile,
    mut headers: HeaderMap,
    key_present: bool,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let endpoint = inventory_endpoint(
        requested_endpoint,
        &format!("{}/v1/models", profile.api_base.trim_end_matches('/')),
    )?;
    headers
        .entry(reqwest::header::HeaderName::from_static(
            "anthropic-version",
        ))
        .or_insert(HeaderValue::from_static("2023-06-01"));
    let response = provider_inventory_client()?
        .get(&endpoint)
        .headers(headers)
        .send()
        .await
        .map_err(classify_inventory_transport_error)?;
    model_inventory_response(response, "data", "id", key_present).await
}

async fn ollama_inventory(
    profile: &NormalizedProviderProfile,
    headers: HeaderMap,
    requested_endpoint: Option<&str>,
) -> Result<ProviderInventory, ApiError> {
    let endpoint = inventory_endpoint(
        requested_endpoint,
        &format!("{}/api/tags", profile.api_base.trim_end_matches('/')),
    )?;
    let response = provider_inventory_client()?
        .get(&endpoint)
        .headers(headers)
        .send()
        .await
        .map_err(classify_inventory_transport_error)?;
    model_inventory_response(response, "models", "name", false).await
}

fn provider_inventory_client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(PROVIDER_INVENTORY_CONNECT_TIMEOUT_MS))
        .timeout(Duration::from_millis(PROVIDER_INVENTORY_TIMEOUT_MS))
        .build()
        .map_err(|_| {
            ApiError::bad_gateway_with_code(
                "provider_transport",
                "provider inventory transport could not be initialized",
            )
        })
}

fn inventory_endpoint(
    requested_endpoint: Option<&str>,
    default_endpoint: &str,
) -> Result<String, ApiError> {
    let endpoint = requested_endpoint
        .filter(|endpoint| !endpoint.trim().is_empty())
        .unwrap_or(default_endpoint)
        .trim();
    if endpoint.len() > MAX_PROVIDER_INVENTORY_ENDPOINT_BYTES {
        return Err(ApiError::bad_request(
            "provider models endpoint is too long",
        ));
    }
    let parsed = reqwest::Url::parse(endpoint)
        .map_err(|_| ApiError::bad_request("provider models endpoint is invalid"))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "provider models endpoint must be an HTTP URL without credentials, query, or fragment",
        ));
    }
    Ok(endpoint.to_string())
}

fn classify_inventory_transport_error(error: reqwest::Error) -> ApiError {
    if error.is_timeout() {
        ApiError::gateway_timeout_with_code(
            "provider_timeout",
            format!("provider model inventory timed out after {PROVIDER_INVENTORY_TIMEOUT_MS}ms"),
        )
    } else {
        ApiError::bad_gateway_with_code(
            "provider_transport",
            "provider model inventory transport failed",
        )
    }
}

async fn model_inventory_response(
    response: reqwest::Response,
    array_field: &str,
    id_field: &str,
    key_present: bool,
) -> Result<ProviderInventory, ApiError> {
    if !response.status().is_success() {
        let status = response.status();
        let code = match status.as_u16() {
            401 | 403 => "provider_authentication",
            429 => "provider_rate_limited",
            400..=499 => "provider_protocol_mismatch",
            _ => "provider_transport",
        };
        let message = match code {
            "provider_authentication" => {
                format!("provider rejected the configured credentials (HTTP {status})")
            }
            "provider_rate_limited" => {
                format!("provider rate limited the inventory request (HTTP {status})")
            }
            "provider_protocol_mismatch" => {
                format!(
                    "provider inventory endpoint rejected the expected protocol (HTTP {status})"
                )
            }
            _ => {
                format!("provider inventory endpoint returned an upstream failure (HTTP {status})")
            }
        };
        return Err(match code {
            "provider_rate_limited" => ApiError::too_many_requests_with_code(code, message),
            "provider_protocol_mismatch" | "provider_authentication" | "provider_transport" => {
                ApiError::bad_gateway_with_code(code, message)
            }
            _ => ApiError::bad_gateway(message),
        });
    }
    let body = read_inventory_json(response).await?;
    let values = body
        .get(array_field)
        .and_then(|value| value.as_array())
        .ok_or_else(|| {
            ApiError::bad_gateway_with_code(
                "provider_protocol_mismatch",
                "provider inventory response did not contain the expected model list",
            )
        })?;
    let models: Vec<String> = values
        .iter()
        .filter_map(|item| {
            item.get(id_field)
                .and_then(|id| id.as_str())
                .filter(|id| !id.trim().is_empty())
                .map(str::to_string)
        })
        .collect();
    if models.is_empty() {
        return Err(ApiError::bad_gateway_with_code(
            "provider_no_models",
            "provider inventory returned no usable models",
        ));
    }
    Ok(ProviderInventory {
        key_present,
        models,
    })
}

async fn read_inventory_json(response: reqwest::Response) -> Result<serde_json::Value, ApiError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_PROVIDER_INVENTORY_RESPONSE_BYTES as u64)
    {
        return Err(ApiError::bad_gateway_with_code(
            "provider_protocol_mismatch",
            "provider inventory response exceeded the 1 MiB limit",
        ));
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(classify_inventory_transport_error)?;
        if bytes.len().saturating_add(chunk.len()) > MAX_PROVIDER_INVENTORY_RESPONSE_BYTES {
            return Err(ApiError::bad_gateway_with_code(
                "provider_protocol_mismatch",
                "provider inventory response exceeded the 1 MiB limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::bad_gateway_with_code(
            "provider_protocol_mismatch",
            "provider inventory response was not valid JSON",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderProfileRequest;

    #[test]
    fn provider_type_maps_to_wire_protocol_and_name_defaults_from_base_url() {
        let profile = normalize_provider_profile(&ProviderProfileRequest {
            provider_type: Some("openai".to_string()),
            name: String::new(),
            api_base: "https://relay.example.com/v1".to_string(),
            api_key_env: None,
        })
        .unwrap();
        assert_eq!(profile.wire_protocol, "openai-completions");
        assert_eq!(profile.provider_type, "openai");
        assert_eq!(profile.name, "relay.example.com");
    }

    #[test]
    fn custom_display_name_is_preserved() {
        let profile = normalize_provider_profile(&ProviderProfileRequest {
            provider_type: Some("anthropic".to_string()),
            name: "team-claude".to_string(),
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
        // Without an explicit provider_type the request is rejected.
        let error = normalize_provider_profile(&ProviderProfileRequest {
            provider_type: None,
            name: "my gateway".to_string(),
            api_base: "https://gateway.test/v1".to_string(),
            api_key_env: None,
        })
        .unwrap_err();
        assert!(
            error.message.contains("requires provider_type"),
            "unexpected error: {}",
            error.message
        );
    }

    #[test]
    fn gemini_relay_uses_openai_type() {
        // Gemini OpenAI Chat Completions gateways are reached with the `openai` type;
        // there is no separate gemini type.
        let profile = normalize_provider_profile(&ProviderProfileRequest {
            provider_type: Some("openai".to_string()),
            name: String::new(),
            api_base: "https://zenmux.ai/api/v1".to_string(),
            api_key_env: None,
        })
        .unwrap();
        assert_eq!(profile.wire_protocol, "openai-completions");
        assert_eq!(profile.provider_type, "openai");
    }

    #[test]
    fn unsupported_type_is_rejected() {
        let error = normalize_provider_profile(&ProviderProfileRequest {
            provider_type: Some("gemini-openai-compat".to_string()),
            name: String::new(),
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

    #[test]
    fn inventory_credentials_are_marked_sensitive() {
        let header = sensitive_provider_header("Bearer provider-secret".to_string()).unwrap();

        assert!(header.is_sensitive());
        assert_eq!(header, "Bearer provider-secret");
    }
}
