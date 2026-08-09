//! Deterministic in-process MCP server fixtures.
//!
//! These exist so Streamable HTTP behavior is verified against a real socket,
//! real headers, and real response bodies rather than a mocked client. A fixture
//! is a test double for the *server*, not for the transport under test.
//!
//! A fixture never stands in for real interoperability with a third-party
//! server: an external-server smoke remains a separate, explicitly reported
//! gate.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::protocol::{MCP_PROTOCOL_VERSION, MCP_SESSION_ID_HEADER};

/// How the fixture should answer a POST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixtureResponseMode {
    /// Answer with `application/json`.
    Json,
    /// Answer with `text/event-stream`, one frame per message.
    EventStream,
    /// Answer 202 with no body, as for a notification.
    Accepted,
}

/// Behavior knobs for one fixture server.
#[derive(Debug, Clone)]
pub struct FixtureConfig {
    pub response_mode: FixtureResponseMode,
    /// Session ID to assign on initialize. `None` omits the header.
    pub session_id: Option<String>,
    /// Protocol version to report. `None` omits the field.
    pub protocol_version: Option<String>,
    /// Tool pages returned by successive `tools/list` calls.
    pub tool_pages: Vec<Value>,
    /// Result returned by `tools/call`.
    pub call_result: Value,
    /// Answer every request with this JSON-RPC error instead of a result.
    pub force_error: Option<(i64, String)>,
    /// Emit this SSE event ID so a client can resume.
    pub sse_event_id: Option<String>,
    /// Answer DELETE with 405 to model a server that owns session lifetime.
    pub reject_delete: bool,
    /// Send an unsolicited notification before each response.
    pub notify_before_response: Option<String>,
    /// Respond with this content type instead of the correct one.
    pub wrong_content_type: Option<String>,
}

impl Default for FixtureConfig {
    fn default() -> Self {
        Self {
            response_mode: FixtureResponseMode::Json,
            session_id: Some("fixture-session-1".to_string()),
            protocol_version: Some(MCP_PROTOCOL_VERSION.to_string()),
            tool_pages: vec![json!({
                "tools": [{
                    "name": "echo",
                    "description": "echoes text",
                    "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } } }
                }]
            })],
            call_result: json!({ "content": [{ "type": "text", "text": "fixture ok" }] }),
            force_error: None,
            sse_event_id: None,
            reject_delete: false,
            notify_before_response: None,
            wrong_content_type: None,
        }
    }
}

/// Requests the fixture observed, for assertions.
#[derive(Debug, Clone, Default)]
pub struct FixtureObservations {
    pub methods: Vec<String>,
    pub session_headers: Vec<Option<String>>,
    pub protocol_version_headers: Vec<Option<String>>,
    pub last_event_ids: Vec<Option<String>>,
    pub http_methods: Vec<String>,
    pub cursors: Vec<Option<String>>,
}

/// A running fixture server bound to loopback.
pub struct McpFixtureServer {
    addr: SocketAddr,
    observations: Arc<Mutex<FixtureObservations>>,
    handle: JoinHandle<()>,
}

impl McpFixtureServer {
    /// Start a fixture on an ephemeral loopback port.
    pub async fn start(config: FixtureConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let observations = Arc::new(Mutex::new(FixtureObservations::default()));
        let shared_observations = Arc::clone(&observations);
        let list_calls = Arc::new(AtomicU64::new(0));

        let handle = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let config = config.clone();
                let observations = Arc::clone(&shared_observations);
                let list_calls = Arc::clone(&list_calls);
                tokio::spawn(async move {
                    let _ = serve_connection(stream, config, observations, list_calls).await;
                });
            }
        });

        Ok(Self {
            addr,
            observations,
            handle,
        })
    }

    /// Endpoint URL for this fixture.
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    pub async fn observations(&self) -> FixtureObservations {
        self.observations.lock().await.clone()
    }
}

