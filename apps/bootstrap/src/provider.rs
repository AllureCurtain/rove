//! Named provider profiles, secret references, and protocol assembly helpers.

use std::{
    collections::{BTreeMap, HashSet},
    fmt,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::Context;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};

use rove_models::ProviderOptions;
use rove_models::provider::protocols::{
    AnthropicMessagesProtocol, OllamaChatProtocol, OpenAiCompletionsProtocol,
    OpenAiResponsesProtocol,
};
use rove_models::provider::{
    ProviderClientConfig, ResolvedAuth, ResolvedHeader, Transport, WireProtocolId,
    WireProtocolRegistry,
};

const MAX_SECRET_BYTES: usize = 16 * 1024;
const MAX_PROTOCOL_OPTIONS_BYTES: usize = 64 * 1024;
const MAX_PROFILE_MODEL_BYTES: usize = 1024;
const MAX_HEADER_COUNT: usize = 64;

/// A secret reference stored in configuration rather than the resolved value.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum SecretSource {
    Env {
        env: String,
    },
    File {
        file: PathBuf,
    },
    Keyring {
        keyring: KeyringReference,
    },
    /// In-memory only. Never loaded from durable config files; used for
    /// request-scoped API profiles that must embed a secret already resolved
    /// from the environment before the caller may clear that environment.
    #[serde(skip_deserializing)]
    Literal(#[serde(skip_serializing)] String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeyringReference {
    pub service: String,
    pub account: String,
}

impl fmt::Debug for SecretSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env { env } => formatter.debug_struct("Env").field("env", env).finish(),
            Self::File { file } => formatter.debug_struct("File").field("file", file).finish(),
            Self::Keyring { keyring } => formatter.debug_tuple("Keyring").field(keyring).finish(),
            Self::Literal(_) => formatter.write_str("Literal([REDACTED])"),
        }
    }
}

/// Authentication configuration for a named provider profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "style", rename_all = "lowercase")]
pub enum ProviderAuthConfig {
    #[default]
    None,
    Bearer {
        secret: SecretSource,
    },
    Header {
        header: String,
        secret: SecretSource,
    },
}

/// A literal or secret-backed custom HTTP header value.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ProviderHeaderValue {
    Literal(String),
    Env { env: String },
    File { file: PathBuf },
    Keyring { keyring: KeyringReference },
}

impl fmt::Debug for ProviderHeaderValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(_) => formatter.write_str("[REDACTED]"),
            Self::Env { env } => formatter.debug_struct("Env").field("env", env).finish(),
            Self::File { file } => formatter.debug_struct("File").field("file", file).finish(),
            Self::Keyring { keyring } => formatter.debug_tuple("Keyring").field(keyring).finish(),
        }
    }
}

impl ProviderHeaderValue {
    fn as_secret_source(&self) -> Option<SecretSource> {
        match self {
            Self::Env { env } => Some(SecretSource::Env { env: env.clone() }),
            Self::File { file } => Some(SecretSource::File { file: file.clone() }),
            Self::Keyring { keyring } => Some(SecretSource::Keyring {
                keyring: keyring.clone(),
            }),
            Self::Literal(_) => None,
        }
    }

    fn literal_value(&self) -> Option<&str> {
        match self {
            Self::Literal(value) => Some(value),
            Self::Env { .. } | Self::File { .. } | Self::Keyring { .. } => None,
        }
    }
}

/// Serializable, named endpoint profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderProfileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Product type: openai | openai-responses | anthropic | ollama | fake.
    /// System maps this to an internal wire protocol id.
    pub provider_type: String,
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub auth: ProviderAuthConfig,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, ProviderHeaderValue>,
    #[serde(default)]
    pub options: ProviderOptions,
    #[serde(default = "empty_protocol_options")]
    pub protocol_options: serde_json::Value,
}

/// Resolved profile data passed to `ProviderClient`.
#[derive(Clone)]
pub struct ResolvedProviderProfile {
    pub protocol_id: WireProtocolId,
    pub base_url: String,
    pub model: String,
    pub auth: ResolvedAuth,
    pub headers: Vec<ResolvedHeader>,
    pub options: ProviderOptions,
    pub protocol_options: serde_json::Value,
}

