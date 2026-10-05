use std::path::Path;

use rove_core::ToolError;
use rove_core::ToolRegistry;
use rove_runtime::executor::Executor;
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::tools::memory::{ReadMemoryTopicTool, SaveMemoryTool, UpdateMemoryIndexTool};
use rove_runtime::tools::runtime_context::runtime_tool_context;
use rove_runtime::types::{ApprovalPolicy, CallId, ToolContext};
use rove_runtime::workspace::Workspace;
use tokio_util::sync::CancellationToken;

fn tool_context(workspace: &Workspace) -> ToolContext<'_> {
    runtime_tool_context(
        CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        ApprovalPolicy::Never,
        None,
        CancellationToken::new(),
    )
}

#[tokio::test]
async fn save_memory_writes_topic_and_index_inside_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let result = executor
        .run(
            &ctx,
            "save_memory",
            serde_json::json!({
                "topic": "Project Conventions",
                "content": "Run cargo fmt before committing.",
                "type": "project"
            }),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "saved memory: project-conventions");

    let topic_path = workspace
        .root
        .join(".rove")
        .join("memory")
        .join("topics")
        .join("project-conventions.md");
    let index_path = workspace
        .root
        .join(".rove")
        .join("memory")
        .join("MEMORY.md");

    let topic = std::fs::read_to_string(topic_path).unwrap();
    assert!(topic.starts_with("---\n"));
    assert!(topic.contains("title: Project Conventions\n"));
    assert!(topic.contains("type: project\n"));
    assert!(topic.ends_with("Run cargo fmt before committing.\n"));

    let index = std::fs::read_to_string(index_path).unwrap();
    assert!(index.starts_with("# rove Memory\n"));
    assert!(index.contains("[Project Conventions](topics/project-conventions.md)"));
    assert!(index.contains("project memory"));
}

#[tokio::test]
async fn save_memory_writes_to_configured_workspace_state_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut workspace = Workspace::detect(tmp.path()).unwrap();
    let custom_state_dir = workspace.root.join("custom-state");
    workspace.state_dir = custom_state_dir.clone();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let result = executor
        .run(
            &ctx,
            "save_memory",
            serde_json::json!({
                "topic": "Project Conventions",
                "content": "Run cargo fmt before committing.",
                "type": "project"
            }),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "saved memory: project-conventions");
    assert!(
        custom_state_dir
            .join("memory")
            .join("topics")
            .join("project-conventions.md")
            .exists()
    );
    assert!(custom_state_dir.join("memory").join("MEMORY.md").exists());
    assert!(!workspace.root.join(".rove").join("memory").exists());
}

#[tokio::test]
async fn save_memory_writes_to_configured_durable_memory_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let durable_dir = workspace.root.join("configured-durable-memory");
    let memory_paths = MemoryPaths {
        session_dir: workspace.state_dir.join("memory").join("sessions"),
        durable_dir: durable_dir.clone(),
        recall_limit: 8,
    };

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new()));
    let executor = Executor::new(&registry);
    let ctx = runtime_tool_context(
        CallId::new(),
        &workspace,
        memory_paths,
        ApprovalPolicy::Never,
        None,
        CancellationToken::new(),
    );

    let result = executor
        .run(
            &ctx,
            "save_memory",
            serde_json::json!({
                "topic": "Project Conventions",
                "content": "Run cargo fmt before committing.",
                "type": "project"
            }),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "saved memory: project-conventions");
    assert!(
        durable_dir
            .join("topics")
            .join("project-conventions.md")
            .exists()
    );
    assert!(durable_dir.join("MEMORY.md").exists());
    assert!(!workspace.state_dir.join("memory").exists());
}

#[tokio::test]
async fn save_memory_rejects_unsafe_topic_without_writing() {
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
                "topic": "../outside",
                "content": "must not escape",
                "type": "project"
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidInput { reason } if reason.contains("safe topic")
    ));
    assert!(!workspace.root.join("outside.md").exists());
    assert!(!workspace.root.join(".rove").join("memory").exists());
}

#[tokio::test]
async fn save_memory_rejects_a_symlinked_topic_without_writing_outside() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    let topics_dir = memory_dir.join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    let outside = workspace.root.join("outside-topic.md");
    std::fs::write(&outside, "outside must stay unchanged").unwrap();
    if !create_test_file_symlink(&outside, &topics_dir.join("project-conventions.md")) {
        return;
    }

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let error = executor
        .run(
            &ctx,
            "save_memory",
            serde_json::json!({
                "topic": "Project Conventions",
                "content": "Run cargo fmt before committing.",
                "type": "project"
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ToolError::ExecutionFailed { reason } if reason.contains("regular file")
    ));
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "outside must stay unchanged"
    );
    assert!(!memory_dir.join("MEMORY.md").exists());
}

#[tokio::test]
async fn save_memory_rejects_secret_content_without_writing() {
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
                "topic": "Deployment Token",
                "content": "API key sk-test-secret should never be stored.",
                "type": "project"
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidInput { reason } if reason.contains("must not contain secrets")
    ));
    assert!(!workspace.root.join(".rove").join("memory").exists());
}

