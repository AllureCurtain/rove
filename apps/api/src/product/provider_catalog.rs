use std::collections::BTreeMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::header::HeaderMap;
use rove_app_bootstrap::{
    ProviderAuthConfig, ProviderCatalog, ProviderCatalogError, ProviderCatalogService,
    ProviderProfileConfig, ProviderProfileId, SecretSource,
};

use super::{
    CreateProductProviderProfileRequest, ProductErrorCode, ProductProviderCredentialSource,
    ProductProviderProfile, ProductProviderProfileId, ProductProviderType,
    UpdateProductProviderProfileRequest,
};
use crate::{ApiError, ProviderProfileRequest};

pub(crate) fn list(catalog: &ProviderCatalog) -> Result<Vec<ProductProviderProfile>, ApiError> {
    catalog
        .profiles()
        .into_iter()
        .map(|profile| product_profile(catalog, &profile.id))
        .collect()
}

pub(crate) fn get(
    catalog: &ProviderCatalog,
    profile_id: &ProductProviderProfileId,
) -> Result<ProductProviderProfile, ApiError> {
    product_profile(catalog, &catalog_id(profile_id)?)
}

pub(crate) fn create(
    service: &ProviderCatalogService,
    request: CreateProductProviderProfileRequest,
) -> Result<ProductProviderProfile, ApiError> {
    let id = ProductProviderProfileId::new();
    let profile_id = catalog_id(&id)?;
    let current = service.load().map_err(catalog_error)?;
    if current.profiles().len() >= super::MAX_PRODUCT_PROVIDER_PROFILES {
        return Err(invalid("provider profile limit reached"));
    }
    let expected = request
        .expected_revision
        .as_deref()
        .unwrap_or(current.revision());
    let profile = profile_from_parts(
        request.label,
        request.provider_type,
        request.api_base,
        request.api_key_env,
        request.default_model,
        None,
        &service.paths().root,
    )?;
    let updated = service
        .upsert_profile(expected, profile_id.clone(), profile)
        .map_err(catalog_error)?;
    product_profile(&updated, &profile_id)
}

pub(crate) fn update(
    service: &ProviderCatalogService,
    profile_id: &ProductProviderProfileId,
    request: UpdateProductProviderProfileRequest,
) -> Result<ProductProviderProfile, ApiError> {
    let profile_id = catalog_id(profile_id)?;
    let current = service.load().map_err(catalog_error)?;
    let existing = current
        .profile_config(&profile_id)
        .map_err(catalog_error)?
        .clone();
    let expected = request
        .expected_revision
        .as_deref()
        .unwrap_or(current.revision());
    let profile = profile_from_parts(
        request.label,
        request.provider_type,
        request.api_base,
        request.api_key_env,
        request.default_model,
        Some(&existing),
        &service.paths().root,
    )?;
    let updated = service
        .upsert_profile(expected, profile_id.clone(), profile)
        .map_err(catalog_error)?;
    product_profile(&updated, &profile_id)
}

pub(crate) fn delete(
    service: &ProviderCatalogService,
    profile_id: &ProductProviderProfileId,
    expected_revision: Option<&str>,
) -> Result<(), ApiError> {
    let profile_id = catalog_id(profile_id)?;
    let current = service.load().map_err(catalog_error)?;
    let expected = expected_revision.unwrap_or(current.revision());
    service
        .delete_profile(expected, &profile_id)
        .map_err(catalog_error)?;
    Ok(())
}

pub(crate) fn inventory_request(
    catalog: &ProviderCatalog,
    profile_id: &ProductProviderProfileId,
    credential_root: &Path,
) -> Result<
    (
        ProviderProfileRequest,
        Option<String>,
        ProductProviderType,
        HeaderMap,
    ),
    ApiError,
