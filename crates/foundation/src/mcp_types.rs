//! Serialized MCP transport/health records. The proxy/transport behaviour
//! lives in `rove_tools::mcp_proxy`.

use serde::{Deserialize, Serialize};

const DEFAULT_MCP_REQUEST_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MCP_STDERR_CAPTURE_BYTES: usize = 16 * 1024;

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

fn default_mcp_stderr_capture_bytes() -> usize {
    DEFAULT_MCP_STDERR_CAPTURE_BYTES
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