impl ProviderProfileConfig {
    /// Rebase relative credential files against their owning configuration
    /// directory. The durable document keeps its original relative paths;
    /// only runtime copies should be rebased.
    pub fn rebase_secret_paths(&mut self, credential_root: &Path) {
        match &mut self.auth {
            ProviderAuthConfig::Bearer { secret } | ProviderAuthConfig::Header { secret, .. } => {
                rebase_secret_source(secret, credential_root);
            }
            ProviderAuthConfig::None => {}
        }
        for value in self.headers.values_mut() {
            if let ProviderHeaderValue::File { file } = value
                && !file.is_absolute()
            {
                *file = normalize_lexical_path(&credential_root.join(&*file));
            }
        }
    }

    pub fn validate(
        &self,
        workspace_root: &Path,
        allow_external_paths: bool,
    ) -> anyhow::Result<()> {
        if self.label.as_ref().is_some_and(|label| {
            label.trim().is_empty() || label.len() > 512 || label.chars().any(char::is_control)
        }) {
            anyhow::bail!("profile.label is empty, too long, or invalid");
        }
        let protocol_id = wire_protocol_for_provider_type(&self.provider_type)?;
        let protocol = protocol_id.as_str();
        if protocol == "fake" {
            if !self.base_url.trim().is_empty() {
                anyhow::bail!("fake provider profile base_url must be empty");
            }
        } else if protocol == "external-adapter-v1" {
            // base_url is optional metadata for the adapter; when present it must
            // still be a valid absolute HTTP(S) URL without credentials.
            if !self.base_url.trim().is_empty() {
                Transport::validate_base_url(self.base_url.trim())
                    .map_err(|error| anyhow::anyhow!("profile endpoint is invalid: {error}"))?;
            }
            // Fail closed early on missing/invalid command arrays.
            let _ = rove_models::provider::ExternalAdapterConfig::from_protocol_options(
                &self.protocol_options,
                self.base_url.clone(),
                self.model.clone(),
                ResolvedAuth::none(),
                Vec::new(),
                self.options,
                workspace_root,
                allow_external_paths,
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        } else {
            Transport::validate_base_url(self.base_url.trim())
                .map_err(|error| anyhow::anyhow!("profile endpoint is invalid: {error}"))?;
        }
        validate_model(&self.model, "profile.model")?;
        validate_provider_options(&self.options, "profile.options")?;
        if !self.protocol_options.is_object() {
            anyhow::bail!("profile.protocol_options must be a JSON object");
        }
        let encoded = serde_json::to_vec(&self.protocol_options)?;
        if encoded.len() > MAX_PROTOCOL_OPTIONS_BYTES {
            anyhow::bail!("profile.protocol_options exceeds {MAX_PROTOCOL_OPTIONS_BYTES} bytes");
        }
        validate_auth_config(&self.auth, workspace_root, allow_external_paths)?;
        if self.headers.len() > MAX_HEADER_COUNT {
            anyhow::bail!("profile.headers must contain at most {MAX_HEADER_COUNT} entries");
        }
        let mut names = HashSet::new();
        for (name, value) in &self.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| anyhow::anyhow!("profile header name `{name}` is invalid"))?;
            if !names.insert(header_name.clone()) {
                anyhow::bail!("profile.headers contains duplicate header `{name}`");
            }
            validate_custom_header_name(&header_name)?;
            validate_header_value(value, workspace_root, allow_external_paths)?;
        }
        Ok(())
    }

    pub fn resolve(
        &self,
        workspace_root: &Path,
        allow_external_paths: bool,
        model_override: Option<&str>,
    ) -> anyhow::Result<ResolvedProviderProfile> {
        self.resolve_with_environment(
            workspace_root,
            allow_external_paths,
            model_override,
            &BTreeMap::new(),
        )
    }

    pub fn resolve_with_environment(
        &self,
        workspace_root: &Path,
        allow_external_paths: bool,
        model_override: Option<&str>,
        project_environment: &BTreeMap<String, String>,
    ) -> anyhow::Result<ResolvedProviderProfile> {
        self.validate(workspace_root, allow_external_paths)?;
        let model = model_override
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(&self.model)
            .trim()
            .to_string();
        validate_model(&model, "resolved profile model")?;
        let auth = resolve_auth(
            &self.auth,
            workspace_root,
            allow_external_paths,
            project_environment,
        )?;
        let mut headers = Vec::with_capacity(self.headers.len());
        for (name, value) in &self.headers {
            let resolved = value
                .as_secret_source()
                .map(|source| {
                    resolve_secret(
                        &source,
                        workspace_root,
                        allow_external_paths,
                        project_environment,
                    )
                })
                .transpose()?
                .or_else(|| value.literal_value().map(str::to_string))
                .ok_or_else(|| anyhow::anyhow!("profile header `{name}` has no value"))?;
            headers.push(ResolvedHeader::try_new(name, resolved)?);
        }
        Ok(ResolvedProviderProfile {
            protocol_id: wire_protocol_for_provider_type(&self.provider_type)?,
            base_url: self.base_url.trim().trim_end_matches('/').to_string(),
            model,
            auth,
            headers,
            options: self.options,
            protocol_options: self.protocol_options.clone(),
        })
    }

    /// Resolve the authentication and custom headers needed by an
    /// inventory request without exposing credential values in a serializable
    /// response type.
    pub fn resolve_http_headers(
        &self,
        credential_root: &Path,
        allow_external_paths: bool,
        project_environment: &BTreeMap<String, String>,
    ) -> anyhow::Result<HeaderMap> {
        self.validate(credential_root, allow_external_paths)?;
        let mut headers = HeaderMap::new();
        match &self.auth {
            ProviderAuthConfig::None => {}
            ProviderAuthConfig::Bearer { secret } => {
                let value = resolve_secret(
                    secret,
                    credential_root,
                    allow_external_paths,
                    project_environment,
                )?;
                insert_sensitive_header(&mut headers, AUTHORIZATION, format!("Bearer {value}"))?;
            }
            ProviderAuthConfig::Header { header, secret } => {
                let name = HeaderName::from_bytes(header.trim().as_bytes())
                    .map_err(|_| anyhow::anyhow!("profile auth header `{header}` is invalid"))?;
                let value = resolve_secret(
                    secret,
                    credential_root,
                    allow_external_paths,
                    project_environment,
                )?;
                insert_sensitive_header(&mut headers, name, value)?;
            }
        }
        for (name, value) in &self.headers {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| anyhow::anyhow!("profile header name `{name}` is invalid"))?;
            let value = value
                .as_secret_source()
                .map(|source| {
                    resolve_secret(
                        &source,
                        credential_root,
                        allow_external_paths,
                        project_environment,
                    )
                })
                .transpose()?
                .or_else(|| value.literal_value().map(str::to_string))
                .ok_or_else(|| anyhow::anyhow!("profile header `{name}` has no value"))?;
            insert_sensitive_header(&mut headers, name, value)?;
        }
        Ok(headers)
    }
}

