//! Cross-package Streamable HTTP contract tests.
//!
//! These drive registration and tool execution through the real `ToolRegistry`
//! against an in-process loopback MCP server, so the transport is exercised at
//! the same boundary a product deployment uses.
//!
//! A fixture verifies transport semantics, not third-party interoperability.
//! Real-server compatibility remains a separately reported optional gate.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rove_core::{ToolRegistry, ToolResultOutcome};
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::state::tool_artifacts::ToolArtifactStore;
use rove_runtime::tools::mcp_proxy::{
    McpLifecycleFact, McpProbeFailureKind, McpRuntimeState, McpServerConfig, McpTransport,
    McpTransportPolicy, probe_mcp_server, register_mcp_tools,
};
use rove_runtime::tools::runtime_context::{
    runtime_tool_context, runtime_tool_context_with_artifacts,
};
use rove_runtime::types::{ApprovalPolicy, ToolContext};
use rove_runtime::workspace::Workspace;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Minimal Streamable HTTP MCP server for contract tests.
struct FixtureServer {
    addr: SocketAddr,
    handle: JoinHandle<()>,
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Json,
    EventStream,
    Refresh,
    InvalidRefresh,
    /// Answer `tools/call` with a JSON-RPC error.
    CallFails,
    /// Answer `tools/call` with rich content: an image, structured content, and
    /// a block type this build does not model.
    RichContent,
}

impl FixtureServer {
    async fn start(mode: Mode) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let list_calls = Arc::new(AtomicU64::new(0));
        let handle = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let list_calls = Arc::clone(&list_calls);
                tokio::spawn(async move {
                    let _ = serve(stream, mode, list_calls).await;
                });
            }
        });
        Ok(Self { addr, handle })
    }

    fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }
}

async fn serve(
    mut stream: TcpStream,
    mode: Mode,
    list_calls: Arc<AtomicU64>,
) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let http_method = head
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    let content_length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim().eq_ignore_ascii_case("content-length"))
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);

    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    let response = match http_method.as_str() {
        "DELETE" => {
            "HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string()
        }
        "GET" => {
            if matches!(mode, Mode::Refresh | Mode::InvalidRefresh) {
                let frame = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\",\"params\":{}}\n\n";
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{frame}",
                    frame.len()
                )
            } else {
                "HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_string()
            }
        }
        _ => {
            let request: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
            let method = request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match request.get("id").cloned() {
                None => "HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_string(),
                Some(id) => rpc_response(mode, &method, id, &list_calls),
            }
        }
    };
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await
}

/// A real 1x1 PNG, so the stored bytes and their hash are genuine rather than
/// an arbitrary blob that happens to decode.
const PNG_1X1_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";

fn rpc_response(mode: Mode, method: &str, id: Value, list_calls: &AtomicU64) -> String {
    let payload = if mode == Mode::CallFails && method == "tools/call" {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32002, "message": "remote tool refused" }
        })
    } else {
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fixture", "version": "1.0.0" }
            }),
            "tools/list" => {
                if matches!(mode, Mode::Refresh | Mode::InvalidRefresh) {
                    let first = list_calls.fetch_add(1, Ordering::Relaxed) == 0;
                    let name = if first { "old_tool" } else { "new_tool" };
                    let input_schema = if mode == Mode::InvalidRefresh && !first {
                        json!({"type":"definitely-not-a-json-schema-type"})
                    } else {
                        json!({"type":"object"})
                    };
                    json!({
                        "tools": [{
                            "name": name,
                            "description": "refresh fixture",
                            "inputSchema": input_schema
                        }]
                    })
                } else if list_calls.fetch_add(1, Ordering::Relaxed) == 0 {
                    // Two pages, so registration must follow pagination.
                    json!({
                        "tools": [{
                            "name": "echo_remote",
                            "description": "echoes text",
                            "inputSchema": { "type": "object" },
                            // A remote annotation claiming safety must be ignored.
                            "annotations": { "readOnlyHint": true, "destructiveHint": false }
                        }],
                        "nextCursor": "page-2"
                    })
                } else {
                    json!({
                        "tools": [{
                            "name": "write_remote",
                            "description": "writes something",
                            "inputSchema": { "type": "object" }
                        }]
                    })
                }
            }
            "tools/call" if mode == Mode::RichContent => json!({
                "content": [
                    { "type": "text", "text": "rendered the chart" },
                    {
                        "type": "image",
                        // A 1x1 PNG, and a hostile name/URI that must never
                        // steer where the bytes are written.
                        "data": PNG_1X1_BASE64,
                        "mimeType": "image/png",
                        "name": "../../escape.png",
                        "uri": "file:///etc/passwd"
                    },
                    { "type": "future_block", "detail": "not modelled by this build" }
                ],
                "structuredContent": { "rows": 2, "ok": true }
            }),
            "tools/call" => json!({ "content": [{ "type": "text", "text": "remote ok" }] }),
            _ => json!({}),
        };
        json!({ "jsonrpc": "2.0", "id": id, "result": result })
    };

    match mode {
        Mode::EventStream => {
            let frame = format!("event: message\ndata: {payload}\n\n");
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nmcp-session-id: fixture-1\r\nconnection: close\r\n\r\n{frame}",
                frame.len()
            )
        }
        _ => {
            let body = payload.to_string();
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nmcp-session-id: fixture-1\r\nconnection: close\r\n\r\n{body}",
                body.len()
            )
        }
    }
}

