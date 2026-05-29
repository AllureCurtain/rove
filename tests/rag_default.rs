#![cfg(not(feature = "rag"))]

use rove::core::types::{ApprovalPolicy, ToolContext};
use rove::core::workspace::Workspace;
use rove::memory::paths::MemoryPaths;
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
    assert_eq!(code.capability.as_ref().unwrap().status, "disabled");
    assert_eq!(
        code.capability.as_ref().unwrap().feature.as_deref(),
        Some("rag")
    );
    assert_eq!(docs.capability.as_ref().unwrap().status, "disabled");
}

#[tokio::test]
async fn rag_retrieve_tool_explains_feature_requirement_without_rag_feature() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let ctx = ToolContext {
        workspace: &workspace,
        memory_paths: MemoryPaths::from_workspace(&workspace, 8),
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
    let json: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(json["capability"], "disabled");
    assert_eq!(json["feature"], "rag");
}