impl ResolvedProviderProfile {
    pub fn into_client_config(self, client_namespace: impl Into<String>) -> ProviderClientConfig {
        ProviderClientConfig {
            client_namespace: client_namespace.into(),
            base_url: self.base_url,
            model: self.model,
            auth: self.auth,
            headers: self.headers,
            options: self.options,
            protocol_options: self.protocol_options,
        }
    }
}

/// Map product `provider_type` to the system wire protocol id.
pub fn wire_protocol_for_provider_type(provider_type: &str) -> anyhow::Result<WireProtocolId> {
    let id = match provider_type.trim().to_ascii_lowercase().as_str() {
        "openai" => "openai-completions",
        "openai-responses" => "openai-responses",
        "anthropic" => "anthropic-messages",
        "ollama" => "ollama",
        "fake" => "fake",
        "external-adapter-v1" => "external-adapter-v1",
        other => anyhow::bail!(
            "unsupported provider_type `{other}`; expected openai, openai-responses, anthropic, ollama, or fake"
        ),
    };
    WireProtocolId::new(id).map_err(|error| anyhow::anyhow!(error.to_string()))
}

pub fn protocol_client_namespace(protocol: &WireProtocolId) -> String {
    match protocol.as_str() {
        "openai-completions" => "openai".to_string(),
        "openai-responses" => "openai-responses".to_string(),
        "anthropic-messages" => "anthropic".to_string(),
        "ollama" => "ollama".to_string(),
        "fake" => "fake".to_string(),
        other => other.to_string(),
    }
}

