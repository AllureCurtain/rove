use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::Context;
use futures::stream::{self, BoxStream};
use rove_models::fake::FakeModelClient;
use rove_models::health::{HealthConfig, ModelHealthStore};
use rove_models::provider::{
    ExternalAdapterClient, ExternalAdapterConfig, ProviderClient, Transport, TransportConfig,
    WireProtocolRegistry,
};
use rove_models::routing::{RetryPolicy, RoutingModelClient};
use rove_models::{Message, ModelClient, ModelClientId, ModelError, ModelEvent, ToolSchema};

use crate::config::AppConfig;
use crate::provider::{
    ResolvedProviderProfile, default_wire_protocol_registry, protocol_client_namespace,
};

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
        self.try_build(config, model_id, None)
    }

    pub fn try_build_model_client_with_health(
        &self,
        config: &AppConfig,
        model_id: String,
        health: Arc<ModelHealthStore>,
    ) -> anyhow::Result<Box<dyn ModelClient>> {
        self.try_build(config, model_id, Some(health))
    }

    fn try_build(
        &self,
        config: &AppConfig,
        model_id: String,
        health: Option<Arc<ModelHealthStore>>,
    ) -> anyhow::Result<Box<dyn ModelClient>> {
        let (primary, fallbacks) = named_targets(config, &model_id)?;
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

fn named_targets(
    config: &AppConfig,
    model_id: &str,
) -> anyhow::Result<(ResolvedProviderProfile, Vec<ResolvedProviderProfile>)> {
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
        .map_err(|error| {
            anyhow::anyhow!("provider profile `{active}` could not be resolved: {error}")
        })?;

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
                .map_err(|error| {
                    anyhow::anyhow!("provider profile `{name}` could not be resolved: {error}")
                })?,
        );
    }
    Ok((primary, fallbacks))
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
