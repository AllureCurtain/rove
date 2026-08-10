use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock, Weak};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::environment::{
    EnvironmentError, ExecutionEnvironment, StdioProcessGuard, local_environment,
};
use crate::state::tool_artifacts::ToolArtifactStore;
use crate::tools::mcp::catalog::{
    CatalogBuilder, McpCatalogSnapshot, McpToolAnnotations, mcp_server_namespace,
    parse_catalog_page,
};
use crate::tools::mcp::protocol::{negotiate_protocol_version, server_identity_hash};
use crate::workspace::Workspace;

use rove_core::ToolDescriptor;
use rove_core::{
    Tool, ToolContext, ToolError, ToolOutput, ToolRegistry, ToolRegistryPublisher,
    ToolRegistryReplacement,
};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_MCP_REQUEST_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MCP_STDERR_CAPTURE_BYTES: usize = 16 * 1024;
const MAX_MCP_CONFIG_BYTES: usize = 256 * 1024;
pub const MAX_MCP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_MCP_ENDPOINT_BYTES: usize = 2_048;
const MCP_REFRESH_BACKOFF_SECONDS: u64 = 1;
const MCP_REFRESH_CIRCUIT_BACKOFF_SECONDS: u64 = 30;
const MCP_REFRESH_CIRCUIT_FAILURES: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default = "default_mcp_server_enabled")]
    pub enabled: bool,
    /// A required server blocks runtime activation when it cannot provide a
    /// complete validated catalog. Optional servers degrade explicitly.
    #[serde(default = "default_mcp_server_required")]
    pub required: bool,
    #[serde(default)]
    pub transport: McpTransport,
    /// Command to spawn (stdio transport only).
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_names: Vec<String>,
    /// SSE endpoint URL (sse transport only).
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub policy: McpTransportPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    #[default]
    Stdio,
    /// Deprecated HTTP+SSE transport. Kept for existing configurations and
    /// deliberately distinct from `streamable_http`, whose session, DELETE, and
    /// POST-SSE abilities it does not have.
    Sse,
    /// Current MCP HTTP transport: POST JSON or SSE, negotiated session and
    /// protocol version, optional GET stream and DELETE.
    StreamableHttp,
}

impl McpTransport {
    /// True for a transport retained only for compatibility.
    pub fn is_deprecated(self) -> bool {
        matches!(self, Self::Sse)
    }

