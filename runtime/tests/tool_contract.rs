use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rove_core::{CallId, ToolContext, ToolError, ToolRegistry};
use rove_runtime::Workspace;
use rove_runtime::environment::{
    ExecutionEnvironment, InMemoryExecutionEnvironment, ProcessOutput,
};
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::tools::coding::{
    DeletePathTool, EditFileTool, GlobPathsTool, ListDirectoryTool, MovePathTool,
    WorkspaceCheckpointTool, WorkspaceDiffTool, WorkspaceRewindTool,
};
use rove_runtime::tools::fs::{FsReadTool, FsWriteTool};
use rove_runtime::tools::memory::SaveMemoryTool;
use rove_runtime::tools::request_input::RequestInputTool;
use rove_runtime::tools::runtime_context::{
    runtime_tool_context, runtime_tool_context_with_environment, runtime_tool_services,
};
use rove_runtime::tools::search::{SearchCodePolicy, SearchCodeTool};
use rove_runtime::tools::shell::{
    ShellOutputTool, ShellPolicy, ShellPtyTool, ShellTerminateTool, ShellTool,
};
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
async fn search_code_returns_structured_matches_and_caps_output() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace_root = temp.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let workspace = Workspace::detect(&workspace_root).unwrap();
    std::fs::create_dir_all(workspace.root.join("src")).unwrap();
    std::fs::write(
        workspace.root.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();
    std::fs::write(workspace.root.join("src/other.toml"), "alpha = 1\n").unwrap();

    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SearchCodeTool::with_policy(
        workspace.root.clone(),
        SearchCodePolicy {
            max_matches: 10,
            max_output_bytes: 64 * 1024,
            ..SearchCodePolicy::default()
        },
    )));

    let output = registry
        .execute(
            "search_code",
            serde_json::json!({
                "query": "alpha",
                "glob": "*.rs"
            }),
            &context,
        )
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(body["match_count"], 1);
    assert_eq!(body["matches"][0]["path"], "src/lib.rs");
    assert!(
        body["matches"][0]["text"]
            .as_str()
            .unwrap()
            .contains("alpha")
    );

    let escaped = registry
        .execute(
            "search_code",
            serde_json::json!({
                "query": "alpha",
                "path": "../"
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        escaped,
        ToolError::PermissionDenied { .. } | ToolError::InvalidInput { .. }
    ));
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

#[tokio::test]
async fn ranged_read_continuation_is_exact_and_rejects_a_stale_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::write(workspace.root.join("note.txt"), "abcdef").unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));

    let first = registry
        .execute(
            "read_file",
            serde_json::json!({"path":"note.txt", "limit":3}),
            &context,
        )
        .await
        .unwrap();
    let first: serde_json::Value = serde_json::from_str(&first.content).unwrap();
    assert_eq!(first["content"], "abc");
    assert_eq!(first["offset"], 0);
    assert_eq!(first["end"], 3);
    assert_eq!(first["truncated"], true);

    let second = registry
        .execute(
            "read_file",
            serde_json::json!({
                "path":"note.txt",
                "limit":3,
                "continuation": first["continuation"]
            }),
            &context,
        )
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_str(&second.content).unwrap();
    assert_eq!(second["content"], "def");
    assert_eq!(second["offset"], 3);
    assert_eq!(second["truncated"], false);

    std::fs::write(workspace.root.join("note.txt"), "changed").unwrap();
    let error = registry
        .execute(
            "read_file",
            serde_json::json!({
                "path":"note.txt",
                "limit":3,
                "continuation": first["continuation"]
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidInput { reason } if reason.contains("stale")));
}

