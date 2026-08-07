use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rove_core::{CallId, Tool, ToolContext, ToolError, ToolOutput, ToolRegistry};
use rove_runtime::executor::Executor;
use rove_runtime::hooks::{
    HookRegistry, PostRunHookContext, PostToolHook, PostToolHookContext, PreToolHook, RunSummary,
    SessionMemoryHook,
};
use rove_runtime::memory::paths::MemoryPaths;
use rove_runtime::tools::runtime_context::runtime_tool_context;
use rove_runtime::types::{
    ApprovalPolicy, JobId, RunId, SessionId, TerminationReason, ToolDescriptor,
    ToolExecutionStatus, ToolMutation, ToolMutationOperation, ToolRiskLevel,
};
use rove_runtime::workspace::Workspace;
use tokio_util::sync::CancellationToken;

struct MutatingTool;

#[async_trait]
impl Tool for MutatingTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "write_note".to_string(),
            description: "Write a note".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            destructive: true,
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
        Ok(ToolOutput {
            content: "wrote note".to_string(),
            mutations: vec![ToolMutation {
                path: "notes/today.md".to_string(),
                operation: ToolMutationOperation::Update,
                diff: Some("+hello".to_string()),
            }],
        })
    }
}

struct RecordingPreHook {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PreToolHook for RecordingPreHook {
    async fn before_tool(
        &self,
        _ctx: &ToolContext<'_>,
        name: &str,
        _args: &serde_json::Value,
    ) -> Result<(), ToolError> {
        self.seen.lock().unwrap().push(name.to_string());
        Ok(())
    }
}

struct RecordingPostHook {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PostToolHook for RecordingPostHook {
    async fn after_tool(&self, ctx: &PostToolHookContext<'_>) -> Result<(), ToolError> {
        self.seen.lock().unwrap().push(ctx.result.output.clone());
        Ok(())
    }
}

#[tokio::test]
async fn executor_pipeline_records_metadata_and_runs_hooks() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MutatingTool));

    let pre = Arc::new(Mutex::new(Vec::new()));
    let post = Arc::new(Mutex::new(Vec::new()));
    let hooks = HookRegistry::default()
        .with_pre_tool(Box::new(RecordingPreHook {
            seen: Arc::clone(&pre),
        }))
        .with_post_tool(Box::new(RecordingPostHook {
            seen: Arc::clone(&post),
        }));

    let ctx = runtime_tool_context(
        CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    );

    let result = Executor::with_hooks(&registry, hooks)
        .run(&ctx, "write_note", serde_json::json!({}), CallId::new())
        .await
        .unwrap();

    assert_eq!(result.metadata.status, ToolExecutionStatus::Ok);
    assert_eq!(result.metadata.risk_level, ToolRiskLevel::High);
    assert!(!result.metadata.read_only);
    assert!(result.metadata.workspace_changed);
    assert_eq!(result.metadata.affected_paths, vec!["notes/today.md"]);
    assert_eq!(*pre.lock().unwrap(), vec!["write_note".to_string()]);
    assert_eq!(*post.lock().unwrap(), vec!["wrote note".to_string()]);
}

#[tokio::test]
async fn session_memory_hook_writes_summary_only_for_final_runs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_paths = MemoryPaths::from_workspace(&workspace, 8);
    std::fs::create_dir_all(&memory_paths.session_dir).unwrap();

    let session_id = SessionId::new();
    let mut summary = RunSummary::new("ship the hooks slice");
    summary.tools_used = vec!["write_note".to_string()];
    summary.tool_mutations = vec![ToolMutation {
        path: "notes/today.md".to_string(),
        operation: ToolMutationOperation::Update,
        diff: None,
    }];

    let hooks = HookRegistry::default().with_post_run(Box::new(SessionMemoryHook));
    hooks
        .run_post_run(&PostRunHookContext {
            workspace: &workspace,
            memory_paths: &memory_paths,
            session_id,
            job_id: JobId::new(),
            run_id: RunId::new(),
            reason: TerminationReason::Final,
            output: Some("done".to_string()),
            summary,
            cancel_token: CancellationToken::new(),
        })
        .await;

    let path = memory_paths.session_dir.join(format!("{session_id}.md"));
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.contains("# Session Summary"));
    assert!(content.contains("ship the hooks slice"));
    assert!(content.contains("write_note"));
    assert!(content.contains("notes/today.md"));
}

#[tokio::test]
async fn executor_rejects_invalid_object_arguments() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MutatingTool));
    let ctx = runtime_tool_context(
        CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        CancellationToken::new(),
    );

    let err = Executor::new(&registry)
        .run(
            &ctx,
            "write_note",
            serde_json::json!("not-an-object"),
            CallId::new(),
        )
        .await
        .unwrap_err();

    match err {
        ToolError::InvalidArgs { reason } => {
            assert!(reason.contains("JSON object"));
        }
        other => panic!("expected InvalidArgs, got {other:?}"),
    }
}
