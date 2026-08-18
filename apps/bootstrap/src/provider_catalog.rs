//! Shared, UI-neutral Provider catalog and selection contracts.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::provider::{KeyringReference, ProviderAuthConfig, ProviderProfileConfig, SecretSource};
use crate::user_config::{UserConfigDocument, UserConfigLoader, UserConfigPaths, UserConfigWriter};

const PROVIDER_KEYRING_SERVICE: &str = "rove.provider";
const MAX_INVENTORY_BYTES: usize = 2 * 1024 * 1024;
const PROVIDER_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderProfileId(pub String);

impl ProviderProfileId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProviderCatalogError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        {
            return Err(ProviderCatalogError::InvalidProfileId(value));
        }
        Ok(Self(value))
    }
}

impl std::fmt::Display for ProviderProfileId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: ProviderProfileId,
    pub label: String,
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub auth_source: CredentialReference,
    pub fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum CredentialReference {
    Env { name: String },
    File { path: String },
    Keyring { service: String, account: String },
    None,
}

impl CredentialReference {
    pub fn ready(&self, workspace: &Path) -> Result<bool, ProviderCatalogError> {
        match self {
            Self::Env { name } => match std::env::var(name) {
                Ok(value) if !value.trim().is_empty() && value.len() <= 16 * 1024 => Ok(true),
                Ok(_) => Err(ProviderCatalogError::CredentialUnavailable {
                    profile: "environment".to_string(),
                    reason: "credential is empty or exceeds the size limit".to_string(),
                }),
                Err(_) => Ok(false),
            },
            Self::File { path } => {
                let path = if Path::new(path).is_absolute() {
                    Path::new(path).to_path_buf()
                } else {
                    workspace.join(path)
                };
                match std::fs::metadata(path) {
                    Ok(metadata) if metadata.is_file() && metadata.len() <= 16 * 1024 => Ok(true),
                    Ok(_) => Err(ProviderCatalogError::CredentialUnavailable {
                        profile: "file".to_string(),
                        reason: "credential is not a bounded regular file".to_string(),
                    }),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                    Err(_) => Err(ProviderCatalogError::CredentialUnavailable {
                        profile: "file".to_string(),
                        reason: "credential metadata is unavailable".to_string(),
                    }),
                }
            }
            Self::Keyring { service, account } => {
                match keyring::Entry::new(service, account).and_then(|entry| entry.get_password()) {
                    Ok(value) if !value.trim().is_empty() && value.len() <= 16 * 1024 => Ok(true),
                    Ok(_) => Err(ProviderCatalogError::CredentialUnavailable {
                        profile: "keyring".to_string(),
                        reason: "credential is empty or exceeds the size limit".to_string(),
                    }),
                    Err(keyring::Error::NoEntry) => Ok(false),
                    Err(_) => Err(ProviderCatalogError::CredentialUnavailable {
                        profile: "keyring".to_string(),
                        reason: "credential lookup failed".to_string(),
                    }),
                }
            }
            Self::None => Ok(true),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelDescriptor {
    pub id: String,
    pub profile_id: ProviderProfileId,
    pub provider_type: String,
    pub supports_reasoning: bool,
    pub inventory_fresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelSelection {
    pub profile_id: ProviderProfileId,
    pub model: String,
    pub reasoning: String,
    pub revision: String,
}

pub type SelectionRevision = String;

#[derive(Clone)]
pub enum OnboardingCredential {
    Secret(String),
    Reference(SecretSource),
    None,
}

impl fmt::Debug for OnboardingCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Secret(_) => formatter.write_str("Secret([REDACTED])"),
            Self::Reference(reference) => {
                formatter.debug_tuple("Reference").field(reference).finish()
            }
            Self::None => formatter.write_str("None"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderOnboardingRequest {
    pub profile_id: ProviderProfileId,
    pub label: String,
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub credential: OnboardingCredential,
    pub make_default: bool,
    pub expected_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderProbeReceipt {
    pub profile_id: ProviderProfileId,
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub inventory_count: usize,
    pub streaming_supported: bool,
    pub native_tool_calls_supported: bool,
    pub usage_supported: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderOnboardingReceipt {
    pub profile_id: ProviderProfileId,
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    pub catalog_revision: String,
    pub credential: CredentialReference,
    pub probe: ProviderProbeReceipt,
    pub selected: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProbeFailureKind {
    Unauthorized,
    RateLimited,
    Upstream,
    Timeout,
    Transport,
    InvalidResponse,
    ModelUnavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderOnboardingError {
    #[error("provider onboarding input is invalid: {0}")]
    Invalid(String),
    #[error("provider credential storage is unavailable")]
    CredentialStore,
    #[error("provider probe failed: {kind:?}")]
    Probe { kind: ProviderProbeFailureKind },
    #[error("provider catalog changed during onboarding")]
    RevisionConflict,
    #[error("provider catalog publication failed: {0}")]
    Catalog(String),
    #[error("provider onboarding publication requires reconciliation")]
    ReconciliationRequired,
}

#[async_trait]
pub trait ProviderCredentialStore: Send + Sync {
    async fn put(&self, service: &str, account: &str, secret: &str) -> Result<(), ()>;
    async fn delete(&self, service: &str, account: &str) -> Result<(), ()>;
}

#[derive(Debug, Default)]
pub struct OsProviderCredentialStore;

#[async_trait]
impl ProviderCredentialStore for OsProviderCredentialStore {
    async fn put(&self, service: &str, account: &str, secret: &str) -> Result<(), ()> {
        let service = service.to_string();
        let account = account.to_string();
        let secret = secret.to_string();
        tokio::task::spawn_blocking(move || {
            keyring::Entry::new(&service, &account)
                .and_then(|entry| entry.set_password(&secret))
                .map_err(|_| ())
        })
        .await
        .map_err(|_| ())?
    }

    async fn delete(&self, service: &str, account: &str) -> Result<(), ()> {
        let service = service.to_string();
        let account = account.to_string();
        tokio::task::spawn_blocking(move || {
            match keyring::Entry::new(&service, &account)
                .and_then(|entry| entry.delete_credential())
            {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(()),
            }
        })
        .await
        .map_err(|_| ())?
    }
}

#[derive(Clone)]
pub struct ProviderOnboardingService {
    catalog: ProviderCatalogService,
    credentials: Arc<dyn ProviderCredentialStore>,
    client: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedRunModel {
    pub profile_id: ProviderProfileId,
    pub provider_type: String,
    pub wire_protocol: String,
    pub base_url: String,
    pub model: String,
    pub reasoning: String,
    pub catalog_revision: String,
    pub safe_config_digest: String,
}

pub type RunModelSnapshot = rove_runtime::runtime_identity::RunModelSnapshot;

#[derive(Debug, thiserror::Error)]
pub enum ProviderCatalogError {
    #[error("provider configuration is unavailable: {0}")]
    Unavailable(String),
    #[error("provider profile id `{0}` is invalid")]
    InvalidProfileId(String),
    #[error("provider profile `{0}` was not found")]
    ProfileNotFound(String),
    #[error("provider model selection is busy while a run is active")]
    Busy,
    #[error("provider catalog revision conflict")]
    RevisionConflict,
    #[error("credential for `{profile}` is unavailable: {reason}")]
    CredentialUnavailable { profile: String, reason: String },
    #[error("invalid provider catalog: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct ProviderCatalog {
    document: UserConfigDocument,
    revision: String,
    modified_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct ProviderCatalogService {
    paths: UserConfigPaths,
}

impl ProviderCatalogService {
    pub fn discover() -> Self {
        Self::new(UserConfigPaths::discover())
    }

    pub fn new(paths: UserConfigPaths) -> Self {
        Self { paths }
    }

    pub fn paths(&self) -> &UserConfigPaths {
        &self.paths
    }

    pub fn load(&self) -> Result<ProviderCatalog, ProviderCatalogError> {
        let document = UserConfigLoader::new(self.paths.clone())
            .load_or_default()
            .map_err(map_user_config_error)?;
        ProviderCatalog::from_document_with_modified_at(document, self.config_modified_at())
    }

    pub fn replace(
        &self,
        expected_revision: &str,
        document: &UserConfigDocument,
    ) -> Result<ProviderCatalog, ProviderCatalogError> {
        let document = UserConfigWriter::new(self.paths.clone())
            .update(Some(expected_revision), document)
            .map_err(map_user_config_error)?;
        ProviderCatalog::from_document_with_modified_at(document, self.config_modified_at())
    }

    pub fn upsert_profile(
        &self,
        expected_revision: &str,
        profile_id: ProviderProfileId,
        profile: crate::provider::ProviderProfileConfig,
    ) -> Result<ProviderCatalog, ProviderCatalogError> {
        let catalog = self.load()?;
        let mut document = catalog.document.clone();
        document
            .provider
            .profiles
            .insert(profile_id.0.clone(), profile);
        if document.model.default_profile.is_none() {
            document.model.default_profile = Some(profile_id.0);
        }
        self.replace(expected_revision, &document)
    }

    pub fn delete_profile(
        &self,
        expected_revision: &str,
        profile_id: &ProviderProfileId,
    ) -> Result<ProviderCatalog, ProviderCatalogError> {
        let catalog = self.load()?;
        let mut document = catalog.document.clone();
        if document.provider.profiles.remove(&profile_id.0).is_none() {
            return Err(ProviderCatalogError::ProfileNotFound(profile_id.0.clone()));
        }
        document
            .provider
            .fallback_profiles
            .retain(|candidate| candidate != &profile_id.0);
        if document.model.default_profile.as_deref() == Some(profile_id.0.as_str()) {
            document.model.default_profile = document.provider.profiles.keys().next().cloned();
            document.model.default_model = document
                .model
                .default_profile
                .as_deref()
                .and_then(|id| document.provider.profiles.get(id))
                .map(|profile| profile.model.clone());
        }
        self.replace(expected_revision, &document)
    }

    fn config_modified_at(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.paths.config_file)
            .and_then(|metadata| metadata.modified())
            .ok()
    }
}

impl ProviderOnboardingService {
    pub fn new(catalog: ProviderCatalogService) -> Self {
        Self::with_credential_store(catalog, Arc::new(OsProviderCredentialStore))
    }

    pub fn discover() -> Self {
        Self::new(ProviderCatalogService::discover())
    }

    pub fn with_credential_store(
        catalog: ProviderCatalogService,
        credentials: Arc<dyn ProviderCredentialStore>,
    ) -> Self {
        Self {
            catalog,
            credentials,
            client: reqwest::Client::new(),
        }
    }

    pub fn catalog(&self) -> &ProviderCatalogService {
        &self.catalog
    }

    pub async fn onboard(
        &self,
        request: ProviderOnboardingRequest,
    ) -> Result<ProviderOnboardingReceipt, ProviderOnboardingError> {
        let initial = self.catalog.load().map_err(map_onboarding_catalog_error)?;
        let expected_revision = request
            .expected_revision
            .as_deref()
            .unwrap_or(initial.revision());
        if expected_revision != initial.revision() {
            return Err(ProviderOnboardingError::RevisionConflict);
        }

        let (secret_source, probe_secret_source, staged_keyring) = self
            .stage_credential(&request.profile_id, &request.credential)
            .await?;
        let profile = match build_onboarding_profile(&request, secret_source) {
            Ok(profile) => profile,
            Err(error) => {
                self.compensate(staged_keyring.as_ref()).await;
                return Err(error);
            }
        };
        let mut probe_profile = profile.clone();
        replace_profile_auth_secret(&mut probe_profile, probe_secret_source);
        let probe = match self.probe_config(&request.profile_id, &probe_profile).await {
            Ok(probe) => probe,
            Err(error) => {
                self.compensate(staged_keyring.as_ref()).await;
                return Err(error);
            }
        };

        let mut document = initial.document().clone();
        document
            .provider
            .profiles
            .insert(request.profile_id.0.clone(), profile);
        if request.make_default || document.model.default_profile.is_none() {
            document.model.default_profile = Some(request.profile_id.0.clone());
            document.model.default_model = Some(request.model.trim().to_string());
        }

        let published = match self.catalog.replace(expected_revision, &document) {
            Ok(catalog) => catalog,
            Err(error) => {
                self.compensate(staged_keyring.as_ref()).await;
                return Err(map_onboarding_catalog_error(error));
            }
        };
        let verified = self
            .catalog
            .load()
            .map_err(|_| ProviderOnboardingError::ReconciliationRequired)?;
        let verified_profile = verified
            .profiles()
            .into_iter()
            .find(|profile| profile.id == request.profile_id)
            .ok_or(ProviderOnboardingError::ReconciliationRequired)?;
        if verified.revision() != published.revision()
            || verified_profile.model != request.model.trim()
            || verified_profile.provider_type != request.provider_type.trim().to_ascii_lowercase()
        {
            return Err(ProviderOnboardingError::ReconciliationRequired);
        }

        let selected = verified
            .default_selection()
            .map(|selection| {
                selection.profile_id == request.profile_id
                    && selection.model == request.model.trim()
            })
            .unwrap_or(false);
        Ok(ProviderOnboardingReceipt {
            profile_id: request.profile_id,
            provider_type: verified_profile.provider_type,
            base_url: verified_profile.base_url,
            model: verified_profile.model,
            catalog_revision: verified.revision().to_string(),
            credential: verified_profile.auth_source,
            probe,
            selected,
        })
    }

    pub async fn probe(
        &self,
        profile_id: &ProviderProfileId,
        model_override: Option<&str>,
    ) -> Result<ProviderProbeReceipt, ProviderOnboardingError> {
        let catalog = self.catalog.load().map_err(map_onboarding_catalog_error)?;
        let mut profile = catalog
            .profile_config(profile_id)
            .map_err(map_onboarding_catalog_error)?
            .clone();
        if let Some(model) = model_override {
            profile.model = model.trim().to_string();
        }
        self.probe_config(profile_id, &profile).await
    }

    pub fn use_profile(
        &self,
        profile_id: &ProviderProfileId,
        model_override: Option<&str>,
        expected_revision: Option<&str>,
    ) -> Result<ModelSelection, ProviderOnboardingError> {
        let catalog = self.catalog.load().map_err(map_onboarding_catalog_error)?;
        if expected_revision.is_some_and(|expected| expected != catalog.revision()) {
            return Err(ProviderOnboardingError::RevisionConflict);
        }
        let profile = catalog
            .profile_config(profile_id)
            .map_err(map_onboarding_catalog_error)?;
        let model = model_override.unwrap_or(&profile.model).trim().to_string();
        if model.is_empty() || model.len() > 1024 {
            return Err(ProviderOnboardingError::Invalid(
                "model is empty or too long".to_string(),
            ));
        }
        let mut document = catalog.document().clone();
        document.model.default_profile = Some(profile_id.0.clone());
        document.model.default_model = Some(model.clone());
        let updated = self
            .catalog
            .replace(catalog.revision(), &document)
            .map_err(map_onboarding_catalog_error)?;
        Ok(ModelSelection {
            profile_id: profile_id.clone(),
            model,
            reasoning: updated.document().model.reasoning.clone(),
            revision: updated.revision().to_string(),
        })
    }

    async fn stage_credential(
        &self,
        profile_id: &ProviderProfileId,
        credential: &OnboardingCredential,
    ) -> Result<(SecretSource, SecretSource, Option<KeyringReference>), ProviderOnboardingError>
    {
        match credential {
            OnboardingCredential::Secret(secret) => {
                if secret.trim().is_empty() || secret.len() > 16 * 1024 {
                    return Err(ProviderOnboardingError::Invalid(
                        "credential is empty or exceeds the size limit".to_string(),
                    ));
                }
                let reference = unique_keyring_reference(profile_id);
                self.credentials
                    .put(&reference.service, &reference.account, secret.trim())
                    .await
                    .map_err(|_| ProviderOnboardingError::CredentialStore)?;
                Ok((
                    SecretSource::Keyring {
                        keyring: reference.clone(),
                    },
                    SecretSource::Literal(secret.trim().to_string()),
                    Some(reference),
                ))
            }
            OnboardingCredential::Reference(reference) => {
                Ok((reference.clone(), reference.clone(), None))
            }
            OnboardingCredential::None => Ok((
                SecretSource::Env {
                    env: "ROVE_UNUSED_CREDENTIAL".to_string(),
                },
                SecretSource::Env {
                    env: "ROVE_UNUSED_CREDENTIAL".to_string(),
                },
                None,
            )),
        }
    }

    async fn compensate(&self, reference: Option<&KeyringReference>) {
        if let Some(reference) = reference {
            let _ = self
                .credentials
                .delete(&reference.service, &reference.account)
                .await;
        }
    }

    async fn probe_config(
        &self,
        profile_id: &ProviderProfileId,
        profile: &ProviderProfileConfig,
    ) -> Result<ProviderProbeReceipt, ProviderOnboardingError> {
        profile
            .validate(&self.catalog.paths().root, true)
            .map_err(|error| ProviderOnboardingError::Invalid(error.to_string()))?;
        let endpoint = inventory_endpoint(profile)?;
        let headers = profile
            .resolve_http_headers(&self.catalog.paths().root, true, &BTreeMap::new())
            .map_err(|_| ProviderOnboardingError::Probe {
                kind: ProviderProbeFailureKind::Unauthorized,
            })?;
        let response = self
            .client
            .get(endpoint)
            .headers(headers)
            .timeout(PROVIDER_PROBE_TIMEOUT)
            .send()
            .await
            .map_err(classify_probe_transport)?;
        if !response.status().is_success() {
            return Err(ProviderOnboardingError::Probe {
                kind: classify_probe_status(response.status()),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_INVENTORY_BYTES as u64)
        {
            return Err(ProviderOnboardingError::Probe {
                kind: ProviderProbeFailureKind::InvalidResponse,
            });
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(classify_probe_transport)?;
            if bytes.len().saturating_add(chunk.len()) > MAX_INVENTORY_BYTES {
                return Err(ProviderOnboardingError::Probe {
                    kind: ProviderProbeFailureKind::InvalidResponse,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| ProviderOnboardingError::Probe {
                kind: ProviderProbeFailureKind::InvalidResponse,
            })?;
        let models = inventory_model_ids(&profile.provider_type, &value)?;
        if !models.iter().any(|model| model == &profile.model) {
            return Err(ProviderOnboardingError::Probe {
                kind: ProviderProbeFailureKind::ModelUnavailable,
            });
        }
        let provider_type = profile.provider_type.trim().to_ascii_lowercase();
        Ok(ProviderProbeReceipt {
            profile_id: profile_id.clone(),
            provider_type: provider_type.clone(),
            base_url: profile.base_url.trim().trim_end_matches('/').to_string(),
            model: profile.model.clone(),
            inventory_count: models.len(),
            streaming_supported: true,
            native_tool_calls_supported: matches!(
                provider_type.as_str(),
                "openai" | "openai-responses" | "anthropic" | "ollama"
            ),
            usage_supported: matches!(
                provider_type.as_str(),
                "openai" | "openai-responses" | "anthropic"
            ),
        })
    }
}

fn replace_profile_auth_secret(profile: &mut ProviderProfileConfig, secret: SecretSource) {
    match &mut profile.auth {
        ProviderAuthConfig::Bearer {
            secret: profile_secret,
        }
        | ProviderAuthConfig::Header {
            secret: profile_secret,
            ..
        } => *profile_secret = secret,
        ProviderAuthConfig::None => {}
    }
}

fn build_onboarding_profile(
    request: &ProviderOnboardingRequest,
    secret: SecretSource,
) -> Result<ProviderProfileConfig, ProviderOnboardingError> {
    let provider_type = request.provider_type.trim().to_ascii_lowercase();
    let auth = match provider_type.as_str() {
        "openai" | "openai-responses"
            if matches!(&request.credential, OnboardingCredential::None) =>
        {
            return Err(ProviderOnboardingError::Invalid(
                "remote Provider credential is required".to_string(),
            ));
        }
        "openai" | "openai-responses" => ProviderAuthConfig::Bearer { secret },
        "anthropic" if matches!(&request.credential, OnboardingCredential::None) => {
            return Err(ProviderOnboardingError::Invalid(
                "remote Provider credential is required".to_string(),
            ));
        }
        "anthropic" => ProviderAuthConfig::Header {
            header: "x-api-key".to_string(),
            secret,
        },
        "ollama" => ProviderAuthConfig::None,
        _ => {
            return Err(ProviderOnboardingError::Invalid(
                "provider type must be openai, openai-responses, anthropic, or ollama".to_string(),
            ));
        }
    };
    let profile = ProviderProfileConfig {
        label: Some(request.label.trim().to_string()),
        provider_type,
        base_url: request.base_url.trim().trim_end_matches('/').to_string(),
        model: request.model.trim().to_string(),
        auth,
        headers: BTreeMap::new(),
        options: rove_models::ProviderOptions::default(),
        protocol_options: serde_json::json!({}),
    };
    profile
        .validate(Path::new("."), true)
        .map_err(|error| ProviderOnboardingError::Invalid(error.to_string()))?;
    Ok(profile)
}

fn inventory_endpoint(profile: &ProviderProfileConfig) -> Result<String, ProviderOnboardingError> {
    let base = profile.base_url.trim().trim_end_matches('/');
    let suffix = match profile.provider_type.trim().to_ascii_lowercase().as_str() {
        "openai" | "openai-responses" => "/models",
        "anthropic" if base.ends_with("/v1") => "/models",
        "anthropic" => "/v1/models",
        "ollama" => "/api/tags",
        _ => {
            return Err(ProviderOnboardingError::Invalid(
                "provider type does not support model inventory".to_string(),
            ));
        }
    };
    Ok(format!("{base}{suffix}"))
}

fn inventory_model_ids(
    provider_type: &str,
    value: &serde_json::Value,
) -> Result<Vec<String>, ProviderOnboardingError> {
    let (collection, field) = if provider_type.eq_ignore_ascii_case("ollama") {
        (value.get("models"), "name")
    } else {
        (value.get("data"), "id")
    };
    let models =
        collection
            .and_then(serde_json::Value::as_array)
            .ok_or(ProviderOnboardingError::Probe {
                kind: ProviderProbeFailureKind::InvalidResponse,
            })?;
    let ids = models
        .iter()
        .filter_map(|item| item.get(field).and_then(serde_json::Value::as_str))
        .filter(|id| !id.is_empty() && id.len() <= 1024)
        .take(4096)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Err(ProviderOnboardingError::Probe {
            kind: ProviderProbeFailureKind::InvalidResponse,
        });
    }
    Ok(ids)
}

fn classify_probe_status(status: StatusCode) -> ProviderProbeFailureKind {
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderProbeFailureKind::Unauthorized,
        StatusCode::TOO_MANY_REQUESTS => ProviderProbeFailureKind::RateLimited,
        status if status.is_server_error() => ProviderProbeFailureKind::Upstream,
        _ => ProviderProbeFailureKind::InvalidResponse,
    }
}

fn classify_probe_transport(error: reqwest::Error) -> ProviderOnboardingError {
    ProviderOnboardingError::Probe {
        kind: if error.is_timeout() {
            ProviderProbeFailureKind::Timeout
        } else {
            ProviderProbeFailureKind::Transport
        },
    }
}

fn unique_keyring_reference(profile_id: &ProviderProfileId) -> KeyringReference {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut digest = Sha256::new();
    digest.update(profile_id.0.as_bytes());
    digest.update(std::process::id().to_le_bytes());
    digest.update(nonce.to_le_bytes());
    let digest = format!("{:x}", digest.finalize());
    KeyringReference {
        service: PROVIDER_KEYRING_SERVICE.to_string(),
        account: format!("{}-{}", profile_id.0, &digest[..16]),
    }
}

fn map_onboarding_catalog_error(error: ProviderCatalogError) -> ProviderOnboardingError {
    match error {
        ProviderCatalogError::RevisionConflict | ProviderCatalogError::Busy => {
            ProviderOnboardingError::RevisionConflict
        }
        other => ProviderOnboardingError::Catalog(other.to_string()),
    }
}

fn map_user_config_error(error: crate::user_config::UserConfigError) -> ProviderCatalogError {
    match error {
        crate::user_config::UserConfigError::RevisionConflict => {
            ProviderCatalogError::RevisionConflict
        }
        crate::user_config::UserConfigError::Busy => ProviderCatalogError::Busy,
        other => ProviderCatalogError::Invalid(other.to_string()),
    }
}

impl ProviderCatalog {
    pub fn from_document(document: UserConfigDocument) -> Result<Self, ProviderCatalogError> {
        Self::from_document_with_modified_at(document, None)
    }

    fn from_document_with_modified_at(
        document: UserConfigDocument,
        modified_at: Option<SystemTime>,
    ) -> Result<Self, ProviderCatalogError> {
        document
            .validate()
            .map_err(|error| ProviderCatalogError::Invalid(error.to_string()))?;
        let revision = document.revision();
        Ok(Self {
            document,
            revision,
            modified_at,
        })
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn modified_at(&self) -> Option<SystemTime> {
        self.modified_at
    }

    pub fn profiles(&self) -> Vec<ProviderProfile> {
        self.document
            .provider
            .profiles
            .iter()
            .map(|(id, profile)| ProviderProfile {
                id: ProviderProfileId(id.clone()),
                label: profile.label.clone().unwrap_or_else(|| id.clone()),
                provider_type: profile.provider_type.clone(),
                base_url: profile.base_url.clone(),
                model: profile.model.clone(),
                auth_source: credential_reference(&profile.auth),
                fallback: self
                    .document
                    .provider
                    .fallback_profiles
                    .iter()
                    .any(|item| item == id),
            })
            .collect()
    }

    pub fn models(&self) -> Vec<ModelDescriptor> {
        self.profiles()
            .into_iter()
            .map(|profile| ModelDescriptor {
                id: profile.model.clone(),
                profile_id: profile.id,
                provider_type: profile.provider_type,
                supports_reasoning: false,
                inventory_fresh: false,
            })
            .collect()
    }

    pub fn document(&self) -> &UserConfigDocument {
        &self.document
    }

    pub fn profile_config(
        &self,
        profile_id: &ProviderProfileId,
    ) -> Result<&crate::provider::ProviderProfileConfig, ProviderCatalogError> {
        self.document
            .profile(&profile_id.0)
            .ok_or_else(|| ProviderCatalogError::ProfileNotFound(profile_id.0.clone()))
    }

    pub fn default_selection(&self) -> Result<ModelSelection, ProviderCatalogError> {
        let profile = self
            .document
            .model
            .default_profile
            .as_deref()
            .or_else(|| {
                self.document
                    .provider
                    .profiles
                    .keys()
                    .next()
                    .map(String::as_str)
            })
            .ok_or_else(|| {
                ProviderCatalogError::Unavailable("no provider profile configured".to_string())
            })?;
        let profile_id = ProviderProfileId::new(profile.to_string())?;
        let model = self
            .document
            .model
            .default_model
            .clone()
            .or_else(|| {
                self.document
                    .profile(profile)
                    .map(|item| item.model.clone())
            })
            .ok_or_else(|| {
                ProviderCatalogError::Unavailable("no provider model configured".to_string())
            })?;
        Ok(ModelSelection {
            profile_id,
            model,
            reasoning: self.document.model.reasoning.clone(),
            revision: self.revision.clone(),
        })
    }

    pub fn resolve(
        &self,
        selection: &ModelSelection,
        workspace: &Path,
    ) -> Result<ResolvedRunModel, ProviderCatalogError> {
        if selection.revision != self.revision {
            return Err(ProviderCatalogError::RevisionConflict);
        }
        let profile = self.profile_config(&selection.profile_id)?;
        profile
            .validate(workspace, true)
            .map_err(|error| ProviderCatalogError::Invalid(error.to_string()))?;
        if selection.model.trim().is_empty() || selection.model.len() > 1024 {
            return Err(ProviderCatalogError::Invalid(
                "selected model is empty or too long".to_string(),
            ));
        }
        let wire_protocol =
            crate::provider::wire_protocol_for_provider_type(&profile.provider_type)
                .map_err(|error| ProviderCatalogError::Invalid(error.to_string()))?;
        let safe = serde_json::json!({"profile_id": selection.profile_id, "provider_type": profile.provider_type, "base_url": profile.base_url.trim().trim_end_matches('/'), "wire_protocol": wire_protocol.as_str(), "model": selection.model, "reasoning": selection.reasoning, "options": profile.options, "protocol_options": profile.protocol_options, "auth": credential_reference(&profile.auth)});
        let mut digest = Sha256::new();
        digest.update(
            serde_json::to_vec(&safe)
                .map_err(|error| ProviderCatalogError::Invalid(error.to_string()))?,
        );
        Ok(ResolvedRunModel {
            profile_id: selection.profile_id.clone(),
            provider_type: profile.provider_type.clone(),
            wire_protocol: wire_protocol.to_string(),
            base_url: profile.base_url.trim().trim_end_matches('/').to_string(),
            model: selection.model.clone(),
            reasoning: selection.reasoning.clone(),
            catalog_revision: self.revision.clone(),
            safe_config_digest: format!("sha256:{:x}", digest.finalize()),
        })
    }

    pub fn snapshot(
        &self,
        selection: &ModelSelection,
        workspace: &Path,
    ) -> Result<RunModelSnapshot, ProviderCatalogError> {
        let resolved = self.resolve(selection, workspace)?;
        Ok(RunModelSnapshot {
            profile_id: resolved.profile_id.to_string(),
            provider_type: resolved.provider_type,
            wire_protocol: resolved.wire_protocol,
            endpoint: resolved.base_url,
            model: resolved.model,
            reasoning: resolved.reasoning,
            catalog_revision: resolved.catalog_revision,
            safe_config_digest: resolved.safe_config_digest,
        })
    }
}

fn credential_reference(auth: &crate::provider::ProviderAuthConfig) -> CredentialReference {
    match auth {
        crate::provider::ProviderAuthConfig::None => CredentialReference::None,
        crate::provider::ProviderAuthConfig::Bearer { secret }
        | crate::provider::ProviderAuthConfig::Header { secret, .. } => match secret {
            SecretSource::Env { env } => CredentialReference::Env { name: env.clone() },
            SecretSource::File { file } => CredentialReference::File {
                path: file.display().to_string(),
            },
            SecretSource::Keyring { keyring } => CredentialReference::Keyring {
                service: keyring.service.clone(),
                account: keyring.account.clone(),
            },
            SecretSource::Literal(_) => CredentialReference::None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingCredentialStore {
        puts: Mutex<Vec<(String, String, String)>>,
        deletes: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl ProviderCredentialStore for RecordingCredentialStore {
        async fn put(&self, service: &str, account: &str, secret: &str) -> Result<(), ()> {
            self.puts.lock().unwrap().push((
                service.to_string(),
                account.to_string(),
                secret.to_string(),
            ));
            Ok(())
        }

        async fn delete(&self, service: &str, account: &str) -> Result<(), ()> {
            self.deletes
                .lock()
                .unwrap()
                .push((service.to_string(), account.to_string()));
            Ok(())
        }
    }

    fn one_response_server(status: &str, body: &str) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_string();
        let body = body.to_string();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
        });
        (format!("http://{address}/v1"), thread)
    }

    #[test]
    fn catalog_resolves_immutable_secret_free_snapshot() {
        let document = UserConfigDocument::from_toml(
            "schema_version = 1\n[model]\ndefault_profile = 'local'\n[provider.profiles.local]\nprovider_type = 'ollama'\nbase_url = 'http://localhost:11434'\nmodel = 'llama3'",
        )
        .unwrap();
        let catalog = ProviderCatalog::from_document(document).unwrap();
        let selection = catalog.default_selection().unwrap();
        let snapshot = catalog.resolve(&selection, Path::new(".")).unwrap();
        assert_eq!(snapshot.model, "llama3");
        assert!(snapshot.safe_config_digest.starts_with("sha256:"));
        assert!(!serde_json::to_string(&snapshot).unwrap().contains("secret"));
    }

    #[test]
    fn snapshot_identity_changes_without_exposing_keyring_metadata_as_a_secret() {
        let document = UserConfigDocument::from_toml(
            "schema_version = 1\n[model]\ndefault_profile = 'remote'\n[provider.profiles.remote]\nprovider_type = 'openai'\nbase_url = 'https://example.test/v1'\nmodel = 'model'\n[provider.profiles.remote.auth]\nstyle = 'bearer'\nsecret = { keyring = { service = 'rove.test', account = 'user' } }",
        )
        .unwrap();
        let catalog = ProviderCatalog::from_document(document).unwrap();
        let selection = catalog.default_selection().unwrap();
        let snapshot = catalog.snapshot(&selection, Path::new(".")).unwrap();
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("rove.test"));
        assert!(!encoded.contains("account"));
        assert!(snapshot.safe_config_digest.starts_with("sha256:"));
    }

    #[tokio::test]
    async fn onboarding_probes_before_publishing_and_persists_only_a_keyring_reference() {
        let temp = tempfile::TempDir::new().unwrap();
        let catalog = ProviderCatalogService::new(UserConfigPaths::from_root(temp.path()));
        let credentials = Arc::new(RecordingCredentialStore::default());
        let service =
            ProviderOnboardingService::with_credential_store(catalog.clone(), credentials.clone());
        let (base_url, server) =
            one_response_server("200 OK", r#"{"data":[{"id":"deepseek-ai/DeepSeek-V3.2"}]}"#);
        let secret_canary = "credential-canary-must-not-be-serialized";

        let receipt = service
            .onboard(ProviderOnboardingRequest {
                profile_id: ProviderProfileId::new("siliconflow").unwrap(),
                label: "SiliconFlow".to_string(),
                provider_type: "openai".to_string(),
                base_url,
                model: "deepseek-ai/DeepSeek-V3.2".to_string(),
                credential: OnboardingCredential::Secret(secret_canary.to_string()),
                make_default: true,
                expected_revision: None,
            })
            .await
            .unwrap();
        server.join().unwrap();

        assert!(receipt.selected);
        assert!(receipt.probe.native_tool_calls_supported);
        assert_eq!(receipt.probe.inventory_count, 1);
        assert!(matches!(
            receipt.credential,
            CredentialReference::Keyring { .. }
        ));
        let config = std::fs::read_to_string(catalog.paths().config_file.clone()).unwrap();
        assert!(!config.contains(secret_canary));
        assert!(
            !serde_json::to_string(&receipt)
                .unwrap()
                .contains(secret_canary)
        );
        assert_eq!(credentials.puts.lock().unwrap().len(), 1);
        assert!(credentials.deletes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_probe_is_typed_redacted_and_compensates_staged_keyring_secret() {
        let temp = tempfile::TempDir::new().unwrap();
        let catalog = ProviderCatalogService::new(UserConfigPaths::from_root(temp.path()));
        let credentials = Arc::new(RecordingCredentialStore::default());
        let service =
            ProviderOnboardingService::with_credential_store(catalog.clone(), credentials.clone());
        let (base_url, server) = one_response_server("401 Unauthorized", "secret upstream body");
        let secret_canary = "credential-canary-must-not-leak";

        let error = service
            .onboard(ProviderOnboardingRequest {
                profile_id: ProviderProfileId::new("remote").unwrap(),
                label: "Remote".to_string(),
                provider_type: "openai".to_string(),
                base_url,
                model: "model".to_string(),
                credential: OnboardingCredential::Secret(secret_canary.to_string()),
                make_default: true,
                expected_revision: None,
            })
            .await
            .unwrap_err();
        server.join().unwrap();

        assert!(matches!(
            error,
            ProviderOnboardingError::Probe {
                kind: ProviderProbeFailureKind::Unauthorized
            }
        ));
        let rendered = error.to_string();
        assert!(!rendered.contains(secret_canary));
        assert!(!rendered.contains("upstream body"));
        assert_eq!(credentials.puts.lock().unwrap().len(), 1);
        assert_eq!(credentials.deletes.lock().unwrap().len(), 1);
        assert!(catalog.load().unwrap().profiles().is_empty());
    }

    #[test]
    fn onboarding_debug_output_redacts_in_memory_secret() {
        let request = ProviderOnboardingRequest {
            profile_id: ProviderProfileId::new("remote").unwrap(),
            label: "Remote".to_string(),
            provider_type: "openai".to_string(),
            base_url: "https://example.test/v1".to_string(),
            model: "model".to_string(),
            credential: OnboardingCredential::Secret("debug-secret-canary".to_string()),
            make_default: true,
            expected_revision: None,
        };
        let rendered = format!("{request:?}");
        assert!(!rendered.contains("debug-secret-canary"));
        assert!(rendered.contains("[REDACTED]"));
    }
}
