use std::path::PathBuf;

use rove_core::{CallId, ToolContext, ToolRegistry};
use rove_runtime::Workspace;
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::tools::mcp_proxy::{
    McpServerConfig, McpTransport, McpTransportPolicy, register_mcp_tools,
};
use rove_runtime::tools::runtime_context::runtime_tool_context;
use rove_runtime::types::ApprovalPolicy;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

static MCP_STDIO_TEST_LOCK: Mutex<()> = Mutex::const_new(());

fn python_command() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn tool_context(workspace: &Workspace) -> ToolContext<'static> {
    runtime_tool_context(
        CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    )
}

#[tokio::test]
async fn runtime_registers_calls_and_classifies_stdio_mcp_tools() {
    let _guard = MCP_STDIO_TEST_LOCK.lock().await;
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let mut registry = ToolRegistry::new();

    let registered = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "mock-server".to_string(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: python_command().to_string(),
            args: vec![fixture("mcp_mock_server.py").to_string_lossy().to_string()],
            env: Default::default(),
            env_names: Vec::new(),
            url: String::new(),
            policy: McpTransportPolicy::default(),
        }],
    )
    .await
    .unwrap();

    assert_eq!(registered, 2);
    let remotely_claimed_read_only = registry
        .descriptor("mcp__mock_server__echo_remote")
        .unwrap();
    assert!(remotely_claimed_read_only.destructive);
    assert!(!remotely_claimed_read_only.parallel_safe);
    let destructive = registry
        .descriptor("mcp__mock_server__delete_remote")
        .unwrap();
    assert!(destructive.destructive);
    assert!(!destructive.parallel_safe);

    let output = registry
        .execute(
            "mcp__mock_server__echo_remote",
            serde_json::json!({"message": "from runtime"}),
            &tool_context(&workspace),
        )
        .await
        .unwrap();
    assert_eq!(output.content, "remote: from runtime");
}