> {
    let profile_id = catalog_id(profile_id)?;
    let profile = catalog.profile_config(&profile_id).map_err(catalog_error)?;
    let provider_type = product_provider_type(&profile.provider_type)?;
    let api_key_env = env_credential(&profile.auth);
    let headers = profile
        .resolve_http_headers(credential_root, true, &BTreeMap::new())
        .map_err(|error| invalid(format!("provider credential is unavailable: {error}")))?;
    Ok((
        ProviderProfileRequest {
            provider_type: Some(profile.provider_type.clone()),
            name: profile
                .label
                .clone()
                .unwrap_or_else(|| profile_id.to_string()),
            api_base: profile.base_url.clone(),
            api_key_env,
        },
        Some(profile.model.clone()),
        provider_type,
        headers,
    ))
}

fn profile_from_parts(
    label: String,
    provider_type: ProductProviderType,
    api_base: String,
    api_key_env: Option<String>,
    default_model: Option<String>,
    existing: Option<&ProviderProfileConfig>,
    credential_root: &Path,
) -> Result<ProviderProfileConfig, ApiError> {
    let label = bounded_text("provider profile label", label)?;
    let model = bounded_text(
        "default model",
        default_model
            .or_else(|| existing.map(|profile| profile.model.clone()))
            .unwrap_or_else(|| "default".to_string()),
    )?;
    let auth = match (provider_type, api_key_env) {
        (ProductProviderType::Fake | ProductProviderType::Ollama, None) => ProviderAuthConfig::None,
        (ProductProviderType::Fake | ProductProviderType::Ollama, Some(_)) => {
            return Err(invalid(
                "local provider profiles cannot reference an API key",
            ));
        }
        (ProductProviderType::Anthropic, Some(env)) => ProviderAuthConfig::Header {
            header: "x-api-key".to_string(),
            secret: SecretSource::Env {
                env: validate_env(env)?,
            },
        },
        (_, Some(env)) => ProviderAuthConfig::Bearer {
            secret: SecretSource::Env {
                env: validate_env(env)?,
            },
        },
        (_, None) => existing
            .map(|profile| profile.auth.clone())
            .unwrap_or(ProviderAuthConfig::None),
    };
    let profile = ProviderProfileConfig {
        label: Some(label),
        provider_type: provider_type_name(provider_type).to_string(),
        base_url: if provider_type == ProductProviderType::Fake {
            String::new()
        } else {
            api_base.trim().trim_end_matches('/').to_string()
        },
        model,
        auth,
        headers: existing
            .map(|profile| profile.headers.clone())
            .unwrap_or_default(),
        options: existing.map(|profile| profile.options).unwrap_or_default(),
        protocol_options: existing
            .map(|profile| profile.protocol_options.clone())
            .unwrap_or_else(|| serde_json::json!({})),
    };
    profile
        .validate(credential_root, true)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(profile)
}

fn product_profile(
    catalog: &ProviderCatalog,
    profile_id: &ProviderProfileId,
) -> Result<ProductProviderProfile, ApiError> {
    let profile = catalog.profile_config(profile_id).map_err(catalog_error)?;
    let credential_source = product_credential_source(&profile.auth);
    let api_key_env = env_credential(&profile.auth);
    let modified_at = DateTime::<Utc>::from(catalog.modified_at().unwrap_or(UNIX_EPOCH))
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    Ok(ProductProviderProfile {
        id: ProductProviderProfileId::from_catalog_id(profile_id.to_string())
            .map_err(ApiError::bad_request)?,
        label: profile
            .label
            .clone()
            .unwrap_or_else(|| profile_id.to_string()),
        provider_type: product_provider_type(&profile.provider_type)?,
        api_base: profile.base_url.clone(),
        api_key_env,
        credential_source,
        default_model: Some(profile.model.clone()),
        created_at: modified_at.clone(),
        updated_at: modified_at,
        catalog_revision: catalog.revision().to_string(),
    })
}

fn env_credential(auth: &ProviderAuthConfig) -> Option<String> {
    match auth {
        ProviderAuthConfig::Bearer {
            secret: SecretSource::Env { env },
        }
        | ProviderAuthConfig::Header {
            secret: SecretSource::Env { env },
            ..
        } => Some(env.clone()),
        _ => None,
    }
}

