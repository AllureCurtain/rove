use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn workspace_path(rel: impl AsRef<Path>) -> PathBuf {
    workspace_root().join(rel)
}

fn workspace_path_string(rel: impl AsRef<Path>) -> String {
    workspace_path(rel).to_string_lossy().into_owned()
}

use rove_core::ToolError;
use rove_core::ToolRegistry;
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::tools::mcp_proxy::{
    MAX_MCP_RESPONSE_BYTES, McpProbeFailureKind, McpServerConfig, McpTransport, McpTransportPolicy,
    probe_mcp_server, register_mcp_tools, resolve_mcp_server_environment,
};
use rove_runtime::tools::runtime_context::runtime_tool_context;
use rove_runtime::types::{ApprovalPolicy, ToolContext};
use rove_runtime::workspace::Workspace;
use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

static MCP_STDIO_TEST_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn mcp_sse_rejects_oversized_discovery_and_json_responses() {
    use axum::Router;
    use axum::http::header::CONTENT_TYPE;
    use axum::routing::{get, post};

    for oversized_discovery in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let oversized = "x".repeat(MAX_MCP_RESPONSE_BYTES + 1);
        let router = if oversized_discovery {
            Router::new().route(
                "/sse",
                get(move || {
                    let body = oversized.clone();
                    async move { ([(CONTENT_TYPE, "text/event-stream")], body) }
                }),
            )
        } else {
            Router::new()
                .route(
                    "/sse",
                    get(|| async {
                        ([(CONTENT_TYPE, "text/event-stream")], "data: /messages\n\n")
                    }),
                )
                .route(
                    "/messages",
                    post(move || {
                        let body = oversized.clone();
                        async move { ([(CONTENT_TYPE, "application/json")], body) }
                    }),
                )
        };
        let server_task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });

        let failure = probe_mcp_server(McpServerConfig {
            name: "bounded-sse".to_string(),
            enabled: true,
            transport: McpTransport::Sse,
            command: String::new(),
            args: Vec::new(),
            env: Default::default(),
            env_names: Vec::new(),
            url: format!("http://{address}/sse"),
            policy: responsive_mcp_policy(),
        })
        .await
        .unwrap_err();

        assert_eq!(failure.kind, McpProbeFailureKind::Protocol);
        server_task.abort();
    }
}

fn python_command() -> String {
    if cfg!(windows) {
        "python".to_string()
    } else {
        "python3".to_string()
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

fn short_mcp_policy() -> McpTransportPolicy {
    McpTransportPolicy {
        request_timeout_ms: 250,
        stderr_capture_bytes: 4096,
    }
}

fn responsive_mcp_policy() -> McpTransportPolicy {
    McpTransportPolicy {
        request_timeout_ms: 2_000,
        stderr_capture_bytes: 4096,
    }
}

#[tokio::test]
async fn mcp_proxy_registers_and_calls_stdio_tools() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "mock-server".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec![workspace_path_string("tests/fixtures/mcp_mock_server.py").to_string()],
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: McpTransportPolicy::default(),
        }],
    )
    .await
    .unwrap();

    assert_eq!(count, 2);
    assert!(registry.has("mcp__mock_server__echo_remote"));
    assert!(registry.has("mcp__mock_server__delete_remote"));
    let remotely_claimed_read_only = registry
        .descriptor("mcp__mock_server__echo_remote")
        .unwrap();
    assert!(remotely_claimed_read_only.destructive);
    assert!(!remotely_claimed_read_only.parallel_safe);
    assert!(
        registry
            .descriptor("mcp__mock_server__delete_remote")
            .unwrap()
            .destructive
    );

    let output = registry
        .execute(
            "mcp__mock_server__echo_remote",
            serde_json::json!({ "message": "hello" }),
            &mcp_context(&workspace),
        )
        .await
        .unwrap();

    assert_eq!(output.content, "remote: hello");
}

#[tokio::test]
async fn mcp_stdio_requests_time_out_when_server_does_not_respond() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let mut registry = ToolRegistry::new();
    let err = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "hanging-server".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec![workspace_path_string("tests/fixtures/mcp_hanging_server.py").to_string()],
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: short_mcp_policy(),
        }],
    )
    .await
    .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("timed out after 250ms"), "{message}");
}

#[tokio::test]
async fn mcp_tool_call_error_maps_to_structured_tool_error() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "error-server".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec![workspace_path_string("tests/fixtures/mcp_error_server.py").to_string()],
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: responsive_mcp_policy(),
        }],
    )
    .await
    .unwrap();

    let err = registry
        .execute(
            "mcp__error_server__fail_remote",
            serde_json::json!({}),
            &mcp_context(&workspace),
        )
        .await
        .unwrap_err();

    match err {
        ToolError::ExecutionFailed { reason } => {
            assert_eq!(reason, "MCP JSON-RPC error -32000: remote boom");
        }
        other => panic!("expected MCP execution failure, got {other:?}"),
    }
}

