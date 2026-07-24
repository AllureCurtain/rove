use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::Context;
use futures::stream::{self, BoxStream};
use reqwest::header::HeaderName;
use rove_models::fake::FakeModelClient;
use rove_models::health::{HealthConfig, ModelHealthStore};
use rove_models::provider::{
    ExternalAdapterClient, ExternalAdapterConfig, ProviderClient, ResolvedAuth, Transport,
    TransportConfig, WireProtocolId, WireProtocolRegistry,
};
use rove_models::routing::{RetryPolicy, RoutingModelClient};
use rove_models::{
    Message, ModelClient, ModelClientId, ModelError, ModelEvent, ProviderOptions, ToolSchema,
};

use crate::config::{AppConfig, FallbackProviderConfig};
use crate::provider::{
    ResolvedProviderProfile, default_wire_protocol_registry, protocol_client_namespace,
};

const DEFAULT_ANTHROPIC_BASE: &str = "https://api.anthropic.com";
const DEFAULT_OLLAMA_BASE: &str = "http://localhost:11434";

/// Registry and shared HTTP transport used to assemble provider targets.
///
/// Applications can inject additional in-process wire protocols without
/// changing the central bootstrap factory. Unsupported no-rebuild protocols
/// remain the responsibility of the later bounded sidecar adapter stage.
#[derive(Clone)]
pub struct ModelClientFactory {
    registry: Arc<WireProtocolRegistry>,
    transport: Arc<Transport>,
}

impl ModelClientFactory {
    pub fn new(registry: Arc<WireProtocolRegistry>, transport: Arc<Transport>) -> Self {
        Self {
            registry,
            transport,
        }
    }

    pub fn with_default_transport(registry: Arc<WireProtocolRegistry>) -> anyhow::Result<Self> {
        let transport = Transport::new(TransportConfig::default())
            .context("provider HTTP transport could not be initialized")?;
        Ok(Self::new(registry, Arc::new(transport)))
    }

    pub fn native() -> anyhow::Result<Self> {
        Self::with_default_transport(default_wire_protocol_registry())
    }

    pub fn try_build_model_client(
        &self,
        config: &AppConfig,
        model_id: String,
    ) -> anyhow::Result<Box<dyn ModelClient>> {
        self.try_build(config, PrimarySelection::Configured, model_id, None)
    }

    pub fn try_build_model_client_with_health(
        &self,
        config: &AppConfig,
        model_id: String,
        health: Arc<ModelHealthStore>,
    ) -> anyhow::Result<Box<dyn ModelClient>> {
        self.try_build(config, PrimarySelection::Configured, model_id, Some(health))
    }

    fn try_build(
        &self,
        config: &AppConfig,
        selection: PrimarySelection,
        model_id: String,
        health: Option<Arc<ModelHealthStore>>,
    ) -> anyhow::Result<Box<dyn ModelClient>> {
        let (primary, fallbacks) = match selection {
            PrimarySelection::Configured if !config.provider.profiles.is_empty() => {
                named_targets(config, &model_id)?
            }
            PrimarySelection::Configured => legacy_targets(config, None, model_id)?,
            PrimarySelection::Legacy(kind) => legacy_targets(config, Some(kind), model_id)?,
        };
        self.build_routed(config, primary, fallbacks, health)
    }

    fn build_routed(
        &self,
        config: &AppConfig,
        primary: ResolvedProviderProfile,
        fallback_targets: Vec<ResolvedProviderProfile>,
        health: Option<Arc<ModelHealthStore>>,
    ) -> anyhow::Result<Box<dyn ModelClient>> {
        let primary = self.build_target(primary)?;
        if fallback_targets.is_empty() {
            return Ok(primary);
        }

        let fallbacks = fallback_targets
            .into_iter()
            .map(|target| self.build_target(target))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let routed = match health {
            Some(health) => RoutingModelClient::with_health_store(primary, fallbacks, health),
            None => RoutingModelClient::new(primary, fallbacks).with_health_config(HealthConfig {
                failure_threshold: config.routing.failure_threshold,
                open_cooldown: Duration::from_millis(config.routing.open_cooldown_ms),
            }),
        };
        Ok(Box::new(routed.with_retry_policy(RetryPolicy {
            max_attempts: config.routing.retry_max_attempts,
            backoff_base: Duration::from_millis(config.routing.retry_backoff_base_ms),
            backoff_max: Duration::from_millis(config.routing.retry_backoff_max_ms),
        })))
    }

