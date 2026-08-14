//! Shared, UI-neutral Provider catalog and selection contracts.

use std::path::Path;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::provider::SecretSource;
use crate::user_config::{UserConfigDocument, UserConfigLoader, UserConfigPaths, UserConfigWriter};

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
    use super::*;

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
}
