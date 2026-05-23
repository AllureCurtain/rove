use rove::core::types::{ApprovalPolicy, ToolContext};
use rove::core::workspace::Workspace;
use rove::tools::mcp_proxy::{McpServerConfig, McpTransport, register_mcp_tools};
use rove::tools::registry::ToolRegistry;
use tokio_util::sync::CancellationToken;

fn python_command() -> String {
    if cfg!(windows) {
        "python".to_string()
    } else {
        "python3".to_string()
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
            &ToolContext {
                workspace: &workspace,
                approval_policy: ApprovalPolicy::Auto,
                cancel_token: CancellationToken::new(),
                input_provider: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(output.content, "remote: hello");
}