    fn build_target(
        &self,
        target: ResolvedProviderProfile,
    ) -> anyhow::Result<Box<dyn ModelClient>> {
        if target.protocol_id.as_str() == "fake" {
            return Ok(Box::new(FakeModelClient::new(format!(
                "fake response from {}",
                target.model
            ))));
        }
        if target.protocol_id.as_str() == "external-adapter-v1" {
            return build_external_adapter_target(target);
        }

        let protocol = self
            .registry
            .get(&target.protocol_id)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let namespace = protocol_client_namespace(&target.protocol_id);
        let client = ProviderClient::new(
            target.into_client_config(namespace),
            protocol,
            Arc::clone(&self.transport),
        )
        .map_err(|error| anyhow::anyhow!("provider target is invalid: {error}"))?;
        Ok(Box::new(client))
    }
}

pub fn try_build_model_client(
    config: &AppConfig,
    model_id: String,
) -> anyhow::Result<Box<dyn ModelClient>> {
    ModelClientFactory::native()?.try_build_model_client(config, model_id)
}

pub fn try_build_model_client_with_registry(
    config: &AppConfig,
    model_id: String,
    registry: Arc<WireProtocolRegistry>,
) -> anyhow::Result<Box<dyn ModelClient>> {
    ModelClientFactory::with_default_transport(registry)?.try_build_model_client(config, model_id)
}

pub fn try_build_model_client_with_health(
    config: &AppConfig,
    model_id: String,
    health: Arc<ModelHealthStore>,
) -> anyhow::Result<Box<dyn ModelClient>> {
    ModelClientFactory::native()?.try_build_model_client_with_health(config, model_id, health)
}

pub fn build_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    let error_model_id = model_id.clone();
    try_build_model_client(config, model_id)
        .unwrap_or_else(|error| invalid_configuration_client(error_model_id, error))
}

pub fn build_model_client_with_health(
    config: &AppConfig,
    model_id: String,
    health: Arc<ModelHealthStore>,
) -> Box<dyn ModelClient> {
    let error_model_id = model_id.clone();
    try_build_model_client_with_health(config, model_id, health)
        .unwrap_or_else(|error| invalid_configuration_client(error_model_id, error))
}

pub fn build_openai_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_legacy_model_client(config, LegacyProviderKind::OpenAiChat, model_id)
}

pub fn build_anthropic_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_legacy_model_client(config, LegacyProviderKind::Anthropic, model_id)
}

pub fn build_ollama_model_client(config: &AppConfig, model_id: String) -> Box<dyn ModelClient> {
    build_legacy_model_client(config, LegacyProviderKind::Ollama, model_id)
}

fn build_legacy_model_client(
    config: &AppConfig,
    kind: LegacyProviderKind,
    model_id: String,
) -> Box<dyn ModelClient> {
    let error_model_id = model_id.clone();
    let result = ModelClientFactory::native().and_then(|factory| {
        factory.try_build(config, PrimarySelection::Legacy(kind), model_id, None)
    });
    result.unwrap_or_else(|error| invalid_configuration_client(error_model_id, error))
}

