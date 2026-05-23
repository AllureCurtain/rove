use rove::core::executor::Executor;
use rove::core::types::{ApprovalPolicy, CallId, ToolContext};
use rove::core::workspace::Workspace;
use rove::errors::ToolError;
use rove::tools::memory::SaveMemoryTool;
use rove::tools::registry::ToolRegistry;

#[tokio::test]
async fn save_memory_writes_topic_and_index_inside_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Never,
    };

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
async fn save_memory_rejects_unsafe_topic_without_writing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Never,
    };

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
async fn save_memory_keeps_index_within_hard_limits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SaveMemoryTool::new(workspace.root.clone())));
    let executor = Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Never,
    };

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