    /// True when the transport is reached over HTTP.
    pub fn is_http(self) -> bool {
        matches!(self, Self::Sse | Self::StreamableHttp)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpTransportPolicy {
    #[serde(default = "default_mcp_request_timeout_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_mcp_stderr_capture_bytes")]
    pub stderr_capture_bytes: usize,
}

impl Default for McpTransportPolicy {
    fn default() -> Self {
        Self {
            request_timeout_ms: DEFAULT_MCP_REQUEST_TIMEOUT_MS,
            stderr_capture_bytes: DEFAULT_MCP_STDERR_CAPTURE_BYTES,
        }
    }
}

fn default_mcp_request_timeout_ms() -> u64 {
    DEFAULT_MCP_REQUEST_TIMEOUT_MS
}

fn default_mcp_server_enabled() -> bool {
    true
}

fn default_mcp_server_required() -> bool {
    true
}

fn default_mcp_stderr_capture_bytes() -> usize {
    DEFAULT_MCP_STDERR_CAPTURE_BYTES
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfigFile {
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub server_name: String,
    pub remote_name: String,
    pub schema: ToolDescriptor,
    pub output_schema: Option<Value>,
    pub annotations: McpToolAnnotations,
    pub catalog_hash: String,
    pub protocol_version: String,
    pub server_identity_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerHealthStatus {
    Ready,
    Degraded,
    Disabled,
}

/// Secret-free identity and health facts for one configured MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerRuntimeSnapshot {
    pub server_config_id: String,
    pub server_config_hash: String,
    pub required: bool,
    pub transport: McpTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    pub server_identity_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot_id: Option<String>,
    pub tool_count: usize,
    pub status: McpServerHealthStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    pub refreshed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpLifecycleFact {
    Degraded {
        server_config_id: String,
        failure_code: String,
    },
    CapabilitiesRefreshed {
        server_config_id: String,
        snapshot_id: String,
        added: Vec<String>,
        removed: Vec<String>,
        changed: Vec<String>,
    },
}

/// Registry-scoped MCP diagnostics and refresh lifecycle authority.
#[derive(Default)]
pub struct McpRuntimeState {
    servers: StdRwLock<BTreeMap<String, McpServerRuntimeSnapshot>>,
    facts: StdMutex<Vec<McpLifecycleFact>>,
    cancel: CancellationToken,
}

impl McpRuntimeState {
    pub fn snapshots(&self) -> Vec<McpServerRuntimeSnapshot> {
        self.servers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    pub fn snapshot(&self, server_config_id: &str) -> Option<McpServerRuntimeSnapshot> {
        self.servers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(server_config_id)
            .cloned()
    }

    pub fn take_facts(&self) -> Vec<McpLifecycleFact> {
        std::mem::take(
            &mut *self
                .facts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    fn publish(&self, snapshot: McpServerRuntimeSnapshot) {
        self.servers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(snapshot.server_config_id.clone(), snapshot);
    }

    fn push_fact(&self, fact: McpLifecycleFact) {
        self.facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(fact);
    }
}

impl Drop for McpRuntimeState {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpProbeFailureKind {
    EnvironmentMissing,
    Spawn,
    Timeout,
    Transport,
    Protocol,
    NoTools,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpProbeFailure {
    pub kind: McpProbeFailureKind,
}

pub async fn register_mcp_tools_from_file(
    registry: &mut ToolRegistry,
    path: impl Into<PathBuf>,
) -> anyhow::Result<usize> {
    let workspace = Workspace::detect(&std::env::current_dir()?)?;
    register_mcp_tools_from_file_with_environment(registry, path, local_environment(&workspace))
        .await
}

pub async fn register_mcp_tools_from_file_with_environment(
    registry: &mut ToolRegistry,
    path: impl Into<PathBuf>,
    environment: Arc<dyn ExecutionEnvironment>,
) -> anyhow::Result<usize> {
    let path = path.into();
    if !environment.capabilities().filesystem_read {
        anyhow::bail!("execution capability unavailable: filesystem_read");
    }
    let relative_path = environment_relative_path(environment.as_ref(), &path)?;
    let read = match environment
        .filesystem()
        .read_relative_bytes(&relative_path, MAX_MCP_CONFIG_BYTES)
        .await
    {
        Ok(read) => read,
        Err(EnvironmentError::NotFound) => return Ok(0),
        Err(error) => return Err(anyhow::anyhow!(error.to_string())),
    };
    if read.truncated {
        anyhow::bail!("MCP config exceeds the supported size");
    }
    let config: McpConfigFile = serde_json::from_slice(&read.bytes)?;
    register_mcp_tools_with_environment(registry, config.servers, environment).await
}

fn environment_relative_path(
    environment: &dyn ExecutionEnvironment,
    path: &Path,
) -> anyhow::Result<String> {
    let relative = if path.is_absolute() {
        path.strip_prefix(environment.filesystem().root())
            .map_err(|_| anyhow::anyhow!("MCP config path is outside the execution workspace"))?
    } else {
        path
    };
    let relative = relative.to_string_lossy().replace('\\', "/");
    if relative.is_empty() {
        anyhow::bail!("MCP config path must name a workspace file");
    }
    Ok(relative)
}

pub async fn register_mcp_tools(
    registry: &mut ToolRegistry,
    servers: Vec<McpServerConfig>,
) -> anyhow::Result<usize> {
    let workspace = Workspace::detect(&std::env::current_dir()?)?;
    register_mcp_tools_with_environment(registry, servers, local_environment(&workspace)).await
}

pub async fn register_mcp_tools_with_environment(
    registry: &mut ToolRegistry,
    servers: Vec<McpServerConfig>,
    environment: Arc<dyn ExecutionEnvironment>,
) -> anyhow::Result<usize> {
    let runtime_state = Arc::new(McpRuntimeState::default());
    registry.attach_extension(Arc::clone(&runtime_state));
    let publisher = registry.publisher();
    let mut registered = 0_usize;

    for configured in servers {
        if !configured.enabled {
            runtime_state.publish(server_runtime_snapshot(
                &configured,
                None,
                McpServerHealthStatus::Disabled,
                None,
            ));
            continue;
        }
        let activation = activate_mcp_server(configured.clone(), Arc::clone(&environment))
            .await
            .and_then(|activation| {
                registry
                    .try_register_batch(activation.tools)
                    .map(|count| (count, activation.catalog, activation.refresh_client))
                    .map_err(anyhow::Error::from)
            });

        match activation {
            Ok((count, catalog, refresh_client)) => {
                registered = registered.saturating_add(count);
                runtime_state.publish(server_runtime_snapshot(
                    &configured,
                    Some(&catalog),
                    McpServerHealthStatus::Ready,
                    None,
                ));
                if let Some(client) = refresh_client {
                    spawn_streamable_catalog_controller(
                        client,
                        configured,
                        publisher.clone(),
                        Arc::downgrade(&runtime_state),
                    );
                }
            }
            Err(error) => {
                let failure_code = activation_failure_code(&error).to_string();
                runtime_state.publish(server_runtime_snapshot(
                    &configured,
                    None,
                    McpServerHealthStatus::Degraded,
                    Some(failure_code.clone()),
                ));
                runtime_state.push_fact(McpLifecycleFact::Degraded {
                    server_config_id: configured.name.clone(),
                    failure_code: failure_code.clone(),
                });
                if configured.required {
                    anyhow::bail!(
                        "required MCP server `{}` failed activation ({failure_code})",
                        configured.name
                    );
                }
            }
        }
    }
    Ok(registered)
}

struct ActivatedMcpServer {
    tools: Vec<Box<dyn Tool>>,
    catalog: McpCatalogSnapshot,
    refresh_client: Option<Arc<crate::tools::mcp::client::StreamableHttpClient>>,
}

async fn activate_mcp_server(
    server: McpServerConfig,
    environment: Arc<dyn ExecutionEnvironment>,
) -> anyhow::Result<ActivatedMcpServer> {
    let server = resolve_mcp_server_environment(server)?;
    match server.transport {
        McpTransport::Stdio => {
            if !environment.capabilities().process_stdio {
                anyhow::bail!("execution capability unavailable: process_stdio");
            }
            let client = Arc::new(
                StdioMcpClient::connect_with_environment(server, Arc::clone(&environment)).await?,
            );
            let infos = client.list_tools().await?;
            let catalog = catalog_from_tool_infos(&infos)?;
            let tools = infos
                .into_iter()
                .map(|tool| {
                    Box::new(McpProxyTool {
                        client: client.clone(),
                        tool,
                    }) as Box<dyn Tool>
                })
                .collect();
            Ok(ActivatedMcpServer {
                tools,
                catalog,
                refresh_client: None,
            })
        }
        McpTransport::Sse => {
            let client = Arc::new(SseMcpClient::connect(server).await?);
            let infos = client.list_tools().await?;
            let catalog = catalog_from_tool_infos(&infos)?;
            let tools = infos
                .into_iter()
                .map(|tool| {
                    Box::new(McpSseProxyTool {
                        client: client.clone(),
                        tool,
                    }) as Box<dyn Tool>
                })
                .collect();
            Ok(ActivatedMcpServer {
                tools,
                catalog,
                refresh_client: None,
            })
        }
        McpTransport::StreamableHttp => {
            let client = Arc::new(connect_streamable_http(&server).await?);
            let catalog = client
                .discover_catalog()
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let tools = streamable_http_proxy_tools(Arc::clone(&client), &catalog);
            Ok(ActivatedMcpServer {
                tools,
                catalog,
                refresh_client: Some(client),
            })
        }
    }
}

fn catalog_from_tool_infos(infos: &[McpToolInfo]) -> anyhow::Result<McpCatalogSnapshot> {
    let first = infos
        .first()
        .ok_or_else(|| anyhow::anyhow!("MCP server exposed no usable tools"))?;
    Ok(McpCatalogSnapshot {
        server_name: first.server_name.clone(),
        protocol_version: first.protocol_version.clone(),
        server_identity_hash: first.server_identity_hash.clone(),
        entries: infos
            .iter()
            .map(|tool| crate::tools::mcp::catalog::McpCatalogEntry {
                remote_name: tool.remote_name.clone(),
                local_name: tool.schema.name.clone(),
                title: None,
                description: tool.schema.description.clone(),
                parameters: tool.schema.parameters.clone(),
                output_schema: tool.output_schema.clone(),
                annotations: tool.annotations.clone(),
                capability_id: tool.schema.capability_id.clone().unwrap_or_default(),
                raw_descriptor_hash: crate::prompt_metadata::stable_hash(
                    &serde_json::to_string(&tool.schema).unwrap_or_default(),
                ),
            })
            .collect(),
        catalog_hash: first.catalog_hash.clone(),
    })
}

fn streamable_http_proxy_tools(
    client: Arc<crate::tools::mcp::client::StreamableHttpClient>,
    catalog: &McpCatalogSnapshot,
) -> Vec<Box<dyn Tool>> {
    tool_infos_from_catalog(catalog)
        .into_iter()
        .map(|tool| {
            Box::new(McpStreamableHttpProxyTool {
                client: Arc::clone(&client),
                tool,
            }) as Box<dyn Tool>
        })
        .collect()
}

fn spawn_streamable_catalog_controller(
    client: Arc<crate::tools::mcp::client::StreamableHttpClient>,
    server: McpServerConfig,
    publisher: ToolRegistryPublisher,
    state: Weak<McpRuntimeState>,
) {
    tokio::spawn(async move {
        let mut consecutive_failures = 0_u32;
        loop {
            let Some(runtime_state) = state.upgrade() else {
                let _ = client.close().await;
                return;
            };
            let cancel = runtime_state.cancel.clone();
            drop(runtime_state);
            let poll = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = client.close().await;
                    return;
                }
                result = client.poll_notifications() => result,
            };

            match poll {
                Ok(_) => {
                    if client.tools_changed().await {
                        if refresh_streamable_catalog(&client, &server, &publisher, &state).await {
                            consecutive_failures = 0;
                        } else {
                            consecutive_failures = consecutive_failures.saturating_add(1);
                        }
                        // Notifications observed while discovering describe the
                        // snapshot just fetched and need no second refresh.
                        let _ = client.take_events().await;
                    } else {
                        consecutive_failures = 0;
                        mark_mcp_notification_stream_ready(&state, &server.name);
                    }
                }
                Err(_) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    mark_mcp_degraded(&state, &server.name, "mcp_notification_poll_failed");
                }
            }

            let backoff = mcp_refresh_backoff(consecutive_failures);
            tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = client.close().await;
                    return;
                }
                _ = tokio::time::sleep(backoff) => {}
            }
        }
    });
}

fn mcp_refresh_backoff(consecutive_failures: u32) -> Duration {
    Duration::from_secs(if consecutive_failures >= MCP_REFRESH_CIRCUIT_FAILURES {
        MCP_REFRESH_CIRCUIT_BACKOFF_SECONDS
    } else {
        MCP_REFRESH_BACKOFF_SECONDS
    })
}

async fn refresh_streamable_catalog(
    client: &Arc<crate::tools::mcp::client::StreamableHttpClient>,
    server: &McpServerConfig,
    publisher: &ToolRegistryPublisher,
    state: &Weak<McpRuntimeState>,
) -> bool {
    let catalog = match client.discover_catalog().await {
        Ok(catalog) => catalog,
        Err(_) => {
            mark_mcp_degraded(state, &server.name, "mcp_catalog_refresh_failed");
            return false;
        }
    };
    let tools = streamable_http_proxy_tools(Arc::clone(client), &catalog);
    let namespace = mcp_server_namespace(&server.name);
    let replacement = match publisher.try_replace_prefix(&namespace, tools) {
        Some(Ok(replacement)) => replacement,
        Some(Err(_)) => {
            mark_mcp_degraded(state, &server.name, "mcp_catalog_publish_failed");
            return false;
        }
        None => return false,
    };
    publish_refreshed_snapshot(state, server, &catalog, replacement);
    true
}

fn publish_refreshed_snapshot(
    state: &Weak<McpRuntimeState>,
    server: &McpServerConfig,
    catalog: &McpCatalogSnapshot,
    replacement: ToolRegistryReplacement,
) {
    let Some(state) = state.upgrade() else {
        return;
    };
    state.publish(server_runtime_snapshot(
        server,
        Some(catalog),
        McpServerHealthStatus::Ready,
        None,
    ));
    state.push_fact(McpLifecycleFact::CapabilitiesRefreshed {
        server_config_id: server.name.clone(),
        snapshot_id: catalog.catalog_hash.clone(),
        added: replacement.added,
        removed: replacement.removed,
        changed: replacement.changed,
    });
}

fn mark_mcp_degraded(state: &Weak<McpRuntimeState>, server_name: &str, failure_code: &str) {
    let Some(state) = state.upgrade() else {
        return;
    };
    if let Some(mut snapshot) = state.snapshot(server_name) {
        snapshot.status = McpServerHealthStatus::Degraded;
        snapshot.failure_code = Some(failure_code.to_string());
        snapshot.refreshed_at = chrono::Utc::now().to_rfc3339();
        state.publish(snapshot);
    }
    state.push_fact(McpLifecycleFact::Degraded {
        server_config_id: server_name.to_string(),
        failure_code: failure_code.to_string(),
    });
}

fn mark_mcp_notification_stream_ready(state: &Weak<McpRuntimeState>, server_name: &str) {
    let Some(state) = state.upgrade() else {
        return;
    };
    let Some(mut snapshot) = state.snapshot(server_name) else {
        return;
    };
    if snapshot.failure_code.as_deref() != Some("mcp_notification_poll_failed") {
        return;
    }
    snapshot.status = McpServerHealthStatus::Ready;
    snapshot.failure_code = None;
    snapshot.refreshed_at = chrono::Utc::now().to_rfc3339();
    state.publish(snapshot);
}

fn server_runtime_snapshot(
    server: &McpServerConfig,
    catalog: Option<&McpCatalogSnapshot>,
    status: McpServerHealthStatus,
    failure_code: Option<String>,
) -> McpServerRuntimeSnapshot {
    let catalog_hash = catalog.map(|catalog| catalog.catalog_hash.clone());
    McpServerRuntimeSnapshot {
        server_config_id: server.name.clone(),
        server_config_hash: safe_server_config_hash(server),
        required: server.required,
        transport: server.transport,
        protocol_version: catalog.map(|catalog| catalog.protocol_version.clone()),
        server_identity_hash: catalog
            .map(|catalog| catalog.server_identity_hash.clone())
            .unwrap_or_else(|| configured_server_identity_hash(server)),
        capability_snapshot_id: catalog_hash.clone(),
        catalog_hash,
        tool_count: catalog.map(McpCatalogSnapshot::tool_count).unwrap_or(0),
        status,
        failure_code,
        refreshed_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn configured_server_identity_hash(server: &McpServerConfig) -> String {
    crate::prompt_metadata::stable_hash(&format!(
        "mcp-server-config:v1:{}:{:?}",
        server.name, server.transport
    ))
}

fn safe_server_config_hash(server: &McpServerConfig) -> String {
    let endpoint = reqwest::Url::parse(&server.url).ok().map(|url| {
        format!(
            "{}://{}{}{}",
            url.scheme(),
            url.host_str().unwrap_or_default(),
            url.port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default(),
            url.path()
        )
    });
    let mut env_names = server.env.keys().cloned().collect::<Vec<_>>();
    env_names.extend(server.env_names.clone());
    env_names.sort();
    env_names.dedup();
    crate::prompt_metadata::stable_hash(
        &serde_json::json!({
            "name": server.name,
            "enabled": server.enabled,
            "required": server.required,
            "transport": server.transport,
            "command_hash": crate::prompt_metadata::stable_hash(&server.command),
            "args_hash": crate::prompt_metadata::stable_hash(&serde_json::to_string(&server.args).unwrap_or_default()),
            "env_names": env_names,
            "endpoint": endpoint,
            "policy": server.policy,
        })
        .to_string(),
    )
}

fn activation_failure_code(error: &anyhow::Error) -> &'static str {
    let message = error.to_string();
    if message.contains("environment variable") {
        "mcp_environment_missing"
    } else if message.contains("timed out") {
        "mcp_activation_timeout"
    } else if message.contains("must use https") {
        "mcp_transport_policy_blocked"
    } else if message.contains("schema") || message.contains("catalog") || message.contains("tool")
    {
        "mcp_catalog_invalid"
    } else if message.contains("capability unavailable") || message.contains("spawn") {
        "mcp_activation_unavailable"
    } else {
        "mcp_activation_failed"
    }
}

pub fn resolve_mcp_server_environment(
    mut server: McpServerConfig,
) -> anyhow::Result<McpServerConfig> {
    for name in &server.env_names {
        if !is_valid_environment_name(name) {
            anyhow::bail!("MCP environment variable name is invalid");
        }
        let value = std::env::var(name)
            .map_err(|_| anyhow::anyhow!("MCP environment variable `{name}` is unavailable"))?;
        server.env.insert(name.clone(), value);
    }
    Ok(server)
}

pub async fn probe_mcp_server(
    server: McpServerConfig,
) -> Result<Vec<McpToolInfo>, McpProbeFailure> {
    let cwd = std::env::current_dir().map_err(|_| McpProbeFailure {
        kind: McpProbeFailureKind::Spawn,
    })?;
    let workspace = Workspace::detect(&cwd).map_err(|_| McpProbeFailure {
        kind: McpProbeFailureKind::Spawn,
    })?;
    probe_mcp_server_with_environment(server, local_environment(&workspace)).await
}

pub async fn probe_mcp_server_with_environment(
    server: McpServerConfig,
    environment: Arc<dyn ExecutionEnvironment>,
) -> Result<Vec<McpToolInfo>, McpProbeFailure> {
    let timeout_ms = server.policy.request_timeout_ms;
    let server = resolve_mcp_server_environment(server).map_err(|_| McpProbeFailure {
        kind: McpProbeFailureKind::EnvironmentMissing,
    })?;
    let probe = async move {
        let tools = match server.transport {
            McpTransport::Stdio => {
                if !environment.capabilities().process_stdio {
                    return Err(McpProbeFailure {
                        kind: McpProbeFailureKind::Spawn,
                    });
                }
                let mut client = StdioMcpClient::spawn_with_environment(server, environment)
                    .await
                    .map_err(|_| McpProbeFailure {
                        kind: McpProbeFailureKind::Spawn,
                    })?;
                client
                    .initialize()
                    .await
                    .map_err(classify_mcp_probe_error)?;
                client
                    .list_tools()
                    .await
                    .map_err(classify_mcp_probe_error)?
            }
            McpTransport::Sse => {
                let client = SseMcpClient::connect(server)
                    .await
                    .map_err(classify_mcp_probe_error)?;
                client
                    .list_tools()
                    .await
                    .map_err(classify_mcp_probe_error)?
            }
            McpTransport::StreamableHttp => {
                let client = connect_streamable_http(&server)
                    .await
                    .map_err(classify_mcp_probe_error)?;
                let tools = streamable_http_tool_infos(&client)
                    .await
                    .map_err(classify_mcp_probe_error)?;
                // A probe is not a session; release the server session so a
                // probed server is not left holding state.
                let _ = client.close().await;
                tools
            }
        };
        if tools.is_empty() {
            return Err(McpProbeFailure {
                kind: McpProbeFailureKind::NoTools,
            });
        }
        Ok(tools)
    };
    timeout(Duration::from_millis(timeout_ms), probe)
        .await
        .map_err(|_| McpProbeFailure {
            kind: McpProbeFailureKind::Timeout,
        })?
}

fn classify_mcp_probe_error(error: anyhow::Error) -> McpProbeFailure {
    let message = error.to_string();
    let kind = if message.contains("exposed no usable tools") {
        McpProbeFailureKind::NoTools
    } else if message.contains("timed out") {
        McpProbeFailureKind::Timeout
    } else if error.chain().any(|source| {
        source
            .downcast_ref::<crate::tools::mcp::protocol::McpProtocolError>()
            .is_some()
            || source.downcast_ref::<serde_json::Error>().is_some()
            || source
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_decode)
    }) || message.contains("JSON-RPC")
        || message.contains("missing tools array")
        || message.contains("missing name")
        || message.contains("empty name")
        || message.contains("too many tools")
        || message.contains("exceeds the supported size")
        || message.contains("MCP SSE endpoint is invalid")
        || message.contains("without providing an endpoint")
    {
        McpProbeFailureKind::Protocol
    } else {
        McpProbeFailureKind::Transport
    };
    McpProbeFailure { kind }
}

fn is_valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

pub struct McpProxyTool {
    client: Arc<StdioMcpClient>,
    tool: McpToolInfo,
}

#[async_trait]
impl Tool for McpProxyTool {
    fn schema(&self) -> ToolDescriptor {
        self.tool.schema.clone()
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let services = crate::tools::runtime_context::runtime_tool_services(ctx).ok();
        let artifacts = services.and_then(|services| services.tool_artifacts.as_deref());
        let started = std::time::Instant::now();
        let result = self
            .client
            .call_tool(&self.tool.remote_name, args)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                reason: err.to_string(),
            })?;
        let duration_ms = elapsed_millis(started);
        let context = mcp_result_context(&self.tool, ctx, artifacts, duration_ms, None);
        Ok(ToolOutput::from_envelope(
            crate::tools::mcp::result_mapping::envelope_from_mcp_result(&result, &context).await,
        ))
    }
}

pub struct StdioMcpClient {
    server_name: String,
    server_identity_hash: String,
    protocol_version: String,
    next_id: AtomicU64,
    policy: McpTransportPolicy,
    _child: StdMutex<Option<StdioProcessGuard>>,
    transport: Mutex<StdioTransport>,
    stderr: Arc<Mutex<StderrCapture>>,
}

impl StdioMcpClient {
    pub async fn connect(config: McpServerConfig) -> anyhow::Result<Self> {
        let workspace = Workspace::detect(&std::env::current_dir()?)?;
        Self::connect_with_environment(config, local_environment(&workspace)).await
    }

    pub async fn connect_with_environment(
        config: McpServerConfig,
        environment: Arc<dyn ExecutionEnvironment>,
    ) -> anyhow::Result<Self> {
        let mut client = Self::spawn_with_environment(config, environment).await?;
        client.initialize().await?;
        Ok(client)
    }

    async fn spawn_with_environment(
        config: McpServerConfig,
        environment: Arc<dyn ExecutionEnvironment>,
    ) -> anyhow::Result<Self> {
        let process = environment
            .processes()
            .spawn_stdio(
                &config.command,
                &config.args,
                &config
                    .env
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Vec<_>>(),
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let (stdin, stdout, stderr, child) = process.into_parts();
        let stderr_capture = Arc::new(Mutex::new(StderrCapture::new(
            config.policy.stderr_capture_bytes,
        )));
        spawn_stderr_capture(stderr, stderr_capture.clone());

        Ok(Self {
            server_identity_hash: configured_server_identity_hash(&config),
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            server_name: config.name,
            next_id: AtomicU64::new(1),
            policy: config.policy,
            _child: StdMutex::new(Some(child)),
            transport: Mutex::new(StdioTransport {
                stdin,
                stdout: BufReader::new(stdout),
            }),
            stderr: stderr_capture,
        })
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "rove",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        self.protocol_version =
            negotiate_protocol_version(result.get("protocolVersion").and_then(Value::as_str))?;
        self.server_identity_hash = server_identity_hash(&result)?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpToolInfo>> {
        Ok(tool_infos_from_catalog(&self.discover_catalog().await?))
    }

    async fn discover_catalog(&self) -> anyhow::Result<McpCatalogSnapshot> {
        let mut builder =
            CatalogBuilder::new(self.server_name.clone(), self.protocol_version.clone())
                .with_server_identity_hash(self.server_identity_hash.clone());
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let page = parse_catalog_page(
                &self.server_name,
                &self.request("tools/list", params).await?,
            )?;
            match builder.push_page(page)? {
                Some(next) => cursor = Some(next),
                None => return Ok(builder.finish()?),
            }
        }
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments,
            }),
        )
        .await
    }

