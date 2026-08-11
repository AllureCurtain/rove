//! Streamable HTTP contract tests against a real loopback socket.
//!
//! These drive the client core end to end: real HTTP framing, real headers, real
//! JSON and SSE bodies. They verify transport semantics, not third-party
//! interoperability, which stays a separately reported gate.

use serde_json::json;

use super::client::{McpClientPolicy, StreamableHttpClient};
use super::fixture::{FixtureConfig, FixtureResponseMode, McpFixtureServer};
use super::http_safety::HttpEndpointPolicy;
use super::protocol::{
    MCP_DEFAULT_NEGOTIATED_VERSION, MCP_PROTOCOL_VERSION, McpProtocolError, McpTransportKind,
};
use super::streamable_http::{StreamableHttpPolicy, streamable_http_transport};

fn policy() -> StreamableHttpPolicy {
    StreamableHttpPolicy {
        request_timeout_ms: 5_000,
        endpoint: HttpEndpointPolicy::loopback_permitted(),
        max_reconnect_attempts: 2,
    }
}

async fn client_for(config: FixtureConfig) -> (McpFixtureServer, StreamableHttpClient) {
    let server = McpFixtureServer::start(config).await.unwrap();
    let transport = streamable_http_transport(&server.url(), policy()).unwrap();
    let client = StreamableHttpClient::new(
        "fixture",
        transport,
        McpClientPolicy {
            request_timeout_ms: 5_000,
            max_retries: 1,
        },
    );
    (server, client)
}

#[tokio::test]
async fn a_json_post_completes_initialize_and_negotiates_session_and_version() {
    let (server, client) = client_for(FixtureConfig::default()).await;

    client.initialize().await.unwrap();

    assert_eq!(
        client.negotiated_version().await.as_deref(),
        Some(MCP_PROTOCOL_VERSION)
    );
    assert_eq!(
        client.session_id().await.as_deref(),
        Some("fixture-session-1")
    );
    let identity = client
        .server_identity_hash()
        .await
        .expect("server identity");
    assert!(identity.starts_with("sha256:"));

    let observed = server.observations().await;
    assert_eq!(
        observed.methods.first().map(String::as_str),
        Some("initialize")
    );
    // The negotiated version is echoed on every later request.
    assert!(
        observed
            .protocol_version_headers
            .iter()
            .all(|header| header.is_some())
    );
    // The initialize request itself has no session yet; later ones carry it.
    assert_eq!(observed.session_headers.first().cloned().flatten(), None);
    assert!(
        observed
            .session_headers
            .iter()
            .skip(1)
            .any(|header| header.as_deref() == Some("fixture-session-1")),
        "post-initialize requests must carry the negotiated session"
    );
}

#[tokio::test]
async fn list_and_call_require_a_completed_initialize_identity() {
    let (server, client) = client_for(FixtureConfig::default()).await;

    let list_error = client.discover_catalog().await.unwrap_err();
    let call_error = client.call_tool("echo", json!({})).await.unwrap_err();

    assert!(list_error.to_string().contains("completed initialize"));
    assert!(call_error.to_string().contains("completed initialize"));
    assert!(
        server.observations().await.methods.is_empty(),
        "pre-initialize operations must not reach the server"
    );
}

#[tokio::test]
async fn a_post_sse_response_is_parsed_as_a_json_rpc_response() {
    let (_server, client) = client_for(FixtureConfig {
        response_mode: FixtureResponseMode::EventStream,
        sse_event_id: Some("evt-9".to_string()),
        ..FixtureConfig::default()
    })
    .await;

    client.initialize().await.unwrap();
    let catalog = client.discover_catalog().await.unwrap();

    assert_eq!(catalog.tool_count(), 1);
    assert_eq!(catalog.entries[0].local_name, "mcp__fixture__echo");
    assert_eq!(
        catalog.server_identity_hash,
        client.server_identity_hash().await.unwrap()
    );
}

#[tokio::test]
async fn tools_call_returns_the_remote_result() {
    let (server, client) = client_for(FixtureConfig::default()).await;
    client.initialize().await.unwrap();

    let result = client
        .call_tool("echo", json!({ "text": "hi" }))
        .await
        .unwrap();

    assert_eq!(result["content"][0]["text"], "fixture ok");
    let observed = server.observations().await;
    assert!(observed.methods.iter().any(|method| method == "tools/call"));
}

