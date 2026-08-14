use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::provider::{ProviderAuthConfig, ProviderHeaderValue, ProviderProfileConfig};

pub const USER_CONFIG_SCHEMA_VERSION: u16 = 1;
const MAX_CONFIG_BYTES: usize = 256 * 1024;
const MAX_PROFILES: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UserConfigDocument {
    pub schema_version: u16,
    pub model: ModelDefaults,
    pub provider: UserProviderConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ModelDefaults {
    pub default_profile: Option<String>,
    pub default_model: Option<String>,
    pub reasoning: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct UserProviderConfig {
    pub fallback_profiles: Vec<String>,
    pub profiles: BTreeMap<String, ProviderProfileConfig>,
}

impl Default for UserConfigDocument {
    fn default() -> Self {
        Self {
            schema_version: USER_CONFIG_SCHEMA_VERSION,
            model: ModelDefaults::default(),
            provider: UserProviderConfig::default(),
        }
    }
}

impl Default for ModelDefaults {
    fn default() -> Self {
        Self {
            default_profile: None,
            default_model: None,
            reasoning: "default".to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UserConfigError {
    #[error("user provider configuration is missing at {path}")]
    Missing { path: String },
    #[error("user provider configuration is invalid: {message}")]
    Invalid { message: String },
    #[error(
        "user provider configuration schema version {found} is unsupported (expected {expected})"
    )]
    UnsupportedSchema { found: u16, expected: u16 },
    #[error(
        "user provider configuration contains a literal credential; use an env or file reference"
    )]
    LiteralCredential,
    #[error("user provider configuration revision conflict")]
    RevisionConflict,
    #[error("user provider configuration is busy")]
    Busy,
    #[error("provider profile `{profile}` is invalid: {message}")]
    InvalidProfile { profile: String, message: String },
}

impl UserConfigDocument {
    pub fn from_toml(text: &str) -> Result<Self, UserConfigError> {
        if text.len() > MAX_CONFIG_BYTES {
            return Err(UserConfigError::Invalid {
                message: format!("document exceeds {MAX_CONFIG_BYTES} bytes"),
            });
        }
        let value: toml::Value =
            toml::from_str(text).map_err(|error| UserConfigError::Invalid {
                message: error.to_string(),
            })?;
        if contains_literal_credential_value(&value) {
            return Err(UserConfigError::LiteralCredential);
        }
        let document: Self = toml::from_str(text).map_err(|error| UserConfigError::Invalid {
            message: error.to_string(),
        })?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_toml(&self) -> Result<String, UserConfigError> {
        self.validate()?;
        toml::to_string_pretty(self).map_err(|error| UserConfigError::Invalid {
            message: error.to_string(),
        })
    }

    pub fn validate(&self) -> Result<(), UserConfigError> {
        if self.schema_version != USER_CONFIG_SCHEMA_VERSION {
            return Err(UserConfigError::UnsupportedSchema {
                found: self.schema_version,
                expected: USER_CONFIG_SCHEMA_VERSION,
            });
        }
        if self.provider.profiles.len() > MAX_PROFILES {
            return Err(UserConfigError::Invalid {
                message: format!("too many provider profiles (maximum {MAX_PROFILES})"),
            });
        }
        for (id, profile) in &self.provider.profiles {
            validate_profile_id(id)?;
            if contains_literal_secret(profile) {
                return Err(UserConfigError::LiteralCredential);
            }
            profile
                .validate(std::path::Path::new("."), true)
                .map_err(|error| UserConfigError::InvalidProfile {
                    profile: id.clone(),
                    message: error.to_string(),
                })?;
        }
        if let Some(profile) = &self.model.default_profile {
            validate_profile_id(profile)?;
            if !self.provider.profiles.contains_key(profile) {
                return Err(UserConfigError::Invalid {
                    message: format!(
                        "model.default_profile references unknown profile `{profile}`"
                    ),
                });
            }
        }
        for profile in &self.provider.fallback_profiles {
            validate_profile_id(profile)?;
            if !self.provider.profiles.contains_key(profile) {
                return Err(UserConfigError::Invalid {
                    message: format!(
                        "provider.fallback_profiles references unknown profile `{profile}`"
                    ),
                });
            }
        }
        Ok(())
    }

    pub fn revision(&self) -> String {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        let mut digest = Sha256::new();
        digest.update(encoded);
        format!("sha256:{:x}", digest.finalize())
    }

    pub fn profile(&self, id: &str) -> Option<&ProviderProfileConfig> {
        self.provider.profiles.get(id)
    }
}

fn contains_literal_credential_value(value: &toml::Value) -> bool {
    let Some(profiles) = value
        .get("provider")
        .and_then(|provider| provider.get("profiles"))
        .and_then(toml::Value::as_table)
    else {
        return false;
    };
    profiles.values().any(|profile| {
        let auth_literal = profile
            .get("auth")
            .and_then(|auth| auth.get("secret"))
            .is_some_and(toml::Value::is_str);
        let header_literal = profile
            .get("headers")
            .and_then(toml::Value::as_table)
            .is_some_and(|headers| headers.values().any(toml::Value::is_str));
        auth_literal || header_literal
    })
}

fn validate_profile_id(id: &str) -> Result<(), UserConfigError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(UserConfigError::Invalid {
            message: format!("profile id `{id}` is invalid"),
        });
    }
    Ok(())
}

