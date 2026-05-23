#![cfg(not(feature = "rag"))]

use rove::core::types::{ApprovalPolicy, ToolContext};
use rove::core::workspace::Workspace;
use rove::tools::rag::RagRetrieveTool;
use rove::tools::traits::Tool;
use tokio_util::sync::CancellationToken;

#[test]
fn rag_retrieve_tool_schemas_exist_without_rag_feature() {
    let code = RagRetrieveTool::code(".".into()).schema();
    let docs = RagRetrieveTool::docs(".".into()).schema();

    assert_eq!(code.name, "retrieve_code");
    assert_eq!(docs.name, "retrieve_docs");
    assert!(!code.destructive);
    assert!(!docs.destructive);
}

#[tokio::test]
async fn rag_retrieve_tool_explains_feature_requirement_without_rag_feature() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
        input_provider: None,
    };
    let tool = RagRetrieveTool::code(workspace.root.clone());

    let output = tool
        .execute(serde_json::json!({"query": "authentication token"}), &ctx)
        .await
        .unwrap();

    assert!(output.content.contains("requires the `rag` feature"));
}