#[tokio::test]
async fn pagination_follows_every_cursor_into_one_catalog() {
    let (server, client) = client_for(FixtureConfig {
        tool_pages: vec![
            json!({
                "tools": [{ "name": "first", "inputSchema": { "type": "object" } }],
                "nextCursor": "cursor-2"
            }),
            json!({
                "tools": [{ "name": "second", "inputSchema": { "type": "object" } }],
                "nextCursor": "cursor-3"
            }),
            json!({ "tools": [{ "name": "third", "inputSchema": { "type": "object" } }] }),
        ],
        ..FixtureConfig::default()
    })
    .await;
    client.initialize().await.unwrap();

    let catalog = client.discover_catalog().await.unwrap();

    assert_eq!(catalog.tool_count(), 3);
    for expected in [
        "mcp__fixture__first",
        "mcp__fixture__second",
        "mcp__fixture__third",
    ] {
        assert!(catalog.entry(expected).is_some(), "missing {expected}");
    }
    // The cursors the server issued were actually sent back.
    let observed = server.observations().await;
    let cursors: Vec<_> = observed.cursors.iter().flatten().cloned().collect();
    assert_eq!(
        cursors,
        vec!["cursor-2".to_string(), "cursor-3".to_string()]
    );
}

#[tokio::test]
async fn a_server_error_surfaces_as_a_typed_protocol_error() {
    let (_server, client) = client_for(FixtureConfig {
        force_error: Some((-32001, "tool unavailable".to_string())),
        ..FixtureConfig::default()
    })
    .await;

    let error = client.initialize().await.unwrap_err();

    match error {
        McpProtocolError::Server(error) => {
            assert_eq!(error.code, -32001);
            assert_eq!(error.message, "tool unavailable");
        }
        other => panic!("expected a server error, got {other:?}"),
    }
}

#[tokio::test]
async fn an_absent_protocol_version_falls_back_to_the_documented_default() {
    let (_server, client) = client_for(FixtureConfig {
        protocol_version: None,
        ..FixtureConfig::default()
    })
    .await;

    client.initialize().await.unwrap();

    assert_eq!(
        client.negotiated_version().await.as_deref(),
        Some(MCP_DEFAULT_NEGOTIATED_VERSION)
    );
}

#[tokio::test]
async fn an_unsupported_protocol_version_fails_the_session() {
    let (_server, client) = client_for(FixtureConfig {
        protocol_version: Some("1999-01-01".to_string()),
        ..FixtureConfig::default()
    })
    .await;

    let error = client.initialize().await.unwrap_err();

    assert!(matches!(
        error,
        McpProtocolError::UnsupportedProtocolVersion { .. }
    ));
}

#[tokio::test]
async fn a_session_id_that_would_inject_a_header_is_refused() {
    let (_server, client) = client_for(FixtureConfig {
        session_id: Some("bad id with spaces".to_string()),
        ..FixtureConfig::default()
    })
    .await;

    let error = client.initialize().await.unwrap_err();

    assert_eq!(error, McpProtocolError::InvalidSessionId);
}

#[tokio::test]
async fn a_wrong_content_type_is_refused_rather_than_guessed() {
    let (_server, client) = client_for(FixtureConfig {
        wrong_content_type: Some("text/html".to_string()),
        ..FixtureConfig::default()
    })
    .await;

    let error = client.initialize().await.unwrap_err();

    assert!(error.is_indeterminate(), "committed response: {error:?}");
    assert!(!error.is_safely_retryable());
}

#[tokio::test]
async fn an_unsolicited_notification_is_delivered_alongside_a_response() {
    let (_server, client) = client_for(FixtureConfig {
        notify_before_response: Some("notifications/tools/list_changed".to_string()),
        ..FixtureConfig::default()
    })
    .await;

    client.initialize().await.unwrap();

    assert!(
        client.tools_changed().await,
        "a list_changed notification must reach the client"
    );
}

#[tokio::test]
async fn the_optional_get_stream_delivers_server_notifications() {
    let (_server, client) = client_for(FixtureConfig {
        sse_event_id: Some("evt-1".to_string()),
        ..FixtureConfig::default()
    })
    .await;
    client.initialize().await.unwrap();

    let routed = client.poll_notifications().await.unwrap();

    assert_eq!(routed, 1);
    assert!(client.tools_changed().await);
}

#[tokio::test]
async fn closing_the_client_deletes_the_negotiated_session() {
    let (server, client) = client_for(FixtureConfig::default()).await;
    client.initialize().await.unwrap();
    assert!(client.session_id().await.is_some());

    client.close().await.unwrap();

    assert!(client.session_id().await.is_none());
    let observed = server.observations().await;
    assert!(
        observed
            .http_methods
            .iter()
            .any(|method| method == "DELETE"),
        "shutdown must release the server session"
    );
}

