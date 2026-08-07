use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use tempfile::TempDir;

use async_trait::async_trait;
use futures::stream::BoxStream;
use rove_cli::cli::oneshot::run_oneshot;
use rove_core::ToolError;
use rove_core::ToolRegistry;
use rove_core::{Tool, ToolOutput};
use rove_models::ModelError;
use rove_models::{ModelClient, ModelEvent};
use rove_runtime::context::ContextManager;
use rove_runtime::engine::{Engine, EngineConfig};
use rove_runtime::executor::Executor;
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::state::store::StateStore;
use rove_runtime::tools::fs::{FsReadTool, FsWriteTool};
use rove_runtime::tools::memory::SaveMemoryTool;
use rove_runtime::tools::runtime_context::runtime_tool_context;
use rove_runtime::tools::search::{SearchCodePolicy, SearchCodeTool};
use rove_runtime::tools::shell::{ShellPolicy, ShellTool};
use rove_runtime::types::{
    ApprovalPolicy, CallId, Message, ModelToolSchema, RunId, SessionId, ToolContext,
    ToolDescriptor, Usage,
};
use rove_runtime::workspace::Workspace;
use tokio_util::sync::CancellationToken;

struct FakeModelClient {
    responses: Vec<String>,
    call_count: AtomicUsize,
}

impl FakeModelClient {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ModelClient for FakeModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let idx = self.call_count.fetch_add(1, Ordering::SeqCst);
        let response = self
            .responses
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "done".to_string());
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta { text: response }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
        ]))
    }

    fn model_id(&self) -> &str {
        "fake-model"
    }
}

struct NestedFixtureTool {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for NestedFixtureTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "nested_fixture".to_string(),
            description: "Test-only nested schema fixture.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "options": {
                        "type": "object",
                        "properties": {
                            "modes": {
                                "type": "array",
                                "items": { "type": "string" },
                                "minItems": 1
                            },
                            "level": {
                                "type": "integer",
                                "minimum": 1,
                                "maximum": 5
                            }
                        },
                        "required": ["modes", "level"],
                        "additionalProperties": false
                    }
                },
                "required": ["options"],
                "additionalProperties": false
            }),
            destructive: false,
            parallel_safe: false,
            capability_id: None,
            capability: None,
        }
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::text("should not run"))
    }
}

#[tokio::test]
async fn write_file_records_diff_metadata_in_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), rove_runtime::types::JobId::new(), run_id)
        .unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    let engine = Engine::with_workspace(
        Box::new(FakeModelClient::new(vec![
            r#"{"tool":"write_file","args":{"path":"note.txt","content":"hello"}}"#.to_string(),
            r#"{"tool":"write_file","args":{"path":"note.txt","content":"goodbye","mode":"overwrite"}}"#.to_string(),
            "done".to_string(),
        ])),
        registry,
        ContextManager::new("test".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    );

    run_oneshot(&engine, "write twice".to_string(), run, None, &state_store).await;

    let report_path = workspace
        .state_dir
        .join("runs")
        .join(run_id.to_string())
        .join("report.json");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    let mutations = report["tool_mutations"]
        .as_array()
        .expect("report should include tool mutations");

    assert_eq!(mutations.len(), 2);
    assert_eq!(mutations[1]["path"], "note.txt");
    assert_eq!(mutations[1]["operation"], "update");
    let diff = mutations[1]["diff"].as_str().unwrap();
    assert!(diff.contains("-hello"), "{diff}");
    assert!(diff.contains("+goodbye"), "{diff}");
}

#[tokio::test]
async fn read_file_rejects_parent_traversal() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let outside = tmp.path().parent().unwrap().join("outside-rove-test.txt");
    std::fs::write(&outside, "outside").unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "read_file",
            serde_json::json!({"path": "../outside-rove-test.txt"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::PermissionDenied { reason } if reason.contains("escapes workspace")
    ));
    let _ = std::fs::remove_file(outside);
}

#[tokio::test]
async fn read_file_rejects_symlink_escape_when_supported() {
    let (_tmp, workspace, outside_file) = workspace_with_outside_file();
    let link = workspace.root.join("linked-outside.txt");
    if !create_file_symlink(&outside_file, &link) {
        return;
    }

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "read_file",
            serde_json::json!({"path": "linked-outside.txt"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::PermissionDenied { reason } if reason.contains("escapes workspace")
    ));
}

#[tokio::test]
async fn write_file_rejects_existing_symlink_escape_when_supported() {
    let (_tmp, workspace, outside_file) = workspace_with_outside_file();
    let link = workspace.root.join("linked-outside.txt");
    if !create_file_symlink(&outside_file, &link) {
        return;
    }

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "write_file",
            serde_json::json!({"path": "linked-outside.txt", "content": "changed"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::PermissionDenied { reason } if reason.contains("escapes workspace")
    ));
    assert_eq!(std::fs::read_to_string(outside_file).unwrap(), "outside");
}

#[tokio::test]
async fn write_file_still_allows_new_normal_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    executor
        .run(
            &ctx,
            "write_file",
            serde_json::json!({"path": "nested/note.txt", "content": "inside"}),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(workspace.root.join("nested").join("note.txt")).unwrap(),
        "inside"
    );
}

#[tokio::test]
async fn search_code_finds_literal_match_inside_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    std::fs::create_dir_all(workspace.root.join("src")).unwrap();
    std::fs::write(
        workspace.root.join("src").join("main.rs"),
        "fn unique_search_marker() {}\n",
    )
    .unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SearchCodeTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let result = executor
        .run(
            &ctx,
            "search_code",
            serde_json::json!({
                "query": "unique_search_marker",
                "glob": "*.rs"
            }),
            CallId::new(),
        )
        .await
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["match_count"], 1);
    assert_eq!(output["matches"][0]["path"], "src/main.rs");
    assert_eq!(output["matches"][0]["line"], 1);
    assert!(
        output["matches"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unique_search_marker")
    );
}

