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
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
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
        let client = Arc::new(StdioMcpClient::connect(server).await?);
        for tool in client.list_tools().await? {
            registry.register(Box::new(McpProxyTool::new(client.clone(), tool)));
            registered += 1;
        }
    }
    Ok(registered)
}

pub struct McpProxyTool {
    client: Arc<StdioMcpClient>,
    tool: McpToolInfo,
}

impl McpProxyTool {
    pub fn new(client: Arc<StdioMcpClient>, tool: McpToolInfo) -> Self {
        Self { client, tool }
    }
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
