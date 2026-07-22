use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rove::core::types::{
    ApprovalPolicy, CallId, PendingUserInput, ToolContext, UserInputProvider, UserInputRequest,
};
use rove::core::workspace::Workspace;
use rove::errors::ToolError;
use rove::memory::paths::MemoryPaths;
use rove::tools::request_input::RequestInputTool;
use rove::tools::runtime_context::runtime_tool_context;
use rove::tools::traits::Tool;
use tokio_util::sync::CancellationToken;

struct StaticInputProvider {
    answer: &'static str,
    prompts: Arc<Mutex<Vec<String>>>,
}

struct LegacyInputProvider;

fn tool_context<'a>(
    workspace: &Workspace,
    input_provider: Option<Arc<dyn UserInputProvider>>,
) -> ToolContext<'a> {
    runtime_tool_context(
        CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Auto,
        input_provider,
        CancellationToken::new(),
    )
}

#[async_trait]
impl UserInputProvider for LegacyInputProvider {
    async fn request_input(&self, request: UserInputRequest) -> Result<String, ToolError> {
        Ok(format!("legacy: {}", request.prompt))
    }
}

#[async_trait]
impl UserInputProvider for StaticInputProvider {
    async fn begin_input(
        &self,
        _input_id: CallId,
        request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        self.prompts.lock().unwrap().push(request.prompt);
        let answer = self.answer.to_string();
        Ok(PendingUserInput::new(async move { Ok(answer) }))
    }
}

#[test]
fn request_input_tool_schema_exposes_prompt_input() {
    let schema = RequestInputTool.schema();

    assert_eq!(schema.name, "request_input");
    assert!(!schema.destructive);
    assert_eq!(schema.parameters["required"][0], "prompt");
    assert_eq!(schema.parameters["properties"]["prompt"]["type"], "string");
}

#[tokio::test]
async fn legacy_one_phase_provider_implementations_remain_source_compatible() {
    let answer = LegacyInputProvider
        .request_input(UserInputRequest {
            prompt: "Which branch?".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(answer, "legacy: Which branch?");
}

#[tokio::test]
async fn request_input_tool_requires_prompt_argument() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let ctx = tool_context(&workspace, None);
    let err = RequestInputTool
        .execute(serde_json::json!({}), &ctx)
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidArgs { reason } if reason.contains("prompt")
    ));
}

#[tokio::test]
async fn request_input_tool_explains_interactive_provider_requirement() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let ctx = tool_context(&workspace, None);
    let output = RequestInputTool
        .execute(
            serde_json::json!({"prompt": "Which branch should I use?"}),
            &ctx,
        )
        .await
        .unwrap();

    assert!(
        output
            .content
            .contains("requires an interactive input provider")
    );
    assert!(output.content.contains("Which branch should I use?"));
}

#[tokio::test]
async fn request_input_tool_returns_interactive_provider_answer() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(StaticInputProvider {
        answer: "Use main.",
        prompts: prompts.clone(),
    });
    let ctx = tool_context(&workspace, Some(provider));

    let output = RequestInputTool
        .execute(
            serde_json::json!({"prompt": "Which branch should I use?"}),
            &ctx,
        )
        .await
        .unwrap();

    assert_eq!(output.content, "Use main.");
    assert_eq!(
        prompts.lock().unwrap().as_slice(),
        ["Which branch should I use?"]
    );
}
