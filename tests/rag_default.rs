#![cfg(not(feature = "rag"))]

use rove::tools::rag::RagRetrieveTool;
use rove::tools::traits::Tool;

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
    let tool = RagRetrieveTool::code(".".into());

    let output = tool
        .execute(serde_json::json!({"query": "authentication token"}))
        .await
        .unwrap();

    assert!(output.content.contains("requires the `rag` feature"));
}
