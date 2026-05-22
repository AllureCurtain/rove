use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use rove::core::context::ContextManager;
use rove::core::engine::{Engine, EngineConfig};
use rove::core::events::StreamEvent;
use rove::core::types::{
    ApprovalPolicy, CallId, JobId, Message, PlanStep, RunId, RunRequest, SessionId, TaskPlan,
    TaskState, ToolContext, ToolSchema, Usage,
};
use rove::core::workspace::{Workspace, WorkspaceKind};
use rove::errors::{ModelError, ToolError};
use rove::interfaces::cli::oneshot::run_oneshot;
use rove::models::traits::{ModelClient, StreamChunk};
use rove::state::report::RunReport;
use rove::state::store::StateStore;
use rove::tools::echo::EchoTool;
use rove::tools::fs::{FsReadTool, FsWriteTool};
use rove::tools::registry::ToolRegistry;
use rove::tools::shell::ShellTool;
use rove::tools::traits::{Tool, ToolOutput};

fn user_message(content: &str) -> Message {
    Message {
        role: rove::core::types::Role::User,
        content: content.to_string(),
    }
}

/// A fake model client that returns predetermined responses.
struct FakeModelClient {
    responses: Vec<String>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl FakeModelClient {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ModelClient for FakeModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = self
            .responses
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "No more responses configured".to_string());

        Box::pin(futures::stream::once(async move {
            Ok(StreamChunk {
                delta: response,
                usage: Some(Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                }),
            })
        }))
    }

    fn model_id(&self) -> &str {
        "fake-model"
    }
}

struct FakeDestructiveTool;

#[async_trait]
impl Tool for FakeDestructiveTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "danger".to_string(),
            description: "A destructive tool used only for boundary tests.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            destructive: true,
        }
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput {
            content: "should never run".to_string(),
        })
    }
}

struct CountingTool {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl Tool for CountingTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "counting".to_string(),
            description: "A tool used to verify executor validation.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"]
            }),
            destructive: false,
        }
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ToolOutput {
            content: "executed".to_string(),
        })
    }
}

fn build_test_engine(responses: Vec<String>) -> Engine {
    let model = Box::new(FakeModelClient::new(responses));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let context_manager = ContextManager::new("You are a test agent.".to_string());
    let config = EngineConfig {
        max_steps: 5,
        plan_enabled: false,
    };
    Engine::new(model, registry, context_manager, config)
}

fn build_planner_test_engine(responses: Vec<String>) -> Engine {
    let model = Box::new(FakeModelClient::new(responses));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let context_manager = ContextManager::new("You are a test agent.".to_string());
    let config = EngineConfig {
        max_steps: 5,
        plan_enabled: true,
    };
    Engine::new(model, registry, context_manager, config)
}

fn build_test_engine_with_workspace(responses: Vec<String>, workspace: Workspace) -> Engine {
    let model = Box::new(FakeModelClient::new(responses));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let context_manager = ContextManager::new("You are a test agent.".to_string());
    let config = EngineConfig {
        max_steps: 5,
        plan_enabled: false,
    };
    Engine::with_workspace(
        model,
        registry,
        context_manager,
        config,
        workspace,
        ApprovalPolicy::Auto,
    )
}

fn build_engine_with_destructive_tool(
    responses: Vec<String>,
    workspace: Workspace,
    approval_policy: ApprovalPolicy,
) -> Engine {
    let model = Box::new(FakeModelClient::new(responses));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FakeDestructiveTool));
    let context_manager = ContextManager::new("You are a test agent.".to_string());
    let config = EngineConfig {
        max_steps: 2,
        plan_enabled: false,
    };
    Engine::with_workspace(
        model,
        registry,
        context_manager,
        config,
        workspace,
        approval_policy,
    )
}

/// Collect all events from a run into a Vec.
async fn collect_events(engine: &Engine, message: &str) -> Vec<StreamEvent> {
    let stream = engine.ask(message.to_string(), None);
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

async fn collect_events_with_request(engine: &Engine, req: RunRequest) -> Vec<StreamEvent> {
    let stream = engine.run(req, None);
    futures::pin_mut!(stream);
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn destructive_tool_is_blocked_when_policy_is_never() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FakeDestructiveTool));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Never,
    };

    let err = executor
        .run(&ctx, "danger", serde_json::json!({}), CallId::new())
        .await
        .unwrap_err();

    assert!(matches!(err, ToolError::PermissionDenied { .. }));
}

#[tokio::test]
async fn destructive_tool_requires_explicit_approval_when_policy_is_ask() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FakeDestructiveTool));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Ask,
    };

    let err = executor
        .run(&ctx, "danger", serde_json::json!({}), CallId::new())
        .await
        .unwrap_err();

    assert!(matches!(err, ToolError::PermissionDenied { .. }));
}

