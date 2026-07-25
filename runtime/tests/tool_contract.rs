use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rove_core::{CallId, ToolContext, ToolError, ToolRegistry};
use rove_runtime::Workspace;
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::tools::fs::FsWriteTool;
use rove_runtime::tools::memory::SaveMemoryTool;
use rove_runtime::tools::request_input::RequestInputTool;
use rove_runtime::tools::runtime_context::{runtime_tool_context, runtime_tool_services};
use rove_runtime::tools::shell::{ShellPolicy, ShellTool};
use rove_runtime::types::{ApprovalPolicy, PendingUserInput, UserInputProvider, UserInputRequest};
use tokio_util::sync::CancellationToken;

struct StaticInputProvider {
    prompts: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl UserInputProvider for StaticInputProvider {
    async fn begin_input(
        &self,
        _input_id: CallId,
        request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        self.prompts.lock().unwrap().push(request.prompt);
        Ok(PendingUserInput::new(async {
            Ok("runtime answer".to_string())
        }))
    }
}

fn tool_context(
    workspace: &Workspace,
    memory_paths: MemoryPaths,
    input_provider: Option<Arc<dyn UserInputProvider>>,
) -> ToolContext<'static> {
    runtime_tool_context(
        CallId::new(),
        workspace,
        memory_paths,
        ApprovalPolicy::Auto,
        input_provider,
        CancellationToken::new(),
    )
}

#[tokio::test]
async fn filesystem_tool_rejects_traversal_and_bounds_writes_to_workspace() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let workspace = Workspace::detect(&workspace_root).unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));

    let error = registry
        .execute(
            "write_file",
            serde_json::json!({"path": "../outside.txt", "content": "escape"}),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ToolError::PermissionDenied { reason } if reason.contains("escapes workspace")
    ));
    assert!(!temp.path().join("outside.txt").exists());

    registry
        .execute(
            "write_file",
            serde_json::json!({"path": "nested/note.txt", "content": "inside"}),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.root.join("nested/note.txt")).unwrap(),
        "inside"
    );
}

#[test]
fn runtime_tool_context_exposes_runtime_owned_services() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let memory_paths = MemoryPaths::from_workspace(&workspace, 5);
    let context = tool_context(&workspace, memory_paths.clone(), None);

    let services = runtime_tool_services(&context).unwrap();
    assert_eq!(services.workspace.root, workspace.root);
    assert_eq!(services.workspace.state_dir, workspace.state_dir);
    assert_eq!(services.workspace.kind, workspace.kind);
    assert_eq!(services.memory_paths, memory_paths);
    assert_eq!(services.approval_policy, ApprovalPolicy::Auto);
    assert!(services.input_provider.is_none());

    let bare_context = ToolContext::new(CallId::new(), CancellationToken::new());
    assert!(matches!(
        runtime_tool_services(&bare_context),
        Err(ToolError::ExecutionFailed { reason })
            if reason.contains("runtime tool services are not available")
    ));
}

#[tokio::test]
async fn request_input_tool_uses_the_runtime_provider() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(StaticInputProvider {
        prompts: prompts.clone(),
    });
    let context = tool_context(
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        Some(provider),
    );
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(RequestInputTool));

    let output = registry
        .execute(
            "request_input",
            serde_json::json!({"prompt": "Which branch?"}),
            &context,
        )
        .await
        .unwrap();

    assert_eq!(output.content, "runtime answer");
    assert_eq!(prompts.lock().unwrap().as_slice(), ["Which branch?"]);
}

#[tokio::test]
async fn memory_tool_uses_configured_paths_and_rejects_unsafe_promotion() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let durable_dir = workspace.root.join("configured-memory");
    let memory_paths = MemoryPaths {
        session_dir: workspace.state_dir.join("memory/sessions"),
        durable_dir: durable_dir.clone(),
        recall_limit: 8,
    };
    let context = tool_context(&workspace, memory_paths, None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new()));

    let traversal_error = registry
        .execute(
            "save_memory",
            serde_json::json!({
                "topic": "../outside",
                "content": "stable project convention",
                "type": "project"
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        traversal_error,
        ToolError::InvalidInput { reason } if reason.contains("safe topic")
    ));

    let secret_error = registry
        .execute(
            "save_memory",
            serde_json::json!({
                "topic": "Deployment",
                "content": "Store API key sk-test-secret",
                "type": "project"
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        secret_error,
        ToolError::InvalidInput { reason } if reason.contains("must not contain secrets")
    ));

    let output = registry
        .execute(
            "save_memory",
            serde_json::json!({
                "topic": "Project Convention",
                "content": "Run cargo fmt before committing.",
                "type": "project"
            }),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(output.content, "saved memory: project-convention");
    assert!(durable_dir.join("topics/project-convention.md").exists());
    assert!(!workspace.state_dir.join("memory/topics").exists());
}

#[tokio::test]
async fn run_shell_rejects_empty_nul_and_denied_commands_before_execution() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        ShellPolicy {
            denylist: vec!["blocked-command".to_string()],
            ..ShellPolicy::default()
        },
    )));

    for (command, expected_code) in [
        ("   ", "invalid_input"),
        ("has\0nul", "invalid_input"),
        ("blocked-command --flag", "permission_denied"),
    ] {
        let error = registry
            .execute(
                "run_shell",
                serde_json::json!({"command": command}),
                &context,
            )
            .await
            .unwrap_err();
        assert_eq!(error.error_code(), expected_code);
    }
}