fn contains_literal_secret(profile: &ProviderProfileConfig) -> bool {
    let auth_literal = match &profile.auth {
        ProviderAuthConfig::Bearer { secret } | ProviderAuthConfig::Header { secret, .. } => {
            matches!(secret, crate::provider::SecretSource::Literal(_))
        }
        ProviderAuthConfig::None => false,
    };
    auth_literal
        || profile
            .headers
            .values()
            .any(|value| matches!(value, ProviderHeaderValue::Literal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_document_round_trips_redacted_references() {
        let text = r#"
schema_version = 1
[model]
default_profile = "local"
default_model = "llama3"
[provider.profiles.local]
provider_type = "ollama"
base_url = "http://localhost:11434"
model = "llama3"
[provider.profiles.local.auth]
style = "none"
"#;
        let document = UserConfigDocument::from_toml(text).unwrap();
        assert_eq!(document.model.default_profile.as_deref(), Some("local"));
        assert!(document.revision().starts_with("sha256:"));
        assert!(!document.to_toml().unwrap().contains("Literal"));
    }

    #[test]
    fn user_document_rejects_unknown_project_profile_and_schema() {
        let error = UserConfigDocument::from_toml("schema_version = 99").unwrap_err();
        assert!(matches!(error, UserConfigError::UnsupportedSchema { .. }));
        let error = UserConfigDocument::from_toml(
            "schema_version = 1\n[model]\ndefault_profile = 'missing'",
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown profile"));
    }

    #[test]
    fn user_document_rejects_literal_auth_and_header_credentials() {
        let literal_auth = UserConfigDocument::from_toml(
            "schema_version = 1\n[provider.profiles.remote]\nprovider_type = 'openai'\nbase_url = 'https://example.test/v1'\nmodel = 'model'\n[provider.profiles.remote.auth]\nstyle = 'bearer'\nsecret = 'not-allowed'",
        )
        .unwrap_err();
        assert!(matches!(literal_auth, UserConfigError::LiteralCredential));

        let literal_header = UserConfigDocument::from_toml(
            "schema_version = 1\n[provider.profiles.remote]\nprovider_type = 'openai'\nbase_url = 'https://example.test/v1'\nmodel = 'model'\n[provider.profiles.remote.headers]\nx-tenant = 'not-allowed'",
        )
        .unwrap_err();
        assert!(matches!(literal_header, UserConfigError::LiteralCredential));
    }
}
