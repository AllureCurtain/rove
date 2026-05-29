use rove::core::types::{ApprovalPolicy, ToolContext};
use rove::core::workspace::Workspace;
use rove::errors::ToolError;
use rove::memory::paths::MemoryPaths;
use rove::tools::mcp_proxy::{
    McpServerConfig, McpTransport, McpTransportPolicy, register_mcp_tools,
};
use rove::tools::registry::ToolRegistry;
use std::collections::HashMap;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

fn python_command() -> String {
    if cfg!(windows) {
        "python".to_string()
    } else {
        "python3".to_string()
    }
}

fn mcp_context<'a>(workspace: &'a Workspace) -> ToolContext<'a> {
    ToolContext {
        workspace,
        memory_paths: MemoryPaths::from_workspace(workspace, 8),
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
        input_provider: None,
    }
}

fn short_mcp_policy() -> McpTransportPolicy {
    McpTransportPolicy {
        request_timeout_ms: 250,
        stderr_capture_bytes: 4096,
    }
}

#[tokio::test]
async fn mcp_proxy_registers_and_calls_stdio_tools() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "mock-server".to_string(),
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec!["tests/fixtures/mcp_mock_server.py".to_string()],
            env: Default::default(),
            url: String::new(),
            policy: McpTransportPolicy::default(),
        }],
    )
    .await
    .unwrap();

    assert_eq!(count, 2);
    assert!(registry.has("mcp__mock_server__echo_remote"));
    assert!(registry.has("mcp__mock_server__delete_remote"));
    assert!(
        registry
            .schema("mcp__mock_server__delete_remote")
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
    let mut registry = ToolRegistry::new();
    let err = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "hanging-server".to_string(),
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec!["tests/fixtures/mcp_hanging_server.py".to_string()],
            env: Default::default(),
            url: String::new(),
            policy: short_mcp_policy(),
        }],
    )
    .await
    .unwrap_err();

    let message = err.to_string();
    assert!(message.contains("timed out after 250ms"), "{message}");
    assert!(
        message.contains("hanging server received initialize"),
        "{message}"
    );
}

#[tokio::test]
async fn mcp_tool_call_error_maps_to_structured_tool_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "error-server".to_string(),
            transport: McpTransport::Stdio,
            command: python_command(),
            args: vec!["tests/fixtures/mcp_error_server.py".to_string()],
            env: Default::default(),
            url: String::new(),
            policy: short_mcp_policy(),
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
                transport: McpTransport::Stdio,
                command: python_command(),
                args: vec!["tests/fixtures/mcp_lifecycle_server.py".to_string()],
                env,
                url: String::new(),
                policy: short_mcp_policy(),
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
async fn mcp_official_filesystem_server_smoke_when_enabled() {
    if std::env::var("ROVE_MCP_FILESYSTEM_SMOKE").ok().as_deref() != Some("1") {
        eprintln!("skipping filesystem MCP smoke; set ROVE_MCP_FILESYSTEM_SMOKE=1 to run");
        return;
    }

    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let note_path = tmp.path().join("note.txt");
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
                tmp.path().to_string_lossy().to_string(),
            ]
        });

    let mut registry = ToolRegistry::new();
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "filesystem-smoke".to_string(),
            transport: McpTransport::Stdio,
            command,
            args,
            env: Default::default(),
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

    assert!(output.content.contains("hello from real filesystem mcp"));
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