#[tokio::test]
async fn save_memory_rejects_transient_content_without_writing() {
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
                "topic": "Temporary Debug Output",
                "content": "Short-term log output from /tmp/current-run.log",
                "type": "reference"
            }),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidInput { reason } if reason.contains("stable long-term facts")
    ));
    assert!(!workspace.root.join(".rove").join("memory").exists());
}

#[tokio::test]
async fn save_memory_keeps_index_within_hard_limits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    for topic in 0..205 {
        executor
            .run(
                &ctx,
                "save_memory",
                serde_json::json!({
                    "topic": format!("topic {topic:03}"),
                    "content": "short durable memory",
                    "type": "reference"
                }),
                CallId::new(),
            )
            .await
            .unwrap();
    }

    let index_path = workspace
        .root
        .join(".rove")
        .join("memory")
        .join("MEMORY.md");
    let index = std::fs::read_to_string(index_path).unwrap();

    assert!(index.lines().count() <= 200);
    assert!(index.len() <= 25_000);
}

#[tokio::test]
async fn reindex_memory_rebuilds_index_from_existing_topics() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let topics_dir = workspace.root.join(".rove").join("memory").join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    std::fs::write(
        topics_dir.join("manual-topic.md"),
        "---\ntitle: Manual Topic\ntype: user\ncreated_at: 2026-05-23T00:00:00Z\nupdated_at: 2026-05-23T00:00:00Z\n---\n\nManual durable fact.\n",
    )
    .unwrap();
    std::fs::write(
        workspace
            .root
            .join(".rove")
            .join("memory")
            .join("MEMORY.md"),
        "# stale\n",
    )
    .unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(UpdateMemoryIndexTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let result = executor
        .run(&ctx, "reindex_memory", serde_json::json!({}), CallId::new())
        .await
        .unwrap();

    assert_eq!(result.output, "updated memory index");
    let index = std::fs::read_to_string(
        workspace
            .root
            .join(".rove")
            .join("memory")
            .join("MEMORY.md"),
    )
    .unwrap();
    assert!(index.starts_with("# rove Memory\n"));
    assert!(index.contains("[Manual Topic](topics/manual-topic.md)"));
    assert!(index.contains("user memory"));
}

#[tokio::test]
async fn reindex_memory_rejects_symlinked_topics_without_replacing_the_index() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    let topics_dir = memory_dir.join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    let original_index = "# original index\n";
    std::fs::write(memory_dir.join("MEMORY.md"), original_index).unwrap();
    let outside = workspace.root.join("outside-topic.md");
    std::fs::write(&outside, "outside must stay unchanged").unwrap();
    if !create_test_file_symlink(&outside, &topics_dir.join("linked.md")) {
        return;
    }

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(UpdateMemoryIndexTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let error = executor
        .run(&ctx, "reindex_memory", serde_json::json!({}), CallId::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ToolError::ExecutionFailed { reason } if reason.contains("regular file")
    ));
    assert_eq!(
        std::fs::read_to_string(memory_dir.join("MEMORY.md")).unwrap(),
        original_index
    );
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "outside must stay unchanged"
    );
}

#[tokio::test]
async fn reindex_memory_rejects_a_symlinked_index_without_writing_outside() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    std::fs::create_dir_all(memory_dir.join("topics")).unwrap();
    let outside = workspace.root.join("outside-index.md");
    std::fs::write(&outside, "outside index must stay unchanged").unwrap();
    if !create_test_file_symlink(&outside, &memory_dir.join("MEMORY.md")) {
        return;
    }

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(UpdateMemoryIndexTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let error = executor
        .run(&ctx, "reindex_memory", serde_json::json!({}), CallId::new())
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        ToolError::ExecutionFailed { reason } if reason.contains("regular file")
    ));
    assert_eq!(
        std::fs::read_to_string(outside).unwrap(),
        "outside index must stay unchanged"
    );
}

#[tokio::test]
async fn read_memory_reads_only_named_topic() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let topics_dir = workspace.root.join(".rove").join("memory").join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    std::fs::write(
        topics_dir.join("project-conventions.md"),
        "---\ntitle: Project Conventions\ntype: project\ncreated_at: 2026-05-23T00:00:00Z\nupdated_at: 2026-05-23T00:00:00Z\n---\n\nRead this durable topic.\n",
    )
    .unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadMemoryTopicTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let result = executor
        .run(
            &ctx,
            "read_memory",
            serde_json::json!({"name": "Project Conventions"}),
            CallId::new(),
        )
        .await
        .unwrap();

    assert!(result.output.starts_with("---\n"));
    assert!(result.output.contains("title: Project Conventions\n"));
    assert!(result.output.contains("Read this durable topic."));
}

#[tokio::test]
async fn read_memory_rejects_unsafe_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ReadMemoryTopicTool::new()));
    let executor = Executor::new(&registry);
    let ctx = tool_context(&workspace);

    let err = executor
        .run(
            &ctx,
            "read_memory",
            serde_json::json!({"name": "../outside"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidInput { reason } if reason.contains("safe topic")
    ));
}

#[cfg(unix)]
fn create_test_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn create_test_file_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}