    async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut transport = self.transport.lock().await;
        transport.write_message(&request).await?;
        let read = timeout(
            Duration::from_millis(self.policy.request_timeout_ms),
            transport.read_response(id),
        )
        .await;
        match read {
            Ok(result) => result,
            Err(_) => {
                let stderr = self.stderr.lock().await.snapshot();
                anyhow::bail!(
                    "{}",
                    format_timeout_error(self.policy.request_timeout_ms, stderr)
                );
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.transport
            .lock()
            .await
            .write_message(&notification)
            .await
    }
}

struct StdioTransport {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl StdioTransport {
    async fn write_message(&mut self, message: &Value) -> anyhow::Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        self.stdin.write_all(&line).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_response(&mut self, id: u64) -> anyhow::Result<Value> {
        while let Some(line) = read_bounded_json_line(&mut self.stdout).await? {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let message: Value = serde_json::from_slice(&line)?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                anyhow::bail!("{}", format_mcp_error(error));
            }
            return Ok(message.get("result").cloned().unwrap_or_else(|| json!({})));
        }
        anyhow::bail!("MCP server closed stdout before responding to request {id}");
    }
}

async fn read_bounded_json_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> anyhow::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(consumed) > MAX_MCP_RESPONSE_BYTES {
            anyhow::bail!("MCP stdio response exceeds the supported size");
        }
        line.extend_from_slice(&available[..consumed]);
        reader.consume(consumed);
        if newline.is_some() {
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            return Ok(Some(line));
        }
    }
}