fn product_credential_source(auth: &ProviderAuthConfig) -> ProductProviderCredentialSource {
    match auth {
        ProviderAuthConfig::None => ProductProviderCredentialSource::None,
        ProviderAuthConfig::Bearer { secret } | ProviderAuthConfig::Header { secret, .. } => {
            match secret {
                SecretSource::Env { env } => {
                    ProductProviderCredentialSource::Env { name: env.clone() }
                }
                SecretSource::File { file } => ProductProviderCredentialSource::File {
                    path: file.display().to_string(),
                },
                SecretSource::Keyring { keyring } => ProductProviderCredentialSource::Keyring {
                    service: keyring.service.clone(),
                    account: keyring.account.clone(),
                },
                SecretSource::Literal(_) => ProductProviderCredentialSource::None,
            }
        }
    }
}

pub(crate) fn catalog_id(id: &ProductProviderProfileId) -> Result<ProviderProfileId, ApiError> {
    ProviderProfileId::new(id.to_string()).map_err(catalog_error)
}

pub(crate) fn product_provider_type(value: &str) -> Result<ProductProviderType, ApiError> {
    match value {
        "openai" => Ok(ProductProviderType::Openai),
        "openai-responses" => Ok(ProductProviderType::OpenaiResponses),
        "anthropic" => Ok(ProductProviderType::Anthropic),
        "ollama" => Ok(ProductProviderType::Ollama),
        "fake" => Ok(ProductProviderType::Fake),
        _ => Err(invalid("unsupported provider type")),
    }
}

pub(crate) fn provider_type_name(value: ProductProviderType) -> &'static str {
    match value {
        ProductProviderType::Openai => "openai",
        ProductProviderType::OpenaiResponses => "openai-responses",
        ProductProviderType::Anthropic => "anthropic",
        ProductProviderType::Ollama => "ollama",
        ProductProviderType::Fake => "fake",
    }
}

fn validate_env(value: String) -> Result<String, ApiError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 256
        || (!bytes[0].is_ascii_uppercase() && bytes[0] != b'_')
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        return Err(invalid("provider credential environment name is invalid"));
    }
    Ok(value)
}

fn bounded_text(field: &str, value: String) -> Result<String, ApiError> {
    let value = value.trim().to_string();
    if value.is_empty()
        || value.len() > super::MAX_PRODUCT_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(invalid(format!("{field} is empty, too long, or invalid")));
    }
    Ok(value)
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::bad_request_with_code(ProductErrorCode::ProductInvalidInput.as_str(), message)
}