pub fn default_wire_protocol_registry() -> Arc<WireProtocolRegistry> {
    let mut registry = WireProtocolRegistry::new();
    registry
        .register(Arc::new(OpenAiCompletionsProtocol::new()))
        .expect("built-in OpenAI Chat protocol ID must be unique");
    registry
        .register(Arc::new(OpenAiResponsesProtocol::new()))
        .expect("built-in OpenAI Responses protocol ID must be unique");
    registry
        .register(Arc::new(AnthropicMessagesProtocol::new()))
        .expect("built-in Anthropic protocol ID must be unique");
    registry
        .register(Arc::new(OllamaChatProtocol::new()))
        .expect("built-in Ollama protocol ID must be unique");
    Arc::new(registry)
}

fn validate_model(model: &str, field: &str) -> anyhow::Result<()> {
    let model = model.trim();
    if model.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    if model.len() > MAX_PROFILE_MODEL_BYTES {
        anyhow::bail!("{field} exceeds {MAX_PROFILE_MODEL_BYTES} bytes");
    }
    Ok(())
}

fn validate_provider_options(options: &ProviderOptions, prefix: &str) -> anyhow::Result<()> {
    if options.max_tokens == Some(0) {
        anyhow::bail!("{prefix}.max_tokens must be greater than 0");
    }
    for (name, value) in [
        ("temperature", options.temperature),
        ("top_p", options.top_p),
        ("frequency_penalty", options.frequency_penalty),
        ("presence_penalty", options.presence_penalty),
    ] {
        if let Some(value) = value
            && !value.is_finite()
        {
            anyhow::bail!("{prefix}.{name} must be finite");
        }
    }
    Ok(())
}

fn validate_auth_config(
    auth: &ProviderAuthConfig,
    workspace_root: &Path,
    allow_external_paths: bool,
) -> anyhow::Result<()> {
    match auth {
        ProviderAuthConfig::None => Ok(()),
        ProviderAuthConfig::Bearer { secret } => {
            validate_secret_source(secret, workspace_root, allow_external_paths)
        }
        ProviderAuthConfig::Header { header, secret } => {
            let name = HeaderName::from_bytes(header.trim().as_bytes())
                .map_err(|_| anyhow::anyhow!("profile auth header `{header}` is invalid"))?;
            validate_auth_header_name(&name)?;
            validate_secret_source(secret, workspace_root, allow_external_paths)
        }
    }
}

fn validate_custom_header_name(name: &HeaderName) -> anyhow::Result<()> {
    if matches!(
        name.as_str(),
        "authorization"
            | "content-length"
            | "transfer-encoding"
            | "host"
            | "connection"
            | "x-api-key"
            | "api-key"
    ) {
        anyhow::bail!("profile header `{}` is managed by provider transport", name);
    }
    Ok(())
}

fn validate_auth_header_name(name: &HeaderName) -> anyhow::Result<()> {
    if matches!(
        name.as_str(),
        "content-length" | "transfer-encoding" | "host" | "connection" | "content-type"
    ) {
        anyhow::bail!(
            "profile auth header `{}` is managed by provider transport",
            name
        );
    }
    Ok(())
}

fn validate_header_value(
    value: &ProviderHeaderValue,
    workspace_root: &Path,
    allow_external_paths: bool,
) -> anyhow::Result<()> {
    if let Some(literal) = value.literal_value() {
        HeaderValue::from_str(literal)
            .map_err(|_| anyhow::anyhow!("profile header value is invalid"))?;
        return Ok(());
    }
    let source = value
        .as_secret_source()
        .expect("non-literal header values have a secret source");
    validate_secret_source(&source, workspace_root, allow_external_paths)
}