#[tokio::test]
async fn large_read_projects_full_content_and_keeps_only_the_requested_page_active() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let full_content = "x".repeat(300_000);
    std::fs::write(workspace.root.join("large.txt"), &full_content).unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));

    let output = registry
        .execute(
            "read_file",
            serde_json::json!({"path":"large.txt", "limit":1024}),
            &context,
        )
        .await
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(output["content"].as_str().unwrap().len(), 1024);
    assert_eq!(output["total_bytes"], 300_000);
    assert_eq!(output["truncated"], true);

    let artifact_ref = output["artifact_ref"].as_str().unwrap();
    let services = runtime_tool_services(&context).unwrap();
    let projected = services
        .environment
        .artifacts()
        .unwrap()
        .get(artifact_ref)
        .await
        .unwrap();
    assert_eq!(projected, full_content.as_bytes());
    let observation_payload = services
        .environment
        .observations()
        .payload(output["observation_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(observation_payload.len(), 1024);
}

#[tokio::test]
async fn exact_edit_requires_unique_observed_current_text() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::write(workspace.root.join("note.txt"), "alpha beta\n").unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(EditFileTool::new()));

    let observed = registry
        .execute(
            "read_file",
            serde_json::json!({"path":"note.txt", "limit":64}),
            &context,
        )
        .await
        .unwrap();
    let observed: serde_json::Value = serde_json::from_str(&observed.content).unwrap();
    registry
        .execute(
            "edit_file",
            serde_json::json!({
                "path":"note.txt",
                "old_text":"beta",
                "new_text":"gamma",
                "observation_id": observed["observation_id"],
                "version": observed["version"]
            }),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.root.join("note.txt")).unwrap(),
        "alpha gamma\n"
    );

    std::fs::write(workspace.root.join("note.txt"), "same same\n").unwrap();
    let observed = registry
        .execute(
            "read_file",
            serde_json::json!({"path":"note.txt", "limit":64}),
            &context,
        )
        .await
        .unwrap();
    let observed: serde_json::Value = serde_json::from_str(&observed.content).unwrap();
    let error = registry
        .execute(
            "edit_file",
            serde_json::json!({
                "path":"note.txt",
                "old_text":"same",
                "new_text":"one",
                "observation_id": observed["observation_id"],
                "version": observed["version"]
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidInput { reason } if reason.contains("exactly once")));

    std::fs::write(workspace.root.join("note.txt"), "fresh value\n").unwrap();
    let observed = registry
        .execute(
            "read_file",
            serde_json::json!({"path":"note.txt", "limit":64}),
            &context,
        )
        .await
        .unwrap();
    let observed: serde_json::Value = serde_json::from_str(&observed.content).unwrap();
    std::fs::write(workspace.root.join("note.txt"), "externally changed\n").unwrap();
    let error = registry
        .execute(
            "edit_file",
            serde_json::json!({
                "path":"note.txt",
                "old_text":"fresh",
                "new_text":"stale",
                "observation_id": observed["observation_id"],
                "version": observed["version"]
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidInput { reason } if reason.contains("stale")));
    assert_eq!(
        std::fs::read_to_string(workspace.root.join("note.txt")).unwrap(),
        "externally changed\n"
    );
}

#[tokio::test]
async fn write_is_create_first_and_overwrite_is_explicit() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));

    registry
        .execute(
            "write_file",
            serde_json::json!({"path":"new.txt", "content":"first"}),
            &context,
        )
        .await
        .unwrap();
    let error = registry
        .execute(
            "write_file",
            serde_json::json!({"path":"new.txt", "content":"second"}),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidInput { reason } if reason.contains("create-first")));
    registry
        .execute(
            "write_file",
            serde_json::json!({"path":"new.txt", "content":"second", "mode":"overwrite"}),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.root.join("new.txt")).unwrap(),
        "second"
    );

    let large = vec![b'x'; 17 * 1024 * 1024];
    std::fs::write(workspace.root.join("large.txt"), &large).unwrap();
    let error = registry
        .execute(
            "write_file",
            serde_json::json!({
                "path":"large.txt",
                "content":"replacement",
                "mode":"overwrite"
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, ToolError::InvalidInput { reason } if reason.contains("file_mutation_capture_bytes"))
    );
    assert_eq!(
        std::fs::metadata(workspace.root.join("large.txt"))
            .unwrap()
            .len(),
        large.len() as u64
    );
}

