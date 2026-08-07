use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::environment::{
    EnvironmentError, ExecutionEnvironment, StdioProcessGuard, local_environment,
};
use crate::workspace::Workspace;

use rove_core::ToolDescriptor;
use rove_core::{Tool, ToolContext, ToolError, ToolOutput, ToolRegistry};

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_MCP_REQUEST_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MCP_STDERR_CAPTURE_BYTES: usize = 16 * 1024;
const MAX_MCP_CONFIG_BYTES: usize = 256 * 1024;
const MAX_MCP_TOOLS_PER_SERVER: usize = 128;
pub const MAX_MCP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_MCP_ENDPOINT_BYTES: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default = "default_mcp_server_enabled")]
    pub enabled: bool,
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
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Stdio,
    Sse,
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
    let mut pending: Vec<Box<dyn Tool>> = Vec::new();
    for server in servers {
        if !server.enabled {
            continue;
        }
        let server = resolve_mcp_server_environment(server)?;
        match server.transport {
            McpTransport::Stdio => {
                if !environment.capabilities().process_stdio {
                    anyhow::bail!("execution capability unavailable: process_stdio");
                }
                let client = Arc::new(
                    StdioMcpClient::connect_with_environment(server, Arc::clone(&environment))
                        .await?,
                );
                for tool in client.list_tools().await? {
                    pending.push(Box::new(McpProxyTool {
                        client: client.clone(),
                        tool,
                    }));
                }
            }
            McpTransport::Sse => {
                let client = Arc::new(SseMcpClient::connect(server).await?);
                for tool in client.list_tools().await? {
                    pending.push(Box::new(McpSseProxyTool {
                        client: client.clone(),
                        tool,
                    }));
                }
            }
        }
    }
    registry
        .try_register_batch(pending)
        .map_err(anyhow::Error::from)
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
                let client = StdioMcpClient::spawn_with_environment(server, environment)
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
    let kind = if message.contains("timed out") {
        McpProbeFailureKind::Timeout
    } else if error.chain().any(|source| {
        source.downcast_ref::<serde_json::Error>().is_some()
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

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let result = self
            .client
            .call_tool(&self.tool.remote_name, args)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                reason: err.to_string(),
            })?;
        Ok(ToolOutput::text(mcp_call_result_to_text(result)))
    }
}

pub struct StdioMcpClient {
    server_name: String,
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
        let client = Self::spawn_with_environment(config, environment).await?;
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

    async fn initialize(&self) -> anyhow::Result<()> {
        self.request(
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
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpToolInfo>> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("MCP tools/list response missing tools array"))?;
        if tools.len() > MAX_MCP_TOOLS_PER_SERVER {
            anyhow::bail!("MCP tools/list response contains too many tools");
        }
        tools
            .iter()
            .map(|tool| self.parse_tool(tool))
            .collect::<anyhow::Result<Vec<_>>>()
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

    fn parse_tool(&self, tool: &Value) -> anyhow::Result<McpToolInfo> {
        let remote_name = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("MCP tool missing name"))?
            .to_string();
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("MCP server tool")
            .to_string();
        let parameters = tool
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" }));
        // Remote annotations describe intent; they are not a trusted local policy grant.
        let destructive = true;
        let parallel_safe = false;
        let (name, capability_id) = mcp_tool_identity(&self.server_name, &remote_name);

        Ok(McpToolInfo {
            server_name: self.server_name.clone(),
            remote_name,
            schema: ToolDescriptor {
                name,
                description,
                parameters,
                destructive,
                parallel_safe,
                capability_id: Some(capability_id),
                capability: None,
            },
        })
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

fn mcp_call_result_to_text(result: Value) -> String {
    let Some(content) = result.get("content").and_then(Value::as_array) else {
        return result.to_string();
    };

    let mut parts = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) == Some("text")
            && let Some(text) = item.get("text").and_then(Value::as_str)
        {
            parts.push(text.to_string());
        }
    }

    if parts.is_empty() {
        result.to_string()
    } else {
        parts.join("\n")
    }
}

// --- SSE Transport ---

pub struct SseMcpClient {
    server_name: String,
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

        let client = Self {
            server_name: config.name,
            http,
            endpoint,
            next_id: AtomicU64::new(1),
            policy: config.policy,
        };
        client.initialize().await?;
        Ok(client)
    }

    async fn initialize(&self) -> anyhow::Result<()> {
        self.request(
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
        self.notify("notifications/initialized", json!({})).await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> anyhow::Result<Vec<McpToolInfo>> {
        let result = self.request("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("MCP tools/list response missing tools array"))?;
        if tools.len() > MAX_MCP_TOOLS_PER_SERVER {
            anyhow::bail!("MCP tools/list response contains too many tools");
        }
        tools
            .iter()
            .map(|tool| self.parse_tool(tool))
            .collect::<anyhow::Result<Vec<_>>>()
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

    fn parse_tool(&self, tool: &Value) -> anyhow::Result<McpToolInfo> {
        let remote_name = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("MCP tool missing name"))?
            .to_string();
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("MCP server tool")
            .to_string();
        let parameters = tool
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({ "type": "object" }));
        // Remote annotations describe intent; they are not a trusted local policy grant.
        let destructive = true;
        let parallel_safe = false;
        let (name, capability_id) = mcp_tool_identity(&self.server_name, &remote_name);

        Ok(McpToolInfo {
            server_name: self.server_name.clone(),
            remote_name,
            schema: ToolDescriptor {
                name,
                description,
                parameters,
                destructive,
                parallel_safe,
                capability_id: Some(capability_id),
                capability: None,
            },
        })
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

    async fn execute(&self, args: Value, _ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let result = self
            .client
            .call_tool(&self.tool.remote_name, args)
            .await
            .map_err(|err| ToolError::ExecutionFailed {
                reason: err.to_string(),
            })?;
        Ok(ToolOutput::text(mcp_call_result_to_text(result)))
    }
}

fn sanitize_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn mcp_tool_identity(server_name: &str, remote_name: &str) -> (String, String) {
    let server = bounded_identity_component(server_name, "server");
    let remote = bounded_identity_component(remote_name, "tool");
    let identity_hash =
        crate::prompt_metadata::stable_hash(&format!("mcp-tool:v1:{server_name}\0{remote_name}"));
    let short_hash = identity_hash
        .strip_prefix("sha256:")
        .unwrap_or(&identity_hash)
        .chars()
        .take(12)
        .collect::<String>();
    (
        format!("mcp__{server}__{remote}"),
        format!("mcp.{server}.{remote}.{short_hash}"),
    )
}

fn bounded_identity_component(value: &str, fallback: &str) -> String {
    let sanitized = sanitize_name(value);
    let bounded = sanitized.chars().take(64).collect::<String>();
    if bounded.is_empty() {
        fallback.to_string()
    } else {
        bounded
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