// --- SSE Transport ---

pub struct SseMcpClient {
    server_name: String,
    server_identity_hash: String,
    protocol_version: String,
    http: reqwest::Client,
    endpoint: String,
    next_id: AtomicU64,
    policy: McpTransportPolicy,
}

impl SseMcpClient {
    pub async fn connect(config: McpServerConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.policy.request_timeout_ms))
            .build()?;
        let endpoint = discover_endpoint(&http, &config.url).await?;

        let mut client = Self {
            server_identity_hash: configured_server_identity_hash(&config),
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            server_name: config.name,
            http,
            endpoint,
            next_id: AtomicU64::new(1),
            policy: config.policy,
        };
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        let result = self
            .request(
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "rove",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;
        self.protocol_version =
            negotiate_protocol_version(result.get("protocolVersion").and_then(Value::as_str))?;
        self.server_identity_hash = server_identity_hash(&result)?;
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpToolInfo>> {
        Ok(tool_infos_from_catalog(&self.discover_catalog().await?))
    }

    async fn discover_catalog(&self) -> anyhow::Result<McpCatalogSnapshot> {
        let mut builder =
            CatalogBuilder::new(self.server_name.clone(), self.protocol_version.clone())
                .with_server_identity_hash(self.server_identity_hash.clone());
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let page = parse_catalog_page(
                &self.server_name,
                &self.request("tools/list", params).await?,
            )?;
            match builder.push_page(page)? {
                Some(next) => cursor = Some(next),
                None => return Ok(builder.finish()?),
            }
        }
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> anyhow::Result<Value> {
        self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments,
            }),
        )
        .await
    }

    async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let response = timeout(
            Duration::from_millis(self.policy.request_timeout_ms),
            async {
                self.http
                    .post(&self.endpoint)
                    .header("content-type", "application/json")
                    .json(&request)
                    .send()
                    .await
            },
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "MCP request timed out after {}ms",
                self.policy.request_timeout_ms
            )
        })??;

        let status = response.status();
        let bytes = read_bounded_mcp_response(response).await?;
        if !status.is_success() {
            let text = String::from_utf8_lossy(&bytes);
            anyhow::bail!("MCP SSE request failed: {text}");
        }

        let body: Value = serde_json::from_slice(&bytes)?;
        if let Some(error) = body.get("error") {
            anyhow::bail!("{}", format_mcp_error(error));
        }
        Ok(body.get("result").cloned().unwrap_or_else(|| json!({})))
    }

    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.http
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .json(&notification)
            .send()
            .await?;
        Ok(())
    }
}