#[tokio::test]
async fn discovery_continuation_then_observed_move_and_delete_are_bounded() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::create_dir_all(workspace.root.join("src")).unwrap();
    for name in ["a.rs", "b.rs", "c.txt"] {
        std::fs::write(workspace.root.join("src").join(name), name).unwrap();
    }
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ListDirectoryTool::new()));
    registry.register(Box::new(GlobPathsTool::new()));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    registry.register(Box::new(MovePathTool::new()));
    registry.register(Box::new(DeletePathTool::new()));

    let first = registry
        .execute(
            "list_directory",
            serde_json::json!({"path":"src", "limit":2}),
            &context,
        )
        .await
        .unwrap();
    let first: serde_json::Value = serde_json::from_str(&first.content).unwrap();
    assert_eq!(first["entries"].as_array().unwrap().len(), 2);
    let second = registry
        .execute(
            "list_directory",
            serde_json::json!({
                "path":"src",
                "limit":2,
                "continuation": first["continuation"]
            }),
            &context,
        )
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_str(&second.content).unwrap();
    assert_eq!(second["entries"].as_array().unwrap().len(), 1);
    let glob = registry
        .execute(
            "glob_paths",
            serde_json::json!({"path":"src", "pattern":"**/*.rs", "limit":10}),
            &context,
        )
        .await
        .unwrap();
    let glob: serde_json::Value = serde_json::from_str(&glob.content).unwrap();
    assert_eq!(glob["total_entries"], 2);

    let observed = registry
        .execute(
            "read_file",
            serde_json::json!({"path":"src/a.rs", "limit":64}),
            &context,
        )
        .await
        .unwrap();
    let observed: serde_json::Value = serde_json::from_str(&observed.content).unwrap();
    registry
        .execute(
            "move_path",
            serde_json::json!({
                "from":"src/a.rs",
                "to":"src/renamed.rs",
                "observation_id": observed["observation_id"],
                "version": observed["version"]
            }),
            &context,
        )
        .await
        .unwrap();
    let observed = registry
        .execute(
            "read_file",
            serde_json::json!({"path":"src/renamed.rs", "limit":64}),
            &context,
        )
        .await
        .unwrap();
    let observed: serde_json::Value = serde_json::from_str(&observed.content).unwrap();
    registry
        .execute(
            "delete_path",
            serde_json::json!({
                "path":"src/renamed.rs",
                "observation_id": observed["observation_id"],
                "version": observed["version"]
            }),
            &context,
        )
        .await
        .unwrap();
    assert!(!workspace.root.join("src/renamed.rs").exists());
}