#[tokio::test]
async fn engine_emits_approval_needed_before_blocking_destructive_tool() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let engine = build_engine_with_destructive_tool(
        vec![
            r#"{"tool":"danger","args":{}}"#.to_string(),
            "blocked".to_string(),
        ],
        workspace,
        ApprovalPolicy::Ask,
    );

    let events = collect_events(&engine, "run danger").await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ToolCallApprovalNeeded { name, .. } if name == "danger"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ToolCallFailed {
                error: ToolError::PermissionDenied { .. },
                ..
            }
        )
    }));
}

#[tokio::test]
async fn executor_rejects_wrong_argument_type_before_tool_runs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CountingTool {
        calls: calls.clone(),
    }));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
    };

    let err = executor
        .run(
            &ctx,
            "counting",
            serde_json::json!({"path": 123}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, ToolError::InvalidArgs { .. }));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn latest_task_state_is_loaded_for_resume() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = rove::state::store::StateStore::new(tmp.path());

    let state = TaskState {
        schema_version: 1,
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "continue".to_string(),
        step: 3,
        history: vec![user_message("continue")],
        summary: Some("working summary".to_string()),
        plan: None,
    };

    store.write_task_state(&state).await.unwrap();
    let loaded = store.load_latest_task_state().await.unwrap().unwrap();
    assert_eq!(loaded.step, 3);
    assert_eq!(loaded.summary.as_deref(), Some("working summary"));
}

#[tokio::test]
async fn planner_persists_steps_and_resumes_mid_plan() {
    let engine = build_planner_test_engine(vec![
        r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"},{"id":"2","title":"write summary"}]}"#.to_string(),
        "step 1 done".to_string(),
        "step 2 done".to_string(),
    ]);

    let events = collect_events(&engine, "fix the docs").await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::PlanCreated { .. })),
        "missing PlanCreated event"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanStepCompleted { .. }))
            .count(),
        2
    );
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Final,
            ..
        })
    ));
}

#[tokio::test]
async fn planner_resumes_at_current_step() {
    let mut plan = TaskPlan {
        goal: "fix docs".to_string(),
        steps: vec![
            PlanStep {
                id: "1".to_string(),
                title: "inspect docs".to_string(),
                done: true,
            },
            PlanStep {
                id: "2".to_string(),
                title: "write summary".to_string(),
                done: false,
            },
        ],
        current_step: 1,
    };
    plan.steps[0].done = true;

    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "fix the docs".to_string(),
        resume_state: Some(TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "fix docs".to_string(),
            step: 1,
            history: vec![],
            summary: None,
            plan: Some(plan),
        }),
    };
    let engine = build_planner_test_engine(vec!["step 2 done".to_string()]);

    let events = collect_events_with_request(&engine, req).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, StreamEvent::PlanCreated { .. }))
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { step, .. } if step.id == "2"
        )
    }));
}

#[tokio::test]
async fn file_tools_read_and_write_inside_workspace() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    registry.register(Box::new(FsReadTool::new(workspace.root.clone())));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
    };

    executor
        .run(
            &ctx,
            "fs_write",
            serde_json::json!({"path": "note.txt", "content": "hello"}),
            CallId::new(),
        )
        .await
        .unwrap();
    let result = executor
        .run(
            &ctx,
            "fs_read",
            serde_json::json!({"path": "note.txt"}),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "hello");
}

#[tokio::test]
async fn shell_tool_is_blocked_when_policy_is_never() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Never,
    };

    let err = executor
        .run(
            &ctx,
            "shell",
            serde_json::json!({"command": "echo should-not-run"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, ToolError::PermissionDenied { .. }));
}

#[test]
fn context_manager_orders_memory_before_trimmed_history_and_current_message() {
    let context = ContextManager::with_max_history("system".to_string(), 2);
    let memory = vec![user_message("memory")];
    let history = vec![
        user_message("old"),
        user_message("recent one"),
        user_message("recent two"),
    ];

    let messages = context.build("current", &memory, &history);
    let contents: Vec<_> = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();

    assert_eq!(
        contents,
        vec!["system", "memory", "recent one", "recent two", "current"]
    );
}

#[test]
fn report_serializes_workspace_and_identity_metadata() {
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let workspace_root = std::path::PathBuf::from("D:/Study/project/agent/rove");

    let report = RunReport::new(
        session_id,
        job_id,
        run_id,
        workspace_root.clone(),
        WorkspaceKind::Folder,
        "fake-model".to_string(),
        rove::core::types::TerminationReason::Final,
    );

    let json = serde_json::to_value(&report).unwrap();

    assert_eq!(json["session_id"], session_id.to_string());
    assert_eq!(json["job_id"], job_id.to_string());
    assert_eq!(json["run_id"], run_id.to_string());
    assert_eq!(json["workspace_root"], workspace_root.display().to_string());
    assert_eq!(json["workspace_kind"], "folder");
    assert_eq!(json["model_id"], "fake-model");
    assert_eq!(json["status"], "success");
}