#[tokio::test]
async fn search_code_rejects_parent_traversal() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SearchCodeTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "search_code",
            serde_json::json!({
                "query": "secret",
                "path": "../"
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::PermissionDenied { reason } | ToolError::InvalidInput { reason }
            if reason.contains("escapes workspace")
                || reason.contains("absolute")
                || reason.contains("empty")
                || reason.contains("invalid path")
    ));
}

#[tokio::test]
async fn search_code_rejects_symlink_escape_when_supported() {
    let (_tmp, workspace, outside_file) = workspace_with_outside_file();
    std::fs::write(&outside_file, "outside_secret_token").unwrap();
    let link = workspace.root.join("linked-outside.txt");
    if !create_file_symlink(&outside_file, &link) {
        return;
    }

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SearchCodeTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "search_code",
            serde_json::json!({
                "query": "outside_secret_token",
                "path": "linked-outside.txt"
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::PermissionDenied { reason } if reason.contains("escapes workspace")
    ));
}

#[tokio::test]
async fn search_code_respects_max_matches_cap() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut content = String::new();
    for i in 0..20 {
        content.push_str(&format!("marker_line_{i}\n"));
    }
    std::fs::write(workspace.root.join("many.txt"), content).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SearchCodeTool::with_policy(
        workspace.root.clone(),
        SearchCodePolicy {
            max_matches: 3,
            ..SearchCodePolicy::default()
        },
    )));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let result = executor
        .run(
            &ctx,
            "search_code",
            serde_json::json!({ "query": "marker_line_" }),
            CallId::new(),
        )
        .await
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["match_count"], 3);
    assert_eq!(output["truncated"], true);
    assert_eq!(output["truncated_reason"], "max_matches");
}

#[tokio::test]
async fn search_code_is_registered_in_default_tool_registry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let registry = rove_app_bootstrap::tool_registry(&workspace);
    assert!(registry.has("search_code"));
    assert!(registry.has("read_file"));
    assert!(registry.has("run_shell"));
}

#[tokio::test]
async fn run_shell_timeout_returns_structured_tool_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let command = if cfg!(windows) {
        "Start-Sleep -Milliseconds 200"
    } else {
        "sleep 0.2"
    };
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        ShellPolicy {
            timeout_ms: 50,
            ..ShellPolicy::default()
        },
    )));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "run_shell",
            serde_json::json!({"command": command}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, ToolError::Timeout { timeout_ms: 50 }));
}

#[tokio::test]
async fn run_shell_output_is_truncated_and_marked() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let command = if cfg!(windows) {
        "Write-Output ('x' * 200)"
    } else {
        "printf '%*s' 200 '' | tr ' ' x"
    };
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::with_policy(
        workspace.root.clone(),
        ShellPolicy {
            max_output_bytes: 32,
            ..ShellPolicy::default()
        },
    )));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let result = executor
        .run(
            &ctx,
            "run_shell",
            serde_json::json!({"command": command}),
            CallId::new(),
        )
        .await
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&result.output).unwrap();

    assert_eq!(output["stdout_truncated"], true);
    assert!(output["stdout"].as_str().unwrap().len() <= 32);
}

#[tokio::test]
async fn save_memory_invalid_type_is_rejected_by_schema_before_execution() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "save_memory",
            serde_json::json!({
                "topic": "Project",
                "content": "Stable fact.",
                "type": "invalid"
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidArgs { reason } if reason.contains("type") && reason.contains("enum")
    ));
    assert!(!workspace.state_dir.join("memory").exists());
}

#[tokio::test]
async fn nested_schema_validation_rejects_array_items_before_execution() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NestedFixtureTool {
        calls: calls.clone(),
    }));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "nested_fixture",
            serde_json::json!({
                "options": {
                    "modes": ["safe", 7],
                    "level": 3
                }
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidArgs { reason } if reason.contains("options.modes[1]")
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn nested_schema_validation_rejects_additional_properties_before_execution() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NestedFixtureTool {
        calls: calls.clone(),
    }));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "nested_fixture",
            serde_json::json!({
                "options": {
                    "modes": ["safe"],
                    "level": 3,
                    "extra": true
                }
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidArgs { reason }
            if reason.contains("options.extra") && reason.contains("additional")
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn nested_schema_validation_rejects_numeric_bounds_before_execution() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NestedFixtureTool {
        calls: calls.clone(),
    }));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "nested_fixture",
            serde_json::json!({
                "options": {
                    "modes": ["safe"],
                    "level": 9
                }
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidArgs { reason }
            if reason.contains("options.level") && reason.contains("maximum")
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

fn tool_context(workspace: &Workspace) -> ToolContext<'_> {
    runtime_tool_context(
        CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    )
}

fn workspace_with_outside_file() -> (TempDir, Workspace, PathBuf) {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace_root = tmp.path().join("workspace");
    std::fs::create_dir(&workspace_root).unwrap();
    let workspace = Workspace::detect(&workspace_root).unwrap();
    let outside_file = tmp.path().join("outside.txt");
    std::fs::write(&outside_file, "outside").unwrap();

    assert!(
        !outside_file
            .canonicalize()
            .unwrap()
            .starts_with(&workspace.root),
        "symlink escape fixture must target a file outside the workspace"
    );

    (tmp, workspace, outside_file)
}

#[cfg(unix)]
fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_file_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}
