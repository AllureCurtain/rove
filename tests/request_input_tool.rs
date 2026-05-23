use rove::errors::ToolError;
use rove::tools::request_input::RequestInputTool;
use rove::tools::traits::Tool;

#[test]
fn request_input_tool_schema_exposes_prompt_input() {
    let schema = RequestInputTool.schema();

    assert_eq!(schema.name, "request_input");
    assert!(!schema.destructive);
    assert_eq!(schema.parameters["required"][0], "prompt");
    assert_eq!(schema.parameters["properties"]["prompt"]["type"], "string");
}

#[tokio::test]
async fn request_input_tool_requires_prompt_argument() {
    let err = RequestInputTool
        .execute(serde_json::json!({}))
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidArgs { reason } if reason.contains("prompt")
    ));
}

#[tokio::test]
async fn request_input_tool_explains_interactive_provider_requirement() {
    let output = RequestInputTool
        .execute(serde_json::json!({"prompt": "Which branch should I use?"}))
        .await
        .unwrap();

    assert!(
        output
            .content
            .contains("requires an interactive input provider")
    );
    assert!(output.content.contains("Which branch should I use?"));
}