#[tokio::test]
async fn directory_mutation_rejects_a_last_page_and_stale_discovery() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::create_dir_all(workspace.root.join("src")).unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        std::fs::write(workspace.root.join("src").join(name), name).unwrap();
    }
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ListDirectoryTool::new()));
    registry.register(Box::new(DeletePathTool::new()));
    registry.register(Box::new(MovePathTool::new()));

    let error = registry
        .execute(
            "delete_path",
            serde_json::json!({
                "path":".",
                "recursive":true,
                "observation_id":"untrusted",
                "version":"untrusted"
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, ToolError::PermissionDenied { reason } if reason.contains("workspace root"))
    );
    let error = registry
        .execute(
            "move_path",
            serde_json::json!({
                "from":".",
                "to":"moved-root",
                "observation_id":"untrusted",
                "version":"untrusted"
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, ToolError::PermissionDenied { reason } if reason.contains("workspace root"))
    );
    assert!(workspace.root.exists());

    #[cfg(windows)]
    {
        std::fs::create_dir(workspace.root.join("CaseSource")).unwrap();
        std::fs::write(workspace.root.join("CaseSource/kept.txt"), "kept").unwrap();
        let error = registry
            .execute(
                "move_path",
                serde_json::json!({
                    "from":"CaseSource",
                    "to":"casesource/nested",
                    "observation_id":"untrusted",
                    "version":"untrusted"
                }),
                &context,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, ToolError::InvalidInput { reason } if reason.contains("not be nested"))
        );
        assert!(!workspace.root.join("CaseSource/nested").exists());
        assert_eq!(
            std::fs::read_to_string(workspace.root.join("CaseSource/kept.txt")).unwrap(),
            "kept"
        );
    }

    let first = registry
        .execute(
            "list_directory",
            serde_json::json!({"path":"src", "recursive":true, "limit":2}),
            &context,
        )
        .await
        .unwrap();
    let first: serde_json::Value = serde_json::from_str(&first.content).unwrap();
    let last = registry
        .execute(
            "list_directory",
            serde_json::json!({
                "path":"src",
                "recursive":true,
                "limit":2,
                "continuation":first["continuation"]
            }),
            &context,
        )
        .await
        .unwrap();
    let last: serde_json::Value = serde_json::from_str(&last.content).unwrap();
    let error = registry
        .execute(
            "delete_path",
            serde_json::json!({
                "path":"src",
                "recursive":true,
                "observation_id":last["observation_id"],
                "version":last["version"]
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, ToolError::InvalidInput { reason } if reason.contains("complete current directory"))
    );
    assert!(workspace.root.join("src").exists());

    std::fs::write(workspace.root.join("src/d.rs"), "d.rs").unwrap();
    let error = registry
        .execute(
            "list_directory",
            serde_json::json!({
                "path":"src",
                "recursive":true,
                "limit":2,
                "continuation":first["continuation"]
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidInput { reason } if reason.contains("stale")));
}

#[tokio::test]
async fn checkpoint_diff_and_explicit_rewind_restore_selected_files() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::write(workspace.root.join("tracked.txt"), "before\n").unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(WorkspaceCheckpointTool::new()));
    registry.register(Box::new(WorkspaceDiffTool::new()));
    registry.register(Box::new(WorkspaceRewindTool::new()));

    let checkpoint = registry
        .execute(
            "workspace_checkpoint",
            serde_json::json!({"paths":["tracked.txt", "created.txt"]}),
            &context,
        )
        .await
        .unwrap();
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint.content).unwrap();
    std::fs::write(workspace.root.join("tracked.txt"), "after\n").unwrap();
    std::fs::write(workspace.root.join("created.txt"), "new\n").unwrap();
    let diff = registry
        .execute(
            "workspace_diff",
            serde_json::json!({"checkpoint_id":checkpoint["checkpoint_id"]}),
            &context,
        )
        .await
        .unwrap();
    let diff: serde_json::Value = serde_json::from_str(&diff.content).unwrap();
    assert_eq!(diff["changed_count"], 2);
    registry
        .execute(
            "workspace_rewind",
            serde_json::json!({
                "checkpoint_id":checkpoint["checkpoint_id"],
                "paths":["tracked.txt", "created.txt"]
            }),
            &context,
        )
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.root.join("tracked.txt")).unwrap(),
        "before\n"
    );
    assert!(!workspace.root.join("created.txt").exists());
}

#[tokio::test]
async fn rewind_preflights_every_selected_path_before_mutating_any_file() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::write(workspace.root.join("a.txt"), "before\n").unwrap();
    std::fs::write(workspace.root.join("z.bin"), [0xff, 0xfe]).unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(WorkspaceCheckpointTool::new()));
    registry.register(Box::new(WorkspaceRewindTool::new()));

    let checkpoint = registry
        .execute(
            "workspace_checkpoint",
            serde_json::json!({"paths":["a.txt", "z.bin"]}),
            &context,
        )
        .await
        .unwrap();
    let checkpoint: serde_json::Value = serde_json::from_str(&checkpoint.content).unwrap();
    std::fs::write(workspace.root.join("a.txt"), "after\n").unwrap();

    let duplicate = registry
        .execute(
            "workspace_rewind",
            serde_json::json!({
                "checkpoint_id":checkpoint["checkpoint_id"],
                "paths":["a.txt", "a.txt"]
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(duplicate, ToolError::InvalidInput { reason } if reason.contains("duplicated"))
    );
    assert_eq!(
        std::fs::read_to_string(workspace.root.join("a.txt")).unwrap(),
        "after\n"
    );

    let invalid = registry
        .execute(
            "workspace_rewind",
            serde_json::json!({
                "checkpoint_id":checkpoint["checkpoint_id"],
                "paths":["a.txt", "z.bin"]
            }),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(invalid, ToolError::InvalidInput { reason } if reason.contains("not UTF-8")));
    assert_eq!(
        std::fs::read_to_string(workspace.root.join("a.txt")).unwrap(),
        "after\n"
    );
}

#[tokio::test]
async fn checkpoint_rejects_aggregate_content_above_its_budget() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    std::fs::write(workspace.root.join("a.txt"), vec![b'a'; 5 * 1024 * 1024]).unwrap();
    std::fs::write(workspace.root.join("b.txt"), vec![b'b'; 4 * 1024 * 1024]).unwrap();
    let context = tool_context(&workspace, MemoryPaths::from_workspace(&workspace, 8), None);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(WorkspaceCheckpointTool::new()));

    let error = registry
        .execute(
            "workspace_checkpoint",
            serde_json::json!({"paths":["a.txt", "b.txt"]}),
            &context,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, ToolError::InvalidInput { reason } if reason.contains("checkpoint content byte limit"))
    );
}