fn named_targets(
    config: &AppConfig,
    model_id: &str,
) -> anyhow::Result<(ResolvedProviderProfile, Vec<ResolvedProviderProfile>)> {
    if !config.provider.fallback_providers.is_empty() {
        anyhow::bail!(
            "provider.fallback_providers cannot be combined with named profiles; use provider.fallback_profiles"
        );
    }
    let active = config
        .provider
        .active
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("provider.active is required when profiles exist"))?;
    let active_profile =
        config.provider.profiles.get(active).ok_or_else(|| {
            anyhow::anyhow!("provider.active references unknown profile `{active}`")
        })?;
    let primary = active_profile
        .resolve(
            &config.source_summary.workspace_root,
            config.state.allow_external_paths,
            Some(model_id),
        )
        .with_context(|| format!("provider profile `{active}` could not be resolved"))?;

    let mut fallbacks = config
        .provider
        .fallback_models
        .iter()
        .map(|model| {
            let mut target = primary.clone();
            target.model = model.trim().to_string();
            target
        })
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    for name in &config.provider.fallback_profiles {
        if name == active {
            anyhow::bail!("provider.fallback_profiles must not contain the active profile");
        }
        if !seen.insert(name) {
            anyhow::bail!("provider.fallback_profiles must not contain duplicates");
        }
        let profile = config.provider.profiles.get(name).ok_or_else(|| {
            anyhow::anyhow!("provider.fallback_profiles references unknown profile `{name}`")
        })?;
        fallbacks.push(
            profile
                .resolve(
                    &config.source_summary.workspace_root,
                    config.state.allow_external_paths,
                    None,
                )
                .with_context(|| format!("provider profile `{name}` could not be resolved"))?,
        );
    }
    Ok((primary, fallbacks))
}

fn legacy_targets(
    config: &AppConfig,
    forced_kind: Option<LegacyProviderKind>,
    model_id: String,
) -> anyhow::Result<(ResolvedProviderProfile, Vec<ResolvedProviderProfile>)> {
    if config.provider.active.is_some() || !config.provider.fallback_profiles.is_empty() {
        anyhow::bail!("named provider selection requires provider.profiles");
    }
    let primary_kind = forced_kind
        .map(Ok)
        .unwrap_or_else(|| LegacyProviderKind::from_name(&config.provider.name))?;
    let primary_key = if primary_kind == LegacyProviderKind::Anthropic
        && !config.provider.anthropic_api_key.is_empty()
    {
        config.provider.anthropic_api_key.clone()
    } else {
        config.provider.api_key.clone()
    };
    let primary = legacy_target(
        primary_kind,
        config.provider.api_base.clone(),
        primary_key,
        model_id,
        config.provider.options,
        config.provider.responses_prompt_cache,
        config.provider.responses_prompt_cache_retention.clone(),
    )?;

    let mut fallbacks = config
        .provider
        .fallback_models
        .iter()
        .map(|model| {
            let mut target = primary.clone();
            target.model = model.trim().to_string();
            target
        })
        .collect::<Vec<_>>();
    for fallback in &config.provider.fallback_providers {
        fallbacks.push(legacy_fallback_target(fallback)?);
    }
    Ok((primary, fallbacks))
}

fn legacy_fallback_target(
    fallback: &FallbackProviderConfig,
) -> anyhow::Result<ResolvedProviderProfile> {
    legacy_target(
        LegacyProviderKind::from_name(&fallback.name)?,
        fallback.api_base.clone(),
        fallback.api_key.clone(),
        fallback.model.clone(),
        fallback.options.unwrap_or_default(),
        false,
        None,
    )
}

fn legacy_target(
    kind: LegacyProviderKind,
    api_base: String,
    api_key: String,
    model: String,
    options: ProviderOptions,
    responses_prompt_cache: bool,
    responses_prompt_cache_retention: Option<String>,
) -> anyhow::Result<ResolvedProviderProfile> {
    let (protocol, base_url, auth, protocol_options) = match kind {
        LegacyProviderKind::OpenAiChat => (
            "openai-chat",
            api_base,
            legacy_bearer_auth(api_key)?,
            serde_json::json!({}),
        ),
        LegacyProviderKind::OpenAiResponses => {
            let mut protocol_options = serde_json::Map::new();
            protocol_options.insert(
                "prompt_cache_enabled".to_string(),
                serde_json::Value::Bool(responses_prompt_cache),
            );
            // Omit unset retention instead of serializing JSON null; the wire
            // protocol rejects non-string values for this option.
            if let Some(retention) = responses_prompt_cache_retention {
                protocol_options.insert(
                    "prompt_cache_retention".to_string(),
                    serde_json::Value::String(retention),
                );
            }
            (
                "openai-responses",
                api_base,
                legacy_bearer_auth(api_key)?,
                serde_json::Value::Object(protocol_options),
            )
        }
        LegacyProviderKind::Anthropic => (
            "anthropic-messages",
            legacy_anthropic_base(api_base),
            legacy_header_auth("x-api-key", api_key)?,
            serde_json::json!({}),
        ),
        LegacyProviderKind::Ollama => (
            "ollama-chat",
            legacy_ollama_base(api_base),
            ResolvedAuth::none(),
            serde_json::json!({}),
        ),
        LegacyProviderKind::Fake => (
            "fake",
            String::new(),
            ResolvedAuth::none(),
            serde_json::json!({}),
        ),
    };
    Ok(ResolvedProviderProfile {
        protocol_id: WireProtocolId::new(protocol)?,
        base_url: base_url.trim().trim_end_matches('/').to_string(),
        model: model.trim().to_string(),
        auth,
        headers: Vec::new(),
        options,
        protocol_options,
    })
}