async fn discover_endpoint(http: &reqwest::Client, sse_url: &str) -> anyhow::Result<String> {
    let response = http.get(sse_url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("MCP SSE endpoint returned HTTP {}", response.status());
    }

    if response
        .content_length()
        .is_some_and(|length| length > MAX_MCP_RESPONSE_BYTES as u64)
    {
        anyhow::bail!("MCP SSE response exceeds the supported size");
    }

    use futures::StreamExt;
    let mut byte_stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut received = 0_usize;

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        received = received.saturating_add(chunk.len());
        if received > MAX_MCP_RESPONSE_BYTES {
            anyhow::bail!("MCP SSE response exceeds the supported size");
        }
        buffer.extend_from_slice(&chunk);

        while let Some(line_end) = buffer.iter().position(|byte| *byte == b'\n') {
            let remaining = buffer.split_off(line_end + 1);
            let line = String::from_utf8_lossy(&buffer[..line_end])
                .trim()
                .to_string();
            buffer = remaining;

            if let Some(data) = line.strip_prefix("data:") {
                return resolve_sse_endpoint(sse_url, data.trim());
            }
        }
    }

    anyhow::bail!("MCP SSE stream closed without providing an endpoint URL")
}

fn resolve_sse_endpoint(sse_url: &str, endpoint: &str) -> anyhow::Result<String> {
    if endpoint.is_empty() || endpoint.len() > MAX_MCP_ENDPOINT_BYTES {
        anyhow::bail!("MCP SSE endpoint is empty or exceeds the supported size");
    }
    let base = reqwest::Url::parse(sse_url)?;
    let resolved = base.join(endpoint)?;
    if !matches!(resolved.scheme(), "http" | "https")
        || !resolved.username().is_empty()
        || resolved.password().is_some()
        || resolved.fragment().is_some()
    {
        anyhow::bail!("MCP SSE endpoint is invalid");
    }
    Ok(resolved.to_string())
}