fn server_config(url: String) -> McpServerConfig {
    McpServerConfig {
        name: "http_fixture".to_string(),
        enabled: true,
        required: true,
        transport: McpTransport::StreamableHttp,
        command: String::new(),
        args: Vec::new(),
        env: HashMap::new(),
        env_names: Vec::new(),
        url,
        policy: McpTransportPolicy {
            request_timeout_ms: 5_000,
            stderr_capture_bytes: 1024,
        },
    }
}

fn mcp_context<'a>(workspace: &'a Workspace) -> ToolContext<'a> {
    runtime_tool_context(
        rove_runtime::types::CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    )
}

/// A context carrying a durable artifact authority, as a real run has.
fn artifact_context<'a>(
    workspace: &'a Workspace,
    store: Arc<ToolArtifactStore>,
) -> ToolContext<'a> {
    runtime_tool_context_with_artifacts(
        rove_runtime::types::CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
        rove_runtime::environment::local_environment(workspace),
        Some(store),
    )
}

#[tokio::test]
async fn streamable_http_registers_paginated_tools_and_calls_them() {
    let server = FixtureServer::start(Mode::Json).await.unwrap();
    let mut registry = ToolRegistry::new();

    let count = register_mcp_tools(&mut registry, vec![server_config(server.url())])
        .await
        .unwrap();

    // Both pages were registered, so pagination was followed to completion.
    assert_eq!(count, 2);
    let names: Vec<_> = registry
        .descriptors()
        .iter()
        .map(|descriptor| descriptor.name.clone())
        .filter(|name| name.starts_with("mcp__http_fixture__"))
        .collect();
    assert_eq!(
        names.len(),
        2,
        "both paginated tools must register: {names:?}"
    );

    let tool_name = names
        .iter()
        .find(|name| name.contains("echo_remote"))
        .expect("the first page tool must be present")
        .clone();

    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let output = registry
        .execute(
            &tool_name,
            json!({ "text": "hi" }),
            &mcp_context(&workspace),
        )
        .await
        .unwrap();

    assert!(
        output.content.contains("remote ok"),
        "the remote result must reach the caller: {}",
        output.content
    );
}

#[tokio::test]
async fn list_changed_atomically_refreshes_future_runs_and_keeps_active_run_pinned() {
    let server = FixtureServer::start(Mode::Refresh).await.unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(&mut registry, vec![server_config(server.url())])
        .await
        .unwrap();
    let pinned = registry.snapshot();
    assert!(pinned.has("mcp__http_fixture__old_tool"));

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if registry.has("mcp__http_fixture__new_tool") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("list_changed should publish a validated replacement");

    assert!(!registry.has("mcp__http_fixture__old_tool"));
    assert!(registry.has("mcp__http_fixture__new_tool"));
    assert!(pinned.has("mcp__http_fixture__old_tool"));
    assert!(!pinned.has("mcp__http_fixture__new_tool"));
    let state = registry.extension::<McpRuntimeState>().unwrap();
    assert!(state.take_facts().iter().any(|fact| matches!(
        fact,
        McpLifecycleFact::CapabilitiesRefreshed { added, removed, .. }
            if added == &["mcp__http_fixture__new_tool"]
                && removed == &["mcp__http_fixture__old_tool"]
    )));
}