#[tokio::test]
async fn background_shell_has_identity_progressive_cursors_and_typed_pty_absence() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(temp.path()).unwrap();
    let environment = Arc::new(InMemoryExecutionEnvironment::new(&workspace));
    let shell_program = if cfg!(windows) { "powershell" } else { "sh" };
    environment
        .processes()
        .set_response(
            shell_program,
            ProcessOutput {
                status_code: Some(0),
                stdout: b"abcdef".to_vec(),
                stderr: b"warning".to_vec(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
        )
        .await;
    let context = runtime_tool_context_with_environment(
        CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
        environment.clone(),
    );
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        ShellPolicy::default(),
    )));
    registry.register(Box::new(ShellOutputTool::new()));
    registry.register(Box::new(ShellTerminateTool::new()));
    registry.register(Box::new(ShellPtyTool::new()));

    let started = registry
        .execute(
            "run_shell",
            serde_json::json!({"command":"fixture", "background":true}),
            &context,
        )
        .await
        .unwrap();
    let started: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let process_id = started["process_id"].as_str().unwrap();
    let first = registry
        .execute(
            "shell_output",
            serde_json::json!({"process_id":process_id, "limit":3}),
            &context,
        )
        .await
        .unwrap();
    let first: serde_json::Value = serde_json::from_str(&first.content).unwrap();
    assert_eq!(first["stdout"], "abc");
    assert_eq!(first["output_complete"], true);
    assert_eq!(first["stdout_has_more"], true);
    let second = registry
        .execute(
            "shell_output",
            serde_json::json!({
                "process_id":process_id,
                "stdout_cursor":first["stdout_cursor"],
                "stderr_cursor":first["stderr_cursor"],
                "limit":3
            }),
            &context,
        )
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_str(&second.content).unwrap();
    assert_eq!(second["stdout"], "def");
    assert_eq!(second["stderr_has_more"], true);
    let error = registry
        .execute(
            "shell_output",
            serde_json::json!({"process_id":process_id, "stdout_cursor":99}),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::InvalidInput { reason } if reason.contains("cursor")));
    let error = registry
        .execute(
            "shell_output",
            serde_json::json!({"process_id":"unknown"}),
            &context,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ToolError::ExecutionFailed { reason } if reason.contains("not found")));
    registry
        .execute(
            "shell_terminate",
            serde_json::json!({"process_id":process_id}),
            &context,
        )
        .await
        .unwrap();
    assert!(!environment.capabilities().process_pty);
    assert_eq!(
        registry
            .descriptor("run_shell_pty")
            .unwrap()
            .capability
            .unwrap()
            .status,
        "unsupported"
    );
    let error = registry
        .execute(
            "run_shell_pty",
            serde_json::json!({"command":"fixture"}),
            &context,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, ToolError::PermissionDenied { reason } if reason.contains("typed unsupported"))
    );
}