async fn read_bounded_mcp_response(response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MCP_RESPONSE_BYTES as u64)
    {
        anyhow::bail!("MCP SSE response exceeds the supported size");
    }
    use futures::StreamExt;
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_MCP_RESPONSE_BYTES {
            anyhow::bail!("MCP SSE response exceeds the supported size");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub struct McpSseProxyTool {
    client: Arc<SseMcpClient>,
    tool: McpToolInfo,
}

#[async_trait]
impl Tool for McpSseProxyTool {
    fn schema(&self) -> ToolDescriptor {
        self.tool.schema.clone()
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let services = crate::tools::runtime_context::runtime_tool_services(ctx).ok();
        let artifacts = services.and_then(|services| services.tool_artifacts.as_deref());
        let started = std::time::Instant::now();
        let result = self
            .client
            .call_tool(&self.tool.remote_name, args)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                reason: err.to_string(),
            })?;
        let duration_ms = elapsed_millis(started);
        let context = mcp_result_context(&self.tool, ctx, artifacts, duration_ms, None);
        Ok(ToolOutput::from_envelope(
            crate::tools::mcp::result_mapping::envelope_from_mcp_result(&result, &context).await,
        ))
    }
}

// --- Streamable HTTP transport ---

/// Connect and initialize a Streamable HTTP client for `server`.
///
/// Plaintext HTTP is permitted only for loopback so a local development server
/// works while a public endpoint still requires TLS.
async fn connect_streamable_http(
    server: &McpServerConfig,
) -> anyhow::Result<crate::tools::mcp::client::StreamableHttpClient> {
    use crate::tools::mcp::client::{McpClientPolicy, StreamableHttpClient};
    use crate::tools::mcp::http_safety::HttpEndpointPolicy;
    use crate::tools::mcp::streamable_http::{StreamableHttpPolicy, streamable_http_transport};

    let transport = streamable_http_transport(
        &server.url,
        StreamableHttpPolicy {
            request_timeout_ms: server.policy.request_timeout_ms,
            endpoint: HttpEndpointPolicy::loopback_permitted(),
            max_reconnect_attempts: crate::tools::mcp::streamable_http::MAX_MCP_RECONNECT_ATTEMPTS,
        },
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let client = StreamableHttpClient::new(
        server.name.clone(),
        transport,
        McpClientPolicy {
            request_timeout_ms: server.policy.request_timeout_ms,
            max_retries: 1,
        },
    );
    client
        .initialize()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(client)
}

/// Project a discovered catalog into the registry's tool-info shape.
///
/// Remote annotations are ignored for safety: every MCP tool stays destructive
/// and non-parallel-safe until the local safety path says otherwise.
async fn streamable_http_tool_infos(
    client: &crate::tools::mcp::client::StreamableHttpClient,
) -> anyhow::Result<Vec<McpToolInfo>> {
    let snapshot = client
        .discover_catalog()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(tool_infos_from_catalog(&snapshot))
}

fn tool_infos_from_catalog(snapshot: &McpCatalogSnapshot) -> Vec<McpToolInfo> {
    snapshot
        .entries
        .iter()
        .map(|entry| McpToolInfo {
            server_name: snapshot.server_name.clone(),
            remote_name: entry.remote_name.clone(),
            schema: ToolDescriptor {
                name: entry.local_name.clone(),
                description: entry.description.clone(),
                parameters: entry.parameters.clone(),
                // Remote annotations are untrusted hints. The local policy
                // remains conservative even when a server claims read-only.
                destructive: true,
                parallel_safe: false,
                capability_id: Some(entry.capability_id.clone()),
                capability: None,
            },
            output_schema: entry.output_schema.clone(),
            annotations: entry.annotations.clone(),
            catalog_hash: snapshot.catalog_hash.clone(),
            protocol_version: snapshot.protocol_version.clone(),
            server_identity_hash: snapshot.server_identity_hash.clone(),
        })
        .collect()
}

pub struct McpStreamableHttpProxyTool {
    client: Arc<crate::tools::mcp::client::StreamableHttpClient>,
    tool: McpToolInfo,
}

/// Hashes a session identifier before it is recorded anywhere.
///
/// A Streamable HTTP session ID is credential-like: possessing it lets a caller
/// speak as this client. It is therefore correlated by hash so a trace, report,
/// or artifact ledger can prove two calls shared a session without carrying the
/// value that would let a reader resume it.
fn session_correlation_hash(session_id: &str) -> String {
    crate::prompt_metadata::stable_hash(&format!("mcp-session:v1:{session_id}"))
}

impl McpStreamableHttpProxyTool {
    /// Assembles the non-result inputs the envelope mapping needs.
    ///
    /// `duration_ms` is measured around the call so a slow server is visible in
    /// the report without the mapping having to own a clock.
    async fn result_context<'a>(
        &'a self,
        ctx: &ToolContext<'_>,
        artifacts: Option<&'a ToolArtifactStore>,
        duration_ms: u64,
    ) -> crate::tools::mcp::result_mapping::McpResultContext<'a> {
        crate::tools::mcp::result_mapping::McpResultContext {
            call_id: ctx.call_id.to_string(),
            remote_tool_name: self.tool.remote_name.clone(),
            server_config_id: self.tool.server_name.clone(),
            server_identity_hash: self.tool.server_identity_hash.clone(),
            protocol_version: self
                .client
                .negotiated_version()
                .await
                .unwrap_or_else(|| "unknown".to_string()),
            capability_snapshot_id: Some(self.tool.catalog_hash.clone()),
            session_hash: self
                .client
                .session_id()
                .await
                .as_deref()
                .map(session_correlation_hash),
            attempt_count: 1,
            duration_ms: Some(duration_ms),
            output_schema: self.tool.output_schema.as_ref(),
            artifacts,
            captured_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

#[async_trait]
impl Tool for McpStreamableHttpProxyTool {
    fn schema(&self) -> ToolDescriptor {
        self.tool.schema.clone()
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        // Services are optional here on purpose: an embedding without a run
        // directory still gets a correct envelope, it just cannot retain binary
        // payloads, and the mapping records that loss instead of inlining bytes.
        let services = crate::tools::runtime_context::runtime_tool_services(ctx).ok();
        let artifacts = services.and_then(|services| services.tool_artifacts.as_deref());

        let started = std::time::Instant::now();
        let outcome = self.client.call_tool(&self.tool.remote_name, args).await;
        let duration_ms = elapsed_millis(started);

        match outcome {
            Ok(result) => {
                let context = self.result_context(ctx, artifacts, duration_ms).await;
                let envelope =
                    crate::tools::mcp::result_mapping::envelope_from_mcp_result(&result, &context)
                        .await;
                Ok(ToolOutput::from_envelope(envelope))
            }
            // A committed request whose effect is unknown must not be presented
            // as a plain failure that a caller might safely retry.
            Err(error) if error.is_indeterminate() => {
                let context = self.result_context(ctx, artifacts, duration_ms).await;
                Ok(ToolOutput::from_envelope(
                    crate::tools::mcp::result_mapping::indeterminate_envelope(
                        &context,
                        &error.to_string(),
                    ),
                ))
            }
            Err(error) => Err(ToolError::ExecutionFailed {
                reason: error.to_string(),
            }),
        }
    }
}

fn elapsed_millis(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn mcp_result_context<'a>(
    tool: &'a McpToolInfo,
    ctx: &ToolContext<'_>,
    artifacts: Option<&'a ToolArtifactStore>,
    duration_ms: u64,
    session_hash: Option<String>,
) -> crate::tools::mcp::result_mapping::McpResultContext<'a> {
    crate::tools::mcp::result_mapping::McpResultContext {
        call_id: ctx.call_id.to_string(),
        remote_tool_name: tool.remote_name.clone(),
        server_config_id: tool.server_name.clone(),
        server_identity_hash: tool.server_identity_hash.clone(),
        protocol_version: tool.protocol_version.clone(),
        capability_snapshot_id: Some(tool.catalog_hash.clone()),
        session_hash,
        attempt_count: 1,
        duration_ms: Some(duration_ms),
        output_schema: tool.output_schema.as_ref(),
        artifacts,
        captured_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn spawn_stderr_capture(stderr: tokio::process::ChildStderr, capture: Arc<Mutex<StderrCapture>>) {
    tokio::spawn(async move {
        let mut stderr = stderr;
        let mut buffer = [0_u8; 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) => break,
                Ok(count) => capture.lock().await.push(&buffer[..count]),
                Err(_) => break,
            }
        }
    });
}

#[derive(Debug)]
struct StderrCapture {
    max_bytes: usize,
    bytes: Vec<u8>,
    truncated: bool,
}

impl StderrCapture {
    fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            bytes: Vec::new(),
            truncated: false,
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        if self.max_bytes == 0 {
            self.truncated = true;
            return;
        }

        let remaining = self.max_bytes.saturating_sub(self.bytes.len());
        if chunk.len() > remaining {
            self.bytes.extend_from_slice(&chunk[..remaining]);
            self.truncated = true;
        } else {
            self.bytes.extend_from_slice(chunk);
        }
    }

    fn snapshot(&self) -> String {
        let mut value = String::from_utf8_lossy(&self.bytes).to_string();
        if self.truncated {
            value.push_str("\n[stderr truncated]");
        }
        value
    }
}