#[tokio::test]
async fn oneshot_report_includes_workspace_and_identity_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = state_store.new_run();
    let run_dir = state_store.run_store.run_dir(&run_id);
    let trace_writer = state_store.run_store.create_trace(&run_id).ok();
    let engine = build_test_engine_with_workspace(vec!["done".to_string()], workspace.clone());

    run_oneshot(
        &engine,
        "say done".to_string(),
        trace_writer,
        run_id,
        run_dir.clone(),
        None,
        &state_store,
    )
    .await;

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(run_dir.join("report.json")).unwrap())
            .unwrap();

    assert_eq!(report["run_id"], run_id.to_string());
    assert!(report["session_id"].as_str().is_some());
    assert!(report["job_id"].as_str().is_some());
    assert_eq!(
        report["workspace_root"],
        workspace.root.display().to_string()
    );
    assert_eq!(report["workspace_kind"], "folder");
    assert_eq!(report["model_id"], "fake-model");
}

#[tokio::test]
async fn run_started_event_uses_request_ids() {
    let engine = build_test_engine(vec!["done".to_string()]);
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "say hello".to_string(),
        resume_state: None,
    };

    let events = collect_events_with_request(&engine, req.clone()).await;

    match &events[0] {
        StreamEvent::RunStarted { run_id, job_id, .. } => {
            assert_eq!(*run_id, req.run_id);
            assert_eq!(*job_id, req.job_id);
        }
        other => panic!("Expected RunStarted, got {:?}", other),
    }
}

#[tokio::test]
async fn engine_produces_final_answer() {
    let engine = build_test_engine(vec!["Hello from the agent!".to_string()]);
    let events = collect_events(&engine, "say hello").await;

    // Should have: RunStarted, LlmChunk, LlmMessage, RunCompleted
    assert!(events.len() >= 3);

    // First event is RunStarted
    assert!(matches!(&events[0], StreamEvent::RunStarted { .. }));

    // Last event is RunCompleted with Final reason
    let last = events.last().unwrap();
    match last {
        StreamEvent::RunCompleted { reason, output } => {
            assert_eq!(*reason, rove::core::types::TerminationReason::Final);
            assert!(output.is_some());
            assert_eq!(output.as_deref().unwrap(), "Hello from the agent!");
        }
        _ => panic!("Expected RunCompleted, got {:?}", last),
    }
}

#[tokio::test]
async fn engine_handles_tool_call() {
    // First response: tool call JSON. Second response: final answer.
    let engine = build_test_engine(vec![
        r#"{"tool": "echo", "args": {"message": "ping"}}"#.to_string(),
        "The echo returned: ping".to_string(),
    ]);
    let events = collect_events(&engine, "echo ping").await;

    // Should contain ToolCallStarted and ToolCallCompleted
    let has_tool_start = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallStarted { name, .. } if name == "echo"));
    let has_tool_complete = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallCompleted { .. }));

    assert!(has_tool_start, "Missing ToolCallStarted event");
    assert!(has_tool_complete, "Missing ToolCallCompleted event");

    // Should end with RunCompleted::Final
    let last = events.last().unwrap();
    assert!(matches!(
        last,
        StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Final,
            ..
        }
    ));
}

#[tokio::test]
async fn engine_respects_step_limit() {
    // Model always returns tool calls — should hit step limit
    let responses: Vec<String> = (0..10)
        .map(|_| r#"{"tool": "echo", "args": {"message": "loop"}}"#.to_string())
        .collect();
    let engine = build_test_engine(responses);
    let events = collect_events(&engine, "keep going").await;

    let last = events.last().unwrap();
    assert!(matches!(
        last,
        StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::StepLimit,
            ..
        }
    ));
}

#[tokio::test]
async fn engine_handles_unknown_tool() {
    let engine = build_test_engine(vec![
        r#"{"tool": "nonexistent", "args": {}}"#.to_string(),
        "Sorry, that tool doesn't exist.".to_string(),
    ]);
    let events = collect_events(&engine, "use bad tool").await;

    let has_tool_failed = events
        .iter()
        .any(|e| matches!(e, StreamEvent::ToolCallFailed { .. }));
    assert!(has_tool_failed, "Missing ToolCallFailed event");
}

#[tokio::test]
async fn trace_writer_records_events() {
    let tmp = tempfile::TempDir::new().unwrap();
    let trace_path = tmp.path().join("trace.jsonl");
    let trace_writer = rove::state::trace::TraceWriter::new(tmp.path()).unwrap();

    let engine = build_test_engine(vec!["done".to_string()]);
    let stream = engine.ask("test".to_string(), Some(trace_writer));
    futures::pin_mut!(stream);
    while stream.next().await.is_some() {}

    // Verify trace file was written
    let content = std::fs::read_to_string(&trace_path).unwrap();
    assert!(!content.is_empty());

    // Each line should be valid JSON
    for line in content.lines() {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(parsed.get("type").is_some());
    }
}