fn legacy_bearer_auth(secret: String) -> anyhow::Result<ResolvedAuth> {
    if secret.trim().is_empty() {
        Ok(ResolvedAuth::none())
    } else {
        Ok(ResolvedAuth::bearer(secret)?)
    }
}

fn legacy_header_auth(name: &'static str, secret: String) -> anyhow::Result<ResolvedAuth> {
    if secret.trim().is_empty() {
        Ok(ResolvedAuth::none())
    } else {
        Ok(ResolvedAuth::header(HeaderName::from_static(name), secret)?)
    }
}

fn legacy_anthropic_base(api_base: String) -> String {
    let trimmed = api_base.trim();
    if trimmed.is_empty() || trimmed.contains("api.openai.com") {
        DEFAULT_ANTHROPIC_BASE.to_string()
    } else {
        trimmed.to_string()
    }
}

fn legacy_ollama_base(api_base: String) -> String {
    let trimmed = api_base.trim();
    if trimmed.is_empty() || trimmed.contains("openai") {
        DEFAULT_OLLAMA_BASE.to_string()
    } else {
        trimmed.to_string()
    }
}

#[derive(Clone, Copy)]
enum PrimarySelection {
    Configured,
    Legacy(LegacyProviderKind),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyProviderKind {
    OpenAiChat,
    OpenAiResponses,
    Anthropic,
    Ollama,
    Fake,
}

impl LegacyProviderKind {
    fn from_name(name: &str) -> anyhow::Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::OpenAiChat),
            "openai-responses" => Ok(Self::OpenAiResponses),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "fake" => Ok(Self::Fake),
            other => anyhow::bail!(
                "unknown provider `{other}`; expected openai, openai-responses, anthropic, ollama, or fake"
            ),
        }
    }
}

struct InvalidConfigurationModelClient {
    model_id: String,
    message: String,
}

impl ModelClient for InvalidConfigurationModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        Box::pin(stream::iter([Err(ModelError::InvalidConfiguration(
            self.message.clone(),
        ))]))
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("invalid-configuration", "local", &self.model_id)
    }
}

fn invalid_configuration_client(model_id: String, error: anyhow::Error) -> Box<dyn ModelClient> {
    Box::new(InvalidConfigurationModelClient {
        model_id,
        message: error.to_string(),
    })
}

fn build_external_adapter_target(
    target: ResolvedProviderProfile,
) -> anyhow::Result<Box<dyn ModelClient>> {
    // External adapters do not use the shared HTTP Transport. They receive a
    // direct argv, optional workspace-bounded working directory, and secrets
    // already resolved by profile assembly.
    let config = ExternalAdapterConfig::from_protocol_options(
        &target.protocol_options,
        target.base_url,
        target.model,
        target.auth,
        target.headers,
        target.options,
        // Workspace root is not on ResolvedProviderProfile; path validation was
        // already performed at profile resolve/config load time when relative
        // paths were constrained. Re-validate with the process CWD and allow
        // absolute paths only if they were already accepted.
        std::path::Path::new("."),
        true,
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let client =
        ExternalAdapterClient::new(config).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(Box::new(client))
}