fn format_timeout_error(timeout_ms: u64, stderr: String) -> String {
    if stderr.trim().is_empty() {
        format!("MCP request timed out after {timeout_ms}ms")
    } else {
        format!(
            "MCP request timed out after {timeout_ms}ms; stderr: {}",
            stderr.trim()
        )
    }
}

fn format_mcp_error(error: &Value) -> String {
    let code = error.get("code").and_then(Value::as_i64);
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown MCP error");

    match code {
        Some(code) => format!("MCP JSON-RPC error {code}: {message}"),
        None => format!("MCP JSON-RPC error: {message}"),
    }
}

#[cfg(test)]
mod refresh_health_tests {
    use super::*;

    fn server() -> McpServerConfig {
        McpServerConfig {
            name: "monitoring".to_string(),
            enabled: true,
            required: false,
            transport: McpTransport::StreamableHttp,
            command: String::new(),
            args: Vec::new(),
            env: HashMap::new(),
            env_names: Vec::new(),
            url: "https://mcp.example.com/mcp".to_string(),
            policy: McpTransportPolicy::default(),
        }
    }

    #[test]
    fn refresh_failures_enter_a_bounded_circuit_backoff() {
        assert_eq!(mcp_refresh_backoff(0), Duration::from_secs(1));
        assert_eq!(mcp_refresh_backoff(2), Duration::from_secs(1));
        assert_eq!(mcp_refresh_backoff(3), Duration::from_secs(30));
        assert_eq!(mcp_refresh_backoff(u32::MAX), Duration::from_secs(30));
    }