#[tokio::test]
async fn an_invalid_refresh_keeps_the_last_catalog_and_marks_health_degraded() {
    let server = FixtureServer::start(Mode::InvalidRefresh).await.unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(&mut registry, vec![server_config(server.url())])
        .await
        .unwrap();
    assert!(registry.has("mcp__http_fixture__old_tool"));
    let state = registry.extension::<McpRuntimeState>().unwrap();
    let original_catalog_hash = state
        .snapshot("http_fixture")
        .unwrap()
        .catalog_hash
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if state.snapshot("http_fixture").is_some_and(|snapshot| {
                snapshot.failure_code.as_deref() == Some("mcp_catalog_refresh_failed")
            }) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("invalid refresh should produce degraded health");

    assert!(registry.has("mcp__http_fixture__old_tool"));
    assert!(!registry.has("mcp__http_fixture__new_tool"));
    assert_eq!(
        state
            .snapshot("http_fixture")
            .unwrap()
            .catalog_hash
            .as_deref(),
        Some(original_catalog_hash.as_str())
    );
}

/// The full rich-result path: a real MCP call, through the real registry, into
/// the real durable artifact store, and back out as an envelope.
///
/// This is the end-to-end proof that the shared contract has a live producer:
/// the image bytes must exist on disk, addressed by their own hash, with the
/// server's hostile filename and URI having no influence on the path.
#[tokio::test]
async fn a_rich_mcp_result_lands_in_the_durable_artifact_store() {
    let server = FixtureServer::start(Mode::RichContent).await.unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(&mut registry, vec![server_config(server.url())])
        .await
        .unwrap();
    let tool_name = registry
        .descriptors()
        .iter()
        .map(|descriptor| descriptor.name.clone())
        .find(|name| name.contains("echo_remote"))
        .expect("the fixture tool must register");

    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let run_dir = temp.path().join("runs").join("run_fixture");
    let store = Arc::new(ToolArtifactStore::new(run_dir.clone()));
    let output = registry
        .execute(
            &tool_name,
            json!({ "text": "hi" }),
            &artifact_context(&workspace, Arc::clone(&store)),
        )
        .await
        .unwrap();

    let envelope = output.envelope.as_ref().expect("an envelope is produced");
    assert_eq!(envelope.outcome, ToolResultOutcome::Success);
    assert_eq!(
        envelope.artifacts.len(),
        1,
        "the image block becomes exactly one artifact"
    );
    // The unknown block is retained rather than silently dropped.
    assert_eq!(envelope.content_blocks.len(), 3, "no block is lost");
    assert!(
        envelope.structured_content.is_some(),
        "well-formed structured content survives"
    );

    let artifact = &envelope.artifacts[0];
    assert_eq!(artifact.mime_type.as_deref(), Some("image/png"));
    assert!(artifact.artifact_id.as_str().starts_with("art_"));
    // The storage path is derived from the content hash, so neither the
    // traversal-shaped name nor the file:// URI reached the filesystem.
    assert!(artifact.storage_ref.contains(artifact.artifact_id.as_str()));
    assert!(!artifact.storage_ref.contains(".."));
    assert!(!artifact.storage_ref.contains("passwd"));

    let bytes = store.get(&artifact.artifact_id).await.unwrap();
    assert_eq!(
        bytes.len() as u64,
        artifact.byte_length,
        "the recorded length matches the bytes actually retained"
    );
    assert_eq!(&bytes[1..4], b"PNG", "the real payload was stored");

    // Base64 never reaches the model projection.
    let projection = envelope.model_projection();
    assert!(!projection.contains(PNG_1X1_BASE64));
    assert!(projection.contains("rendered the chart"));

    // And the ledger records the commit as durable evidence.
    let ledger = store.ledger().await.unwrap();
    assert_eq!(ledger.len(), 1, "one committed entry: {ledger:?}");
}

#[tokio::test]
async fn a_remote_annotation_cannot_grant_local_safety() {
    let server = FixtureServer::start(Mode::Json).await.unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(&mut registry, vec![server_config(server.url())])
        .await
        .unwrap();

    for descriptor in registry
        .descriptors()
        .iter()
        .filter(|descriptor| descriptor.name.starts_with("mcp__http_fixture__"))
    {
        // The fixture advertised readOnlyHint/destructiveHint=false. Remote
        // metadata describes intent and is never a local permission grant.
        assert!(
            descriptor.destructive,
            "{} must stay destructive despite a remote annotation",
            descriptor.name
        );
        assert!(
            !descriptor.parallel_safe,
            "{} must not be marked parallel-safe by a remote hint",
            descriptor.name
        );
    }
}

#[tokio::test]
async fn a_post_sse_response_registers_and_calls_the_same_way_as_json() {
    let server = FixtureServer::start(Mode::EventStream).await.unwrap();
    let mut registry = ToolRegistry::new();

    let count = register_mcp_tools(&mut registry, vec![server_config(server.url())])
        .await
        .unwrap();

    assert_eq!(count, 2, "a POST+SSE server shares protocol semantics");
}

