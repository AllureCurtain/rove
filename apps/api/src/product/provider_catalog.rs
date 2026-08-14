use std::collections::BTreeMap;

use chrono::{SecondsFormat, Utc};
use rove_app_bootstrap::{
    ProviderAuthConfig, ProviderCatalog, ProviderCatalogError, ProviderCatalogService,
    ProviderProfileConfig, ProviderProfileId, SecretSource,
};

use super::{
    CreateProductProviderProfileRequest, ProductErrorCode, ProductProviderProfile,
    ProductProviderProfileId, ProductProviderType, UpdateProductProviderProfileRequest,
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
    current.profile_config(&profile_id).map_err(catalog_error)?;
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
) -> Result<(ProviderProfileRequest, Option<String>, ProductProviderType), ApiError> {
    let profile_id = catalog_id(profile_id)?;
    let profile = catalog.profile_config(&profile_id).map_err(catalog_error)?;
    let provider_type = product_provider_type(&profile.provider_type)?;
    let api_key_env = env_credential(&profile.auth)?;
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
    ))
}

fn profile_from_parts(
    label: String,
    provider_type: ProductProviderType,
    api_base: String,
    api_key_env: Option<String>,
    default_model: Option<String>,
) -> Result<ProviderProfileConfig, ApiError> {
    let label = bounded_text("provider profile label", label)?;
    let model = bounded_text(
        "default model",
        default_model.unwrap_or_else(|| "default".to_string()),
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
        (_, None) => ProviderAuthConfig::None,
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
        headers: BTreeMap::new(),
        options: Default::default(),
        protocol_options: serde_json::json!({}),
    };
    profile
        .validate(std::path::Path::new("."), true)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(profile)
}

fn product_profile(
    catalog: &ProviderCatalog,
    profile_id: &ProviderProfileId,
) -> Result<ProductProviderProfile, ApiError> {
    let profile = catalog.profile_config(profile_id).map_err(catalog_error)?;
    let api_key_env = env_credential(&profile.auth).unwrap_or(None);
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
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
        default_model: Some(profile.model.clone()),
        created_at: now.clone(),
        updated_at: now,
        catalog_revision: catalog.revision().to_string(),
    })
}

fn env_credential(auth: &ProviderAuthConfig) -> Result<Option<String>, ApiError> {
    match auth {
        ProviderAuthConfig::Bearer {
            secret: SecretSource::Env { env },
        }
        | ProviderAuthConfig::Header {
            secret: SecretSource::Env { env },
            ..
        } => Ok(Some(env.clone())),
        ProviderAuthConfig::None => Ok(None),
        _ => Err(invalid(
            "this API compatibility view supports env credential references only",
        )),
    }
}

pub(crate) fn catalog_id(id: &ProductProviderProfileId) -> Result<ProviderProfileId, ApiError> {
    ProviderProfileId::new(id.to_string()).map_err(catalog_error)
}

fn product_provider_type(value: &str) -> Result<ProductProviderType, ApiError> {
    match value {
        "openai" => Ok(ProductProviderType::Openai),
        "openai-responses" => Ok(ProductProviderType::OpenaiResponses),
        "anthropic" => Ok(ProductProviderType::Anthropic),
        "ollama" => Ok(ProductProviderType::Ollama),
        "fake" => Ok(ProductProviderType::Fake),
        _ => Err(invalid("unsupported provider type")),
    }
}

fn provider_type_name(value: ProductProviderType) -> &'static str {
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
