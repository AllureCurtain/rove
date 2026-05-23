use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use super::registry::ToolRegistry;
use super::traits::{Tool, ToolOutput};
use crate::core::types::{ToolContext, ToolSchema};
use crate::errors::ToolError;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    #[serde(default)]
    pub transport: McpTransport,
    /// Command to spawn (stdio transport only).
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// SSE endpoint URL (sse transport only).
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    #[default]
    Stdio,
    Sse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfigFile {
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub server_name: String,
    pub remote_name: String,
    pub schema: ToolSchema,
}

pub async fn register_mcp_tools_from_file(
    registry: &mut ToolRegistry,
    path: impl Into<PathBuf>,
) -> anyhow::Result<usize> {
    let path = path.into();
    if !path.exists() {
        return Ok(0);
    }

    let bytes = tokio::fs::read(path).await?;
    let config: McpConfigFile = serde_json::from_slice(&bytes)?;
    register_mcp_tools(registry, config.servers).await
}

pub async fn register_mcp_tools(
    registry: &mut ToolRegistry,
    servers: Vec<McpServerConfig>,
) -> anyhow::Result<usize> {
    let mut registered = 0;
    for server in servers {
        match server.transport {
            McpTransport::Stdio => {
                let client = Arc::new(StdioMcpClient::connect(server).await?);
                for tool in client.list_tools().await? {
                    registry.register(Box::new(McpProxyTool {
                        client: client.clone(),
                        tool,
                    }));
                    registered += 1;
                }
            }
            McpTransport::Sse => {
                let client = Arc::new(SseMcpClient::connect(server).await?);
                for tool in client.list_tools().await? {
                    registry.register(Box::new(McpSseProxyTool {
                        client: client.clone(),
                        tool,
                    }));
                    registered += 1;
                }
            }
        }
    }
    Ok(registered)
}

pub struct McpProxyTool {
    client: Arc<StdioMcpClient>,
    tool: McpToolInfo,
}

#[async_trait]
impl Tool for McpProxyTool {
    fn schema(&self) -> ToolSchema {
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
        Ok(ToolOutput {
            content: mcp_call_result_to_text(result),
        })
    }
}

pub struct StdioMcpClient {
    server_name: String,
    next_id: AtomicU64,
    transport: Mutex<StdioTransport>,
}

impl StdioMcpClient {
    pub async fn connect(config: McpServerConfig) -> anyhow::Result<Self> {
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdout unavailable"))?;

        let client = Self {
            server_name: config.name,
            next_id: AtomicU64::new(1),
            transport: Mutex::new(StdioTransport {
                child,
                stdin,
                lines: BufReader::new(stdout).lines(),
            }),
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
        transport.read_response(id).await
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
        let destructive = tool
            .get("annotations")
            .and_then(|annotations| annotations.get("destructiveHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let name = format!("mcp__{}__{}", sanitize_name(&self.server_name), remote_name);

        Ok(McpToolInfo {
            server_name: self.server_name.clone(),
            remote_name,
            schema: ToolSchema {
                name,
                description,
                parameters,
                destructive,
            },
        })
    }
}

impl Drop for StdioMcpClient {
    fn drop(&mut self) {
        if let Ok(mut transport) = self.transport.try_lock() {
            let _ = transport.child.start_kill();
        }
    }
}

struct StdioTransport {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
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
        while let Some(line) = self.lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&line)?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                anyhow::bail!("MCP error response: {error}");
            }
            return Ok(message.get("result").cloned().unwrap_or_else(|| json!({})));
        }
        anyhow::bail!("MCP server closed stdout before responding to request {id}");
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
}

impl SseMcpClient {
    pub async fn connect(config: McpServerConfig) -> anyhow::Result<Self> {
        let http = reqwest::Client::new();
        let endpoint = discover_endpoint(&http, &config.url).await?;

        let client = Self {
            server_name: config.name,
            http,
            endpoint,
            next_id: AtomicU64::new(1),
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
        let response = self
            .http
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("MCP SSE request failed: {text}");
        }

        let body: Value = response.json().await?;
        if let Some(error) = body.get("error") {
            anyhow::bail!("MCP error response: {error}");
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
        let destructive = tool
            .get("annotations")
            .and_then(|annotations| annotations.get("destructiveHint"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let name = format!("mcp__{}__{}", sanitize_name(&self.server_name), remote_name);

        Ok(McpToolInfo {
            server_name: self.server_name.clone(),
            remote_name,
            schema: ToolSchema {
                name,
                description,
                parameters,
                destructive,
            },
        })
    }
}

async fn discover_endpoint(http: &reqwest::Client, sse_url: &str) -> anyhow::Result<String> {
    let response = http.get(sse_url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("MCP SSE endpoint returned HTTP {}", response.status());
    }

    use futures::StreamExt;
    let mut byte_stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = byte_stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer = buffer[line_end + 1..].to_string();

            if let Some(data) = line.strip_prefix("data: ") {
                let endpoint = data.trim().to_string();
                if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
                    return Ok(endpoint);
                }
                let base = sse_url.rfind('/').map(|i| &sse_url[..i]).unwrap_or(sse_url);
                return Ok(format!("{}/{}", base, endpoint.trim_start_matches('/')));
            }
        }
    }

    anyhow::bail!("MCP SSE stream closed without providing an endpoint URL")
}

pub struct McpSseProxyTool {
    client: Arc<SseMcpClient>,
    tool: McpToolInfo,
}

#[async_trait]
impl Tool for McpSseProxyTool {
    fn schema(&self) -> ToolSchema {
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
        Ok(ToolOutput {
            content: mcp_call_result_to_text(result),
        })
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