fn validate_secret_source(
    source: &SecretSource,
    workspace_root: &Path,
    allow_external_paths: bool,
) -> anyhow::Result<()> {
    match source {
        SecretSource::Env { env } => validate_env_name(env),
        SecretSource::File { file } => {
            if file.as_os_str().is_empty() || file.to_string_lossy().len() > 1024 {
                anyhow::bail!("profile secret file path is empty or too long");
            }
            if file.to_string_lossy().contains('\0') {
                anyhow::bail!("profile secret file path contains NUL");
            }
            let resolved = resolve_secret_path(file, workspace_root);
            if !allow_external_paths && !resolved.starts_with(workspace_root) {
                anyhow::bail!("profile secret file resolves outside the workspace");
            }
            Ok(())
        }
        SecretSource::Keyring { keyring } => {
            validate_keyring_component("service", &keyring.service)?;
            validate_keyring_component("account", &keyring.account)
        }
        SecretSource::Literal(value) => {
            if value.trim().is_empty() {
                anyhow::bail!("profile secret literal is empty");
            }
            if value.len() > MAX_SECRET_BYTES {
                anyhow::bail!("profile secret exceeds {MAX_SECRET_BYTES} bytes");
            }
            Ok(())
        }
    }
}

fn validate_env_name(env: &str) -> anyhow::Result<()> {
    let bytes = env.as_bytes();
    if bytes.is_empty()
        || bytes.len() > 256
        || (!bytes[0].is_ascii_uppercase() && bytes[0] != b'_')
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
    {
        anyhow::bail!("profile secret environment name is invalid");
    }
    Ok(())
}

fn resolve_secret(
    source: &SecretSource,
    workspace_root: &Path,
    allow_external_paths: bool,
    project_environment: &BTreeMap<String, String>,
) -> anyhow::Result<String> {
    let value = match source {
        SecretSource::Env { env } => std::env::var(env)
            .ok()
            .or_else(|| project_environment.get(env).cloned())
            .with_context(|| format!("profile secret environment variable `{env}` is not set"))?,
        SecretSource::File { file } => read_secret_file(
            &resolve_secret_path(file, workspace_root),
            allow_external_paths,
            workspace_root,
        )?,
        SecretSource::Keyring { keyring } => {
            keyring::Entry::new(&keyring.service, &keyring.account)
                .and_then(|entry| entry.get_password())
                .with_context(|| "profile keyring credential is unavailable")?
        }
        SecretSource::Literal(value) => value.clone(),
    };
    let value = value.trim().to_string();
    if value.is_empty() {
        anyhow::bail!("profile secret source resolved to an empty value");
    }
    if value.len() > MAX_SECRET_BYTES {
        anyhow::bail!("profile secret exceeds {MAX_SECRET_BYTES} bytes");
    }
    Ok(value)
}

fn validate_keyring_component(field: &str, value: &str) -> anyhow::Result<()> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        anyhow::bail!("profile keyring {field} is empty, too long, or invalid");
    }
    Ok(())
}

fn read_secret_file(
    path: &Path,
    allow_external_paths: bool,
    workspace_root: &Path,
) -> anyhow::Result<String> {
    let path = if allow_external_paths {
        path.to_path_buf()
    } else {
        let canonical_workspace = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| normalize_lexical_path(workspace_root));
        let canonical_path = path
            .canonicalize()
            .with_context(|| "profile secret file could not be opened")?;
        if !canonical_path.starts_with(&canonical_workspace) {
            anyhow::bail!("profile secret file resolves outside the workspace");
        }
        canonical_path
    };
    let file =
        std::fs::File::open(&path).with_context(|| "profile secret file could not be opened")?;
    let mut bytes = Vec::new();
    file.take((MAX_SECRET_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| "profile secret file could not be read")?;
    if bytes.len() > MAX_SECRET_BYTES {
        anyhow::bail!("profile secret exceeds {MAX_SECRET_BYTES} bytes");
    }
    String::from_utf8(bytes).context("profile secret file must be UTF-8")
}

fn resolve_secret_path(path: &Path, workspace_root: &Path) -> PathBuf {
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    normalize_lexical_path(&resolved)
}

fn rebase_secret_source(source: &mut SecretSource, credential_root: &Path) {
    if let SecretSource::File { file } = source
        && !file.is_absolute()
    {
        *file = normalize_lexical_path(&credential_root.join(&*file));
    }
}

