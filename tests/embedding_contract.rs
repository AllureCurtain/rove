use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::StreamExt;

use rove_core::{
    Agent, AgentConfig, AgentEvent, AgentStopReason, Tool, ToolContext, ToolDescriptor, ToolError,
    ToolOutput, ToolRegistry,
};
use rove_models::{FakeModelClient, FakeTurn};

struct UppercaseTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for UppercaseTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
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
    let agent = Agent::new(
        Box::new(model),
        tools,
        AgentConfig {
            system_prompt: Some("You are an embedded test agent.".to_string()),
            max_model_turns: 4,
            max_tool_calls: 4,
        },
    );

    let events = agent.ask("uppercase rove").collect::<Vec<_>>().await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::ToolCallCompleted { output, .. } if output.content == "ROVE"
        )
    }));
    assert!(matches!(
        events.last(),
        Some(AgentEvent::Completed { outcome })
            if outcome.reason == AgentStopReason::Final
                && outcome.output.as_deref() == Some("ROVE")
    ));
    assert!(
        !temp.path().join(".rove").exists(),
        "in-memory embedding must not create persistent runtime state"
    );
}