impl Drop for McpFixtureServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn serve_connection(
    mut stream: TcpStream,
    config: FixtureConfig,
    observations: Arc<Mutex<FixtureObservations>>,
    list_calls: Arc<AtomicU64>,
) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];

    // Read until the headers are complete, then read exactly Content-Length.
    let header_end = loop {
        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 1024 * 1024 {
            return Ok(());
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let (http_method, headers) = parse_head(&head);
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    {
        let mut observed = observations.lock().await;
        observed.http_methods.push(http_method.clone());
        observed
            .session_headers
            .push(headers.get(MCP_SESSION_ID_HEADER).cloned());
        observed
            .protocol_version_headers
            .push(headers.get("mcp-protocol-version").cloned());
        observed
            .last_event_ids
            .push(headers.get("last-event-id").cloned());
    }

    let response = match http_method.as_str() {
        "DELETE" => {
            if config.reject_delete {
                "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_string()
            } else {
                "HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_string()
            }
        }
        // The optional GET notification stream.
        "GET" => {
            let frame = sse_frame(
                &json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/tools/list_changed",
                    "params": {}
                }),
                config.sse_event_id.as_deref(),
            );
            sse_response(&frame, &config, None)
        }
        _ => {
            let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let cursor = request
                .get("params")
                .and_then(|params| params.get("cursor"))
                .and_then(Value::as_str)
                .map(str::to_string);
            {
                let mut observed = observations.lock().await;
                observed.methods.push(method.clone());
                observed.cursors.push(cursor);
            }

            // A notification has no ID and is acknowledged with 202.
            match request.get("id").cloned() {
                None => "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_string(),
                Some(id) => build_rpc_response(&config, &method, id, &list_calls),
            }
        }
    };

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

fn build_rpc_response(
    config: &FixtureConfig,
    method: &str,
    id: Value,
    list_calls: &AtomicU64,
) -> String {
    let payload = if let Some((code, message)) = &config.force_error {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        })
    } else {
        let result = match method {
            "initialize" => {
                let mut result = json!({
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "fixture", "version": "1.0.0" }
                });
                if let Some(version) = &config.protocol_version {
                    result["protocolVersion"] = json!(version);
                }
                result
            }
            "tools/list" => {
                let index = list_calls.fetch_add(1, Ordering::Relaxed) as usize;
                config
                    .tool_pages
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| json!({ "tools": [] }))
            }
            "tools/call" => config.call_result.clone(),
            _ => json!({}),
        };
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    };

    let session_header = (method == "initialize")
        .then(|| config.session_id.clone())
        .flatten();

    match config.response_mode {
        FixtureResponseMode::Accepted => {
            "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string()
        }
        FixtureResponseMode::Json => {
            let mut body = String::new();
            if let Some(method_name) = &config.notify_before_response {
                // A JSON body may carry a batch, so an unsolicited notification
                // rides alongside the response.
                let batch = json!([
                    { "jsonrpc": "2.0", "method": method_name, "params": {} },
                    payload
                ]);
                body.push_str(&batch.to_string());
            } else {
                body.push_str(&payload.to_string());
            }
            let content_type = config
                .wrong_content_type
                .clone()
                .unwrap_or_else(|| "application/json".to_string());
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n{}connection: close\r\n\r\n{body}",
                body.len(),
                session_header
                    .map(|id| format!("{MCP_SESSION_ID_HEADER}: {id}\r\n"))
                    .unwrap_or_default(),
            )
        }
        FixtureResponseMode::EventStream => {
            let mut frames = String::new();
            if let Some(method_name) = &config.notify_before_response {
                frames.push_str(&sse_frame(
                    &json!({ "jsonrpc": "2.0", "method": method_name, "params": {} }),
                    None,
                ));
            }
            frames.push_str(&sse_frame(&payload, config.sse_event_id.as_deref()));
            sse_response(&frames, config, session_header)
        }
    }
}

fn sse_frame(payload: &Value, event_id: Option<&str>) -> String {
    let mut frame = String::new();
    if let Some(id) = event_id {
        frame.push_str(&format!("id: {id}\n"));
    }
    frame.push_str("event: message\n");
    frame.push_str(&format!("data: {}\n\n", payload));
    frame
}

fn sse_response(body: &str, config: &FixtureConfig, session_id: Option<String>) -> String {
    let content_type = config
        .wrong_content_type
        .clone()
        .unwrap_or_else(|| "text/event-stream".to_string());
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n{}connection: close\r\n\r\n{body}",
        body.len(),
        session_id
            .map(|id| format!("{MCP_SESSION_ID_HEADER}: {id}\r\n"))
            .unwrap_or_default(),
    )
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_head(head: &str) -> (String, HashMap<String, String>) {
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let http_method = request_line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    (http_method, headers)
}