    #[test]
    fn a_validated_empty_catalog_keeps_the_stable_probe_classification() {
        let failure =
            classify_mcp_probe_error(anyhow::anyhow!("MCP server exposed no usable tools"));
        assert_eq!(failure.kind, McpProbeFailureKind::NoTools);

        let protocol = classify_mcp_probe_error(anyhow::Error::new(
            crate::tools::mcp::protocol::McpProtocolError::MalformedFrame,
        ));
        assert_eq!(protocol.kind, McpProbeFailureKind::Protocol);
    }

    #[test]
    fn only_notification_poll_recovery_clears_its_degraded_health() {
        let state = Arc::new(McpRuntimeState::default());
        state.publish(server_runtime_snapshot(
            &server(),
            None,
            McpServerHealthStatus::Ready,
            None,
        ));
        let weak = Arc::downgrade(&state);

        mark_mcp_degraded(&weak, "monitoring", "mcp_notification_poll_failed");
        mark_mcp_notification_stream_ready(&weak, "monitoring");
        let recovered = state.snapshot("monitoring").unwrap();
        assert_eq!(recovered.status, McpServerHealthStatus::Ready);
        assert!(recovered.failure_code.is_none());

        mark_mcp_degraded(&weak, "monitoring", "mcp_catalog_refresh_failed");
        mark_mcp_notification_stream_ready(&weak, "monitoring");
        let still_degraded = state.snapshot("monitoring").unwrap();
        assert_eq!(still_degraded.status, McpServerHealthStatus::Degraded);
        assert_eq!(
            still_degraded.failure_code.as_deref(),
            Some("mcp_catalog_refresh_failed")
        );
    }
}
