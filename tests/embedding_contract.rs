use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::StreamExt;

use rove::core::context::ContextManager;
use rove::core::engine::{Engine, EngineConfig};
use rove::core::events::StreamEvent;
use rove::core::types::{ApprovalPolicy, TerminationReason, ToolContext, ToolSchema};
use rove::core::workspace::Workspace;
use rove::errors::ToolError;
use rove::hooks::HookRegistry;
use rove::models::fake::{FakeModelClient, FakeTurn};
use rove::tools::registry::ToolRegistry;
use rove::tools::traits::{Tool, ToolOutput};

struct UppercaseTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for UppercaseTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "uppercase".to_string(),
            description: "Convert text to uppercase.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string" }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
            destructive: false,
            parallel_safe: true,
            capability: None,
        }
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = args
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "text is required".to_string(),
            })?;
        Ok(ToolOutput::text(text.to_uppercase()))
    }
}

#[tokio::test]
async fn fake_model_and_custom_tool_embed_without_creating_runtime_state() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let model = FakeModelClient::with_turns(
        "unused fallback".to_string(),
        vec![
            FakeTurn::ToolUse {
                id: "call-uppercase".to_string(),
                name: "uppercase".to_string(),
                args: serde_json::json!({ "text": "rove" }),
            },
            FakeTurn::Text("ROVE".to_string()),
        ],
    );
    let mut tools = ToolRegistry::new();
    tools.register(Box::new(UppercaseTool {
        calls: calls.clone(),
    }));
    let engine = Engine::with_workspace(
        Box::new(model),
        tools,
        ContextManager::new("You are an embedded test agent.".to_string()),
        EngineConfig {
            max_steps: 4,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    )
    .with_hooks(HookRegistry::default());

    let events = engine
        .ask("uppercase rove".to_string(), None)
        .collect::<Vec<_>>()
        .await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ToolCallCompleted { result, .. } if result.output == "ROVE"
        )
    }));
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some(output),
        }) if output == "ROVE"
    ));
    assert!(
        !temp.path().join(".rove").exists(),
        "in-memory embedding must not create persistent runtime state"
    );
}