#[tokio::test]
async fn a_server_that_rejects_delete_still_releases_the_local_session() {
    let (_server, client) = client_for(FixtureConfig {
        reject_delete: true,
        ..FixtureConfig::default()
    })
    .await;
    client.initialize().await.unwrap();

    client.close().await.unwrap();

    assert!(
        client.session_id().await.is_none(),
        "a 405 means the server owns lifetime; the local session is still released"
    );
}

#[tokio::test]
async fn a_notification_acknowledged_with_202_is_not_treated_as_a_result() {
    let (server, client) = client_for(FixtureConfig::default()).await;

    client.initialize().await.unwrap();

    let observed = server.observations().await;
    assert!(
        observed
            .methods
            .iter()
            .any(|method| method == "notifications/initialized"),
        "the initialized notification must be sent"
    );
}

#[tokio::test]
async fn a_cancelled_client_stops_issuing_requests() {
    let (_server, client) = client_for(FixtureConfig::default()).await;
    client.initialize().await.unwrap();

    client.cancel_token().cancel();
    let error = client.call_tool("echo", json!({})).await.unwrap_err();

    assert_eq!(error, McpProtocolError::Cancelled);
}

#[tokio::test]
async fn a_committed_call_timeout_is_indeterminate_and_cleans_up_its_waiter() {
    let server = McpFixtureServer::start(FixtureConfig {
        response_mode: FixtureResponseMode::Accepted,
        ..FixtureConfig::default()
    })
    .await
    .unwrap();
    let transport = streamable_http_transport(
        &server.url(),
        StreamableHttpPolicy {
            request_timeout_ms: 250,
            endpoint: HttpEndpointPolicy::loopback_permitted(),
            max_reconnect_attempts: 0,
        },
    )
    .unwrap();
    transport
        .record_initialize(
            &json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "serverInfo": { "name": "fixture", "version": "1.0.0" }
            }),
            Some("fixture-session-1"),
        )
        .await
        .unwrap();
    let client = StreamableHttpClient::new(
        "accepted-without-result",
        transport,
        McpClientPolicy {
            request_timeout_ms: 25,
            max_retries: 3,
        },
    );

    let error = client.call_tool("echo", json!({})).await.unwrap_err();

    assert!(error.is_indeterminate(), "committed call: {error:?}");
    assert!(!error.is_safely_retryable());
    assert_eq!(client.pending_request_count().await, 0);
    let observed = server.observations().await;
    assert_eq!(
        observed
            .methods
            .iter()
            .filter(|method| method.as_str() == "tools/call")
            .count(),
        1,
        "an indeterminate call must never be retried"
    );
}

#[tokio::test]
async fn an_unreachable_endpoint_is_reported_as_retryable_not_indeterminate() {
    // Port 1 on loopback refuses immediately, so nothing was ever sent.
    let transport = streamable_http_transport("http://127.0.0.1:1/mcp", policy()).unwrap();
    let client = StreamableHttpClient::new(
        "unreachable",
        transport,
        McpClientPolicy {
            request_timeout_ms: 2_000,
            max_retries: 0,
        },
    );

    let error = client.initialize().await.unwrap_err();

    assert!(
        error.is_safely_retryable(),
        "a refused connection never delivered a request: {error:?}"
    );
    assert!(!error.is_indeterminate());
}

#[tokio::test]
async fn the_transport_reports_its_real_feature_set() {
    let (_server, client) = client_for(FixtureConfig::default()).await;

    assert_eq!(client.transport_kind(), McpTransportKind::StreamableHttp);
    let features = client.features();
    assert!(features.http_session);
    assert!(features.delete_session);
    assert!(features.get_stream);
    assert!(features.concurrent_requests);
    assert!(!features.child_lifecycle, "HTTP owns no child process");
    assert_eq!(client.safe_summary(), "fixture via streamable_http");
}

#[tokio::test]
async fn an_empty_catalog_is_refused_rather_than_registered() {
    let (_server, client) = client_for(FixtureConfig {
        tool_pages: vec![json!({ "tools": [] })],
        ..FixtureConfig::default()
    })
    .await;
    client.initialize().await.unwrap();

    let error = client.discover_catalog().await.unwrap_err();

    assert!(
        matches!(error, McpProtocolError::Transport { .. }),
        "{error:?}"
    );
}