#[tokio::test]
async fn dropping_stdio_mcp_registry_cleans_up_child_process() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let pid_path = tmp.path().join("mcp.pid");
    let mut env = HashMap::new();
    env.insert(
        "ROVE_MCP_TEST_PID_FILE".to_string(),
        pid_path.to_string_lossy().to_string(),
    );

    {
        let mut registry = ToolRegistry::new();
        register_mcp_tools(
            &mut registry,
            vec![McpServerConfig {
                name: "lifecycle-server".to_string(),
                enabled: true,
                transport: McpTransport::Stdio,
                command: python_command(),
                args: vec![
                    workspace_path_string("tests/fixtures/mcp_lifecycle_server.py").to_string(),
                ],
                env,
                env_names: Vec::new(),
                url: String::new(),
                policy: responsive_mcp_policy(),
            }],
        )
        .await
        .unwrap();
        assert!(registry.has("mcp__lifecycle_server__ping_remote"));
    }

    let pid: u32 = std::fs::read_to_string(&pid_path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_process_exits(pid, Duration::from_secs(3));
}

#[tokio::test]
async fn disabled_mcp_servers_are_never_assembled_or_environment_resolved() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let mut registry = ToolRegistry::new();
    // A disabled server must be skipped before environment resolution and spawn,
    // so an unavailable variable and a bogus command must not fail assembly.
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "disabled_server".to_string(),
            enabled: false,
            transport: McpTransport::Stdio,
            command: "rove-command-that-does-not-exist-019fcfd2".to_string(),
            args: Vec::new(),
            env: Default::default(),
            env_names: vec!["ROVE_MCP_ENV_MISSING_019FCFD2".to_string()],
            url: String::new(),
            policy: short_mcp_policy(),
        }],
    )
    .await
    .unwrap();

    assert_eq!(count, 0);
    assert!(!registry.has("mcp__disabled_server__echo_remote"));
    assert!(
        registry
            .descriptors()
            .iter()
            .all(|descriptor| !descriptor.name.starts_with("mcp__disabled_server__"))
    );
}

#[test]
fn mcp_environment_resolution_rejects_invalid_and_unavailable_names() {
    let base = McpServerConfig {
        name: "env_server".to_string(),
        enabled: true,
        transport: McpTransport::Stdio,
        command: python_command(),
        args: Vec::new(),
        env: Default::default(),
        env_names: Vec::new(),
        url: String::new(),
        policy: short_mcp_policy(),
    };

    for invalid in ["BAD-NAME", "1LEADING_DIGIT", "", "HAS SPACE", "HAS=EQUALS"] {
        let mut server = base.clone();
        server.env_names = vec![invalid.to_string()];
        let error = resolve_mcp_server_environment(server).unwrap_err();
        assert!(
            error.to_string().contains("invalid"),
            "unexpected error for {invalid:?}: {error}"
        );
    }

    let mut missing = base.clone();
    missing.env_names = vec!["ROVE_MCP_ENV_MISSING_019FCFD2".to_string()];
    let error = resolve_mcp_server_environment(missing).unwrap_err();
    assert!(
        error.to_string().contains("unavailable"),
        "unexpected error: {error}"
    );

    // A present variable is injected by name only; the catalog never stores the value.
    let mut present = base;
    present.env_names = vec!["PATH".to_string()];
    let resolved = resolve_mcp_server_environment(present).unwrap();
    assert_eq!(resolved.env_names, vec!["PATH".to_string()]);
    assert_eq!(
        resolved.env.get("PATH"),
        Some(&std::env::var("PATH").unwrap())
    );
}

#[tokio::test]
async fn mcp_official_filesystem_server_smoke_when_enabled() {
    if std::env::var("ROVE_MCP_FILESYSTEM_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("skipping filesystem MCP smoke; set ROVE_MCP_FILESYSTEM_SMOKE=1 to run");
        return;
    }
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;

    let tmp = tempfile::TempDir::new().unwrap();
    let allowed_dir = normalize_windows_extended_path(tmp.path().canonicalize().unwrap());
    let workspace = Workspace::detect(&allowed_dir).unwrap();
    let note_path = allowed_dir.join("note.txt");
    std::fs::write(&note_path, "hello from real filesystem mcp").unwrap();

    let command = std::env::var("ROVE_MCP_FILESYSTEM_COMMAND").unwrap_or_else(|_| {
        if cfg!(windows) {
            "npx.cmd".to_string()
        } else {
            "npx".to_string()
        }
    });
    let args = std::env::var("ROVE_MCP_FILESYSTEM_ARGS")
        .ok()
        .map(|value| value.split_whitespace().map(str::to_string).collect())
        .unwrap_or_else(|| {
            vec![
                "-y".to_string(),
                "@modelcontextprotocol/server-filesystem".to_string(),
                allowed_dir.to_string_lossy().to_string(),
            ]
        });

    let mut registry = ToolRegistry::new();
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "filesystem-smoke".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command,
            args,
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: McpTransportPolicy {
                request_timeout_ms: 15_000,
                stderr_capture_bytes: 8192,
            },
        }],
    )
    .await
    .unwrap();

    assert!(count > 0);
    assert!(registry.has("mcp__filesystem_smoke__read_file"));

    let output = registry
        .execute(
            "mcp__filesystem_smoke__read_file",
            serde_json::json!({ "path": note_path }),
            &mcp_context(&workspace),
        )
        .await
        .unwrap();

    assert!(
        output.content.contains("hello from real filesystem mcp"),
        "unexpected filesystem MCP output: {}",
        output.content
    );
}

fn normalize_windows_extended_path(path: std::path::PathBuf) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(stripped) = raw.strip_prefix(r"\\?\") {
            return std::path::PathBuf::from(stripped);
        }
    }
    path
}

fn assert_process_exits(pid: u32, timeout: Duration) {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if !process_is_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("MCP child process {pid} was still alive after {timeout:?}");
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
}

#[cfg(not(windows))]
fn process_is_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