pub(crate) fn catalog_error(error: ProviderCatalogError) -> ApiError {
    match error {
        ProviderCatalogError::ProfileNotFound(_) => ApiError::not_found_with_code(
            ProductErrorCode::ProductProviderProfileUnavailable.as_str(),
            error.to_string(),
        ),
        ProviderCatalogError::RevisionConflict | ProviderCatalogError::Busy => {
            ApiError::conflict_with_code(
                ProductErrorCode::ProductRevisionConflict.as_str(),
                error.to_string(),
            )
        }
        _ => invalid(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_app_bootstrap::{
        KeyringReference, ProviderHeaderValue, UserConfigDocument, UserConfigPaths,
        UserConfigWriter,
    };
    use rove_models::ProviderOptions;

    fn configured_profile(auth: ProviderAuthConfig) -> ProviderProfileConfig {
        ProviderProfileConfig {
            label: Some("Original".to_string()),
            provider_type: "openai".to_string(),
            base_url: "https://gateway.example.test/v1".to_string(),
            model: "original-model".to_string(),
            auth,
            headers: BTreeMap::from([(
                "x-tenant".to_string(),
                ProviderHeaderValue::File {
                    file: "credentials/tenant.txt".into(),
                },
            )]),
            options: ProviderOptions {
                max_tokens: Some(4_096),
                temperature: Some(0.2),
                ..ProviderOptions::default()
            },
            protocol_options: serde_json::json!({"prompt_cache_enabled": true}),
        }
    }

    #[test]
    fn api_catalog_round_trip_preserves_unrepresented_profile_configuration() {
        let temp = tempfile::TempDir::new().unwrap();
        let paths = UserConfigPaths::from_root(temp.path().join("user"));
        let service = ProviderCatalogService::new(paths.clone());
        let mut document = UserConfigDocument::default();
        document.provider.profiles.insert(
            "file-profile".to_string(),
            configured_profile(ProviderAuthConfig::Bearer {
                secret: SecretSource::File {
                    file: "credentials/provider.key".into(),
                },
            }),
        );
        document.provider.profiles.insert(
            "keyring-profile".to_string(),
            configured_profile(ProviderAuthConfig::Header {
                header: "x-api-key".to_string(),
                secret: SecretSource::Keyring {
                    keyring: KeyringReference {
                        service: "rove.test".to_string(),
                        account: "integration".to_string(),
                    },
                },
            }),
        );
        UserConfigWriter::new(paths)
            .update(None, &document)
            .unwrap();

        let first = service.load().unwrap();
        let first_profiles = list(&first).unwrap();
        assert!(matches!(
            first_profiles
                .iter()
                .find(|profile| profile.id.as_str() == "file-profile")
                .unwrap()
                .credential_source,
            ProductProviderCredentialSource::File { .. }
        ));
        assert!(matches!(
            first_profiles
                .iter()
                .find(|profile| profile.id.as_str() == "keyring-profile")
                .unwrap()
                .credential_source,
            ProductProviderCredentialSource::Keyring { .. }
        ));
        let stable_timestamp = first_profiles[0].updated_at.clone();
        assert_eq!(
            list(&service.load().unwrap()).unwrap()[0].updated_at,
            stable_timestamp
        );

        let file_id = ProductProviderProfileId::from_catalog_id("file-profile").unwrap();
        let updated = update(
            &service,
            &file_id,
            UpdateProductProviderProfileRequest {
                label: "Renamed".to_string(),
                provider_type: ProductProviderType::Openai,
                api_base: "https://gateway.example.test/v2".to_string(),
                api_key_env: None,
                default_model: Some("updated-model".to_string()),
                expected_revision: Some(first.revision().to_string()),
            },
        )
        .unwrap();
        assert!(matches!(
            updated.credential_source,
            ProductProviderCredentialSource::File { .. }
        ));

        let keyring_id = ProductProviderProfileId::from_catalog_id("keyring-profile").unwrap();
        let after_file_update = service.load().unwrap();
        let keyring_updated = update(
            &service,
            &keyring_id,
            UpdateProductProviderProfileRequest {
                label: "Keyring renamed".to_string(),
                provider_type: ProductProviderType::Openai,
                api_base: "https://gateway.example.test/v1".to_string(),
                api_key_env: None,
                default_model: None,
                expected_revision: Some(after_file_update.revision().to_string()),
            },
        )
        .unwrap();
        assert!(matches!(
            keyring_updated.credential_source,
            ProductProviderCredentialSource::Keyring { .. }
        ));

        let reloaded = service.load().unwrap();
        let preserved = reloaded
            .profile_config(&ProviderProfileId::new("file-profile").unwrap())
            .unwrap();
        assert!(matches!(
            preserved.auth,
            ProviderAuthConfig::Bearer {
                secret: SecretSource::File { .. }
            }
        ));
        assert!(matches!(
            preserved.headers.get("x-tenant"),
            Some(ProviderHeaderValue::File { .. })
        ));
        assert_eq!(preserved.options.max_tokens, Some(4_096));
        assert_eq!(preserved.options.temperature, Some(0.2));
        assert_eq!(preserved.protocol_options["prompt_cache_enabled"], true);
        let keyring_preserved = reloaded
            .profile_config(&ProviderProfileId::new("keyring-profile").unwrap())
            .unwrap();
        assert!(matches!(
            keyring_preserved.auth,
            ProviderAuthConfig::Header {
                secret: SecretSource::Keyring { .. },
                ..
            }
        ));
        assert_eq!(keyring_preserved.options.max_tokens, Some(4_096));
        assert_eq!(
            keyring_preserved.protocol_options["prompt_cache_enabled"],
            true
        );
    }
}