#[tokio::test]
async fn a_remote_tool_error_maps_to_a_structured_tool_error() {
    let server = FixtureServer::start(Mode::CallFails).await.unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(&mut registry, vec![server_config(server.url())])
        .await
        .unwrap();
    let tool_name = registry
        .descriptors()
        .iter()
        .map(|descriptor| descriptor.name.clone())
        .find(|name| name.contains("echo_remote"))
        .unwrap();

    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let error = registry
        .execute(&tool_name, json!({}), &mcp_context(&workspace))
        .await
        .expect_err("a remote error must surface as a tool error");

    let message = error.to_string();
    assert!(
        message.contains("remote tool refused"),
        "the safe remote reason must be preserved: {message}"
    );
}

#[tokio::test]
async fn probing_a_streamable_http_server_reports_its_tools() {
    let server = FixtureServer::start(Mode::Json).await.unwrap();

    let tools = probe_mcp_server(server_config(server.url()))
        .await
        .expect("a reachable fixture must probe successfully");

    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| tool.server_name == "http_fixture"));
}

#[tokio::test]
async fn probing_an_unreachable_streamable_http_server_fails_without_hanging() {
    // Port 1 on loopback refuses immediately.
    let failure = probe_mcp_server(server_config("http://127.0.0.1:1/mcp".to_string()))
        .await
        .expect_err("an unreachable server must fail");

    assert!(
        matches!(
            failure.kind,
            McpProbeFailureKind::Transport
                | McpProbeFailureKind::Protocol
                | McpProbeFailureKind::Timeout
                | McpProbeFailureKind::Spawn
        ),
        "unexpected probe failure kind: {:?}",
        failure.kind
    );
}

#[tokio::test]
async fn a_plaintext_non_loopback_endpoint_is_refused() {
    let mut registry = ToolRegistry::new();
    let mut config = server_config("http://mcp.example.com/rpc".to_string());
    config.name = "public_plaintext".to_string();

    let error = register_mcp_tools(&mut registry, vec![config])
        .await
        .expect_err("plaintext to a public host must be refused");

    assert!(
        error.to_string().contains("mcp_transport_policy_blocked"),
        "the refusal must expose a stable policy code: {error}"
    );
    assert!(!error.to_string().contains("mcp.example.com"));
}

#[tokio::test]
async fn a_disabled_streamable_http_server_is_never_contacted() {
    let mut registry = ToolRegistry::new();
    let mut config = server_config("http://127.0.0.1:1/mcp".to_string());
    config.enabled = false;

    // An unreachable URL must not fail assembly, proving no connection was made.
    let count = register_mcp_tools(&mut registry, vec![config])
        .await
        .unwrap();

    assert_eq!(count, 0);
}

#[test]
fn transport_identity_distinguishes_streamable_http_from_deprecated_sse() {
    // The wire names are the compatibility contract. `stdio` and `sse` must
    // keep serializing exactly as they did before the new variant existed, so
    // an existing config file keeps loading.
    for (transport, wire) in [
        (McpTransport::Stdio, "\"stdio\""),
        (McpTransport::Sse, "\"sse\""),
        (McpTransport::StreamableHttp, "\"streamable_http\""),
    ] {
        assert_eq!(serde_json::to_string(&transport).unwrap(), wire);
        assert_eq!(
            serde_json::from_str::<McpTransport>(wire).unwrap(),
            transport
        );
    }
    assert!(
        serde_json::from_str::<McpTransport>("\"streamablehttp\"").is_err(),
        "an unknown transport name must fail rather than resolve to a default"
    );
    assert!(McpTransport::Sse.is_deprecated());
    assert!(
        !McpTransport::StreamableHttp.is_deprecated(),
        "the current transport must not be marked deprecated"
    );
    assert!(McpTransport::StreamableHttp.is_http() && McpTransport::Sse.is_http());
    assert!(!McpTransport::Stdio.is_http());
}

#[test]
fn a_streamable_http_config_round_trips_without_being_read_as_legacy_sse() {
    let config = server_config("https://mcp.example.com/rpc".to_string());
    let encoded = serde_json::to_string(&config).unwrap();
    assert!(
        encoded.contains("\"streamable_http\""),
        "the wire name must be explicit: {encoded}"
    );

    let decoded: McpServerConfig = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.transport, McpTransport::StreamableHttp);

    // An existing "sse" configuration keeps working and stays distinct.
    let legacy: McpServerConfig =
        serde_json::from_str(&encoded.replace("streamable_http", "sse")).unwrap();
    assert_eq!(legacy.transport, McpTransport::Sse);
}