fn insert_sensitive_header(
    headers: &mut HeaderMap,
    name: HeaderName,
    value: String,
) -> anyhow::Result<()> {
    let mut value = HeaderValue::from_str(&value)
        .map_err(|_| anyhow::anyhow!("profile header value is invalid"))?;
    value.set_sensitive(true);
    if headers.insert(name.clone(), value).is_some() {
        anyhow::bail!("profile header `{name}` conflicts with the authentication header");
    }
    Ok(())
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn empty_protocol_options() -> serde_json::Value {
    serde_json::json!({})
}

fn resolve_auth(
    auth: &ProviderAuthConfig,
    workspace_root: &Path,
    allow_external_paths: bool,
    project_environment: &BTreeMap<String, String>,
) -> anyhow::Result<ResolvedAuth> {
    match auth {
        ProviderAuthConfig::None => Ok(ResolvedAuth::none()),
        ProviderAuthConfig::Bearer { secret } => Ok(ResolvedAuth::bearer(resolve_secret(
            secret,
            workspace_root,
            allow_external_paths,
            project_environment,
        )?)?),
        ProviderAuthConfig::Header { header, secret } => {
            let name = HeaderName::from_bytes(header.trim().as_bytes())
                .map_err(|_| anyhow::anyhow!("profile auth header `{header}` is invalid"))?;
            Ok(ResolvedAuth::header(
                name,
                resolve_secret(
                    secret,
                    workspace_root,
                    allow_external_paths,
                    project_environment,
                )?,
            )?)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn test_profile() -> ProviderProfileConfig {
        ProviderProfileConfig {
            label: None,
            provider_type: "openai".to_string(),
            base_url: "https://gateway.example.test/v1".to_string(),
            model: "team/model".to_string(),
            auth: ProviderAuthConfig::None,
            headers: BTreeMap::new(),
            options: ProviderOptions::default(),
            protocol_options: serde_json::json!({}),
        }
    }

    #[test]
    fn profile_debug_redacts_literal_header_values() {
        let mut profile = test_profile();
        profile.headers.insert(
            "x-tenant".to_string(),
            ProviderHeaderValue::Literal("header-secret".to_string()),
        );
        let debug = format!("{profile:?}");
        assert!(!debug.contains("header-secret"));
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn profile_resolves_env_secret_and_custom_header() {
        let name = "ROVE_TEST_PROFILE_SECRET";
        unsafe { std::env::set_var(name, "profile-secret") };
        let mut profile = test_profile();
        profile.auth = ProviderAuthConfig::Bearer {
            secret: SecretSource::Env {
                env: name.to_string(),
            },
        };
        profile.headers.insert(
            "x-tenant".to_string(),
            ProviderHeaderValue::Literal("tenant-a".to_string()),
        );
        let resolved = profile.resolve(Path::new("."), true, None).unwrap();
        assert!(format!("{:?}", resolved.auth).contains("REDACTED"));
        assert_eq!(resolved.headers.len(), 1);
        unsafe { std::env::remove_var(name) };
    }

    #[test]
    fn profile_reports_missing_environment_secret_without_a_value() {
        let name = "ROVE_TEST_PROFILE_MISSING_SECRET";
        unsafe { std::env::remove_var(name) };
        let mut profile = test_profile();
        profile.auth = ProviderAuthConfig::Bearer {
            secret: SecretSource::Env {
                env: name.to_string(),
            },
        };

        let error = profile
            .resolve(Path::new("."), true, None)
            .err()
            .unwrap()
            .to_string();

        assert!(error.contains(name));
        assert!(error.contains("is not set"));
    }

    #[test]
    fn profile_secret_files_are_workspace_bounded_sized_and_utf8() {
        let workspace = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let mut profile = test_profile();
        profile.auth = ProviderAuthConfig::Bearer {
            secret: SecretSource::File {
                file: PathBuf::from("secret.txt"),
            },
        };

        std::fs::write(workspace.path().join("secret.txt"), "file-secret\n").unwrap();
        let resolved = profile.resolve(workspace.path(), false, None).unwrap();
        assert!(!format!("{:?}", resolved.auth).contains("file-secret"));

        profile.auth = ProviderAuthConfig::Bearer {
            secret: SecretSource::File {
                file: outside.path().join("secret.txt"),
            },
        };
        std::fs::write(outside.path().join("secret.txt"), "outside-secret").unwrap();
        assert!(
            profile
                .resolve(workspace.path(), false, None)
                .err()
                .unwrap()
                .to_string()
                .contains("outside the workspace")
        );

        profile.auth = ProviderAuthConfig::Bearer {
            secret: SecretSource::File {
                file: PathBuf::from("secret.txt"),
            },
        };
        std::fs::write(
            workspace.path().join("secret.txt"),
            vec![b'x'; MAX_SECRET_BYTES + 1],
        )
        .unwrap();
        assert!(
            profile
                .resolve(workspace.path(), false, None)
                .err()
                .unwrap()
                .to_string()
                .contains("exceeds")
        );

        std::fs::write(workspace.path().join("secret.txt"), [0xff, 0xfe]).unwrap();
        assert!(
            profile
                .resolve(workspace.path(), false, None)
                .err()
                .unwrap()
                .to_string()
                .contains("must be UTF-8")
        );
    }

    #[test]
    fn inventory_headers_resolve_relative_files_from_the_supplied_credential_root() {
        let credential_root = tempfile::TempDir::new().unwrap();
        std::fs::write(
            credential_root.path().join("provider.key"),
            "provider-secret\n",
        )
        .unwrap();
        std::fs::write(credential_root.path().join("tenant.txt"), "tenant-a\n").unwrap();
        let mut profile = test_profile();
        profile.auth = ProviderAuthConfig::Bearer {
            secret: SecretSource::File {
                file: PathBuf::from("provider.key"),
            },
        };
        profile.headers.insert(
            "x-tenant".to_string(),
            ProviderHeaderValue::File {
                file: PathBuf::from("tenant.txt"),
            },
        );

        let headers = profile
            .resolve_http_headers(credential_root.path(), true, &BTreeMap::new())
            .unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).unwrap(),
            "Bearer provider-secret"
        );
        assert_eq!(headers.get("x-tenant").unwrap(), "tenant-a");
    }

    #[test]
    fn profile_auth_and_custom_headers_have_separate_managed_boundaries() {
        let mut profile = test_profile();
        profile.auth = ProviderAuthConfig::Header {
            header: "x-api-key".to_string(),
            secret: SecretSource::Env {
                env: "ANTHROPIC_API_KEY".to_string(),
            },
        };
        assert!(profile.validate(Path::new("."), true).is_ok());

        profile.auth = ProviderAuthConfig::Header {
            header: "content-type".to_string(),
            secret: SecretSource::Env {
                env: "ANTHROPIC_API_KEY".to_string(),
            },
        };
        assert!(
            profile
                .validate(Path::new("."), true)
                .unwrap_err()
                .to_string()
                .contains("managed by provider transport")
        );

        let mut profile = test_profile();
        profile.headers.insert(
            "X-Tenant".to_string(),
            ProviderHeaderValue::Literal("first".to_string()),
        );
        profile.headers.insert(
            "x-tenant".to_string(),
            ProviderHeaderValue::Literal("second".to_string()),
        );
        assert!(
            profile
                .validate(Path::new("."), true)
                .unwrap_err()
                .to_string()
                .contains("duplicate header")
        );
    }

    #[test]
    fn profile_rejects_managed_header_and_external_secret_path() {
        let mut profile = test_profile();
        profile.headers.insert(
            "authorization".to_string(),
            ProviderHeaderValue::Literal("bypass".to_string()),
        );
        assert!(profile.validate(Path::new("C:/workspace"), false).is_err());

        let mut profile = test_profile();
        profile.auth = ProviderAuthConfig::Bearer {
            secret: SecretSource::File {
                file: PathBuf::from("../outside/secret"),
            },
        };
        assert!(profile.validate(Path::new("C:/workspace"), false).is_err());
    }

    #[test]
    fn default_registry_contains_all_native_protocols() {
        let registry = default_wire_protocol_registry();
        assert_eq!(registry.len(), 4);
        assert!(
            registry
                .ids()
                .iter()
                .any(|id| id.as_str() == "openai-completions")
        );
        assert!(registry.ids().iter().any(|id| id.as_str() == "ollama"));
        assert!(
            registry
                .ids()
                .iter()
                .any(|id| id.as_str() == "anthropic-messages")
        );
    }
}
