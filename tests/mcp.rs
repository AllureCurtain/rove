use rove::tools::mcp_proxy::{McpServerConfig, register_mcp_tools};
use rove::tools::registry::ToolRegistry;

fn python_command() -> String {
    if cfg!(windows) {
        "python".to_string()
    } else {
        "python3".to_string()
    }
}

#[tokio::test]
async fn mcp_proxy_registers_and_calls_stdio_tools() {
    let mut registry = ToolRegistry::new();
    let count = register_mcp_tools(
        &mut registry,
        vec![McpServerConfig {
            name: "mock-server".to_string(),
            command: python_command(),
            args: vec!["tests/fixtures/mcp_mock_server.py".to_string()],
            env: Default::default(),
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
        )
        .await
        .unwrap();

    assert_eq!(output.content, "remote: hello");
}
