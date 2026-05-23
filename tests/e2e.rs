use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use rove::core::context::ContextManager;
use rove::core::engine::{Engine, EngineConfig};
use rove::core::events::StreamEvent;
use rove::core::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, Message, PlanStep, RunId, RunRequest,
    SessionId, TaskPlan, TaskState, TerminationReason, ToolContext, ToolSchema, Usage,
};
use rove::core::workspace::{Workspace, WorkspaceKind};
use rove::errors::{ModelError, ToolError};
use rove::hooks::{
    HookRegistry, PostRunHook, PostRunHookContext, PostToolHook, PostToolHookContext, PreToolHook,
};
use rove::interfaces::cli::oneshot::{run_oneshot, run_oneshot_with_cancel};
use rove::memory::durable::read_memory_index_sync;
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

struct RecordingModelClient {
    captured_messages: Arc<Mutex<Option<Vec<Message>>>>,
}

impl RecordingModelClient {
    fn new(captured_messages: Arc<Mutex<Option<Vec<Message>>>>) -> Self {
        Self { captured_messages }
    }
}

#[async_trait]
impl ModelClient for RecordingModelClient {
    fn stream(
        &self,
        messages: &[Message],
        _tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        *self.captured_messages.lock().unwrap() = Some(messages.to_vec());
        Box::pin(futures::stream::once(async {
            Ok(StreamChunk {
                delta: "done".to_string(),
                usage: Some(Usage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                }),
            })
        }))
    }

    fn model_id(&self) -> &str {
        "recording-model"
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

struct NeverCompletesTool;

#[async_trait]
impl Tool for NeverCompletesTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "wait_forever".to_string(),
            description: "A tool that stays pending until the run is cancelled.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            destructive: false,
        }
    }

    async fn execute(&self, _args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        futures::future::pending::<()>().await;
        unreachable!("pending tool should only finish by cancellation")
    }
}

struct BlockingPreHook;

#[async_trait]
impl PreToolHook for BlockingPreHook {
    async fn before_tool(
        &self,
        _ctx: &ToolContext<'_>,
        name: &str,
        _args: &serde_json::Value,
    ) -> Result<(), ToolError> {
        Err(ToolError::HookBlocked {
            reason: format!("{name} blocked by test hook"),
        })
    }
}

struct RecordingPostHook {
    records: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PostToolHook for RecordingPostHook {
    async fn after_tool(&self, ctx: &PostToolHookContext<'_>) -> Result<(), ToolError> {
        self.records
            .lock()
            .unwrap()
            .push(format!("{}:{}", ctx.name, ctx.result.output));
        Ok(())
    }
}

struct RecordingCancelTokenHook {
    states: Arc<Mutex<Vec<bool>>>,
}

#[async_trait]
impl PreToolHook for RecordingCancelTokenHook {
    async fn before_tool(
        &self,
        ctx: &ToolContext<'_>,
        _name: &str,
        _args: &serde_json::Value,
    ) -> Result<(), ToolError> {
        self.states
            .lock()
            .unwrap()
            .push(ctx.cancel_token.is_cancelled());
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct PostRunRecord {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    reason: TerminationReason,
    output: Option<String>,
    cancel_token_was_cancelled: bool,
}

struct RecordingPostRunHook {
    records: Arc<Mutex<Vec<PostRunRecord>>>,
}

#[async_trait]
impl PostRunHook for RecordingPostRunHook {
    async fn after_run(&self, ctx: &PostRunHookContext<'_>) -> anyhow::Result<()> {
        self.records.lock().unwrap().push(PostRunRecord {
            session_id: ctx.session_id,
            job_id: ctx.job_id,
            run_id: ctx.run_id,
            reason: ctx.reason.clone(),
            output: ctx.output.clone(),
            cancel_token_was_cancelled: ctx.cancel_token.is_cancelled(),
        });
        Ok(())
    }
}

struct NeverCompletesPostRunHook {
    entered: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl PostRunHook for NeverCompletesPostRunHook {
    async fn after_run(&self, _ctx: &PostRunHookContext<'_>) -> anyhow::Result<()> {
        self.entered.notify_one();
        futures::future::pending::<()>().await;
        unreachable!("pending post-run hook should only finish by cancellation")
    }
}

struct TimedOutPostRunHook {
    calls: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl PostRunHook for TimedOutPostRunHook {
    fn timeout(&self) -> Duration {
        Duration::from_millis(10)
    }

    async fn after_run(&self, _ctx: &PostRunHookContext<'_>) -> anyhow::Result<()> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        futures::future::pending::<()>().await;
        unreachable!("pending post-run hook should only finish by timeout")
    }
}

struct PanickingPostRunHook;

#[async_trait]
impl PostRunHook for PanickingPostRunHook {
    async fn after_run(&self, _ctx: &PostRunHookContext<'_>) -> anyhow::Result<()> {
        panic!("post-run panic from test hook")
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
    approval_decision: ApprovalDecision,
) -> Engine {
    let model = Box::new(FakeModelClient::new(responses));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FakeDestructiveTool));
    let context_manager = ContextManager::new("You are a test agent.".to_string());
    let config = EngineConfig {
        max_steps: 2,
        plan_enabled: false,
    };
    Engine::with_workspace_and_approval_decision(
        model,
        registry,
        context_manager,
        config,
        workspace,
        approval_policy,
        approval_decision,
    )
}

fn build_engine_with_counting_tool_and_hooks(
    responses: Vec<String>,
    workspace: Workspace,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    hooks: HookRegistry,
) -> Engine {
    let model = Box::new(FakeModelClient::new(responses));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CountingTool { calls }));
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
        ApprovalPolicy::Auto,
    )
    .with_hooks(hooks)
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
        cancel_token: CancellationToken::new(),
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
        cancel_token: CancellationToken::new(),
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
        ApprovalDecision::Reject,
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
async fn approved_destructive_tool_runs_when_policy_is_ask() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let engine = build_engine_with_destructive_tool(
        vec![
            r#"{"tool":"danger","args":{}}"#.to_string(),
            "finished".to_string(),
        ],
        workspace,
        ApprovalPolicy::Ask,
        ApprovalDecision::Approve,
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
            StreamEvent::ToolCallCompleted {
                result,
                ..
            } if result.output == "should never run"
        )
    }));
}

#[tokio::test]
async fn rejected_destructive_tool_does_not_run_when_policy_is_ask() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let engine = build_engine_with_destructive_tool(
        vec![
            r#"{"tool":"danger","args":{}}"#.to_string(),
            "blocked".to_string(),
        ],
        workspace,
        ApprovalPolicy::Ask,
        ApprovalDecision::Reject,
    );

    let events = collect_events(&engine, "run danger").await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ToolCallFailed {
                error: ToolError::PermissionDenied { .. },
                ..
            }
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ToolCallCompleted {
                result,
                ..
            } if result.output == "should never run"
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
        cancel_token: CancellationToken::new(),
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
async fn empty_hook_registry_preserves_tool_result() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CountingTool {
        calls: calls.clone(),
    }));

    let executor = rove::core::executor::Executor::with_hooks(&registry, HookRegistry::default());
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
    };

    let result = executor
        .run(
            &ctx,
            "counting",
            serde_json::json!({"path": "src/lib.rs"}),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "executed");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn pre_tool_hook_can_block_before_tool_runs() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CountingTool {
        calls: calls.clone(),
    }));
    let hooks = HookRegistry::default().with_pre_tool(Box::new(BlockingPreHook));
    let executor = rove::core::executor::Executor::with_hooks(&registry, hooks);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
    };

    let err = executor
        .run(
            &ctx,
            "counting",
            serde_json::json!({"path": "src/lib.rs"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::HookBlocked { reason } if reason.contains("counting blocked")
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pre_tool_hook_receives_cancellation_token_in_context() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let states = Arc::new(Mutex::new(Vec::new()));

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CountingTool { calls }));
    let hooks = HookRegistry::default().with_pre_tool(Box::new(RecordingCancelTokenHook {
        states: states.clone(),
    }));
    let executor = rove::core::executor::Executor::with_hooks(&registry, hooks);
    let cancel = CancellationToken::new();
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: cancel.clone(),
    };
    cancel.cancel();

    executor
        .run(
            &ctx,
            "counting",
            serde_json::json!({"path": "src/lib.rs"}),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(states.lock().unwrap().as_slice(), [true]);
}

#[tokio::test]
async fn post_tool_hook_observes_successful_tool_result() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let records = Arc::new(Mutex::new(Vec::new()));

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(CountingTool {
        calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    }));
    let hooks = HookRegistry::default().with_post_tool(Box::new(RecordingPostHook {
        records: records.clone(),
    }));
    let executor = rove::core::executor::Executor::with_hooks(&registry, hooks);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
    };

    let result = executor
        .run(
            &ctx,
            "counting",
            serde_json::json!({"path": "src/lib.rs"}),
            CallId::new(),
        )
        .await
        .unwrap();

    assert_eq!(result.output, "executed");
    assert_eq!(records.lock().unwrap().as_slice(), ["counting:executed"]);
}

#[tokio::test]
async fn post_run_hook_observes_completed_run_context() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let hooks = HookRegistry::default().with_post_run(Box::new(RecordingPostRunHook {
        records: records.clone(),
    }));
    let engine = build_test_engine(vec!["done".to_string()]).with_hooks(hooks);
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "finish".to_string(),
        resume_state: None,
    };

    let events = collect_events_with_request(&engine, req.clone()).await;

    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some(output),
        }) if output == "done"
    ));
    let records = records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, req.session_id);
    assert_eq!(records[0].job_id, req.job_id);
    assert_eq!(records[0].run_id, req.run_id);
    assert_eq!(records[0].reason, TerminationReason::Final);
    assert_eq!(records[0].output.as_deref(), Some("done"));
    assert!(!records[0].cancel_token_was_cancelled);
}

#[tokio::test]
async fn post_run_hook_wait_is_cancelled_before_stream_closes() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let hooks = HookRegistry::default().with_post_run(Box::new(NeverCompletesPostRunHook {
        entered: entered.clone(),
    }));
    let engine = build_test_engine(vec!["done".to_string()]).with_hooks(hooks);
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "finish".to_string(),
        resume_state: None,
    };
    let cancel = CancellationToken::new();
    let mut stream = engine.run_with_cancel(req, None, cancel.clone());

    let mut saw_terminal_event = false;
    while let Some(event) = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("stream should reach a terminal event")
    {
        if matches!(
            event,
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                ..
            }
        ) {
            saw_terminal_event = true;
            break;
        }
    }

    assert!(
        saw_terminal_event,
        "run should complete before post-run hooks"
    );
    let next = stream.next();
    futures::pin_mut!(next);
    tokio::select! {
        _ = entered.notified() => {
            cancel.cancel();
        }
        result = &mut next => {
            panic!("stream should wait for post-run hook before closing, got {result:?}");
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => {
            panic!("post-run hook should start after the terminal event");
        }
    }

    let completed = tokio::time::timeout(Duration::from_secs(2), next)
        .await
        .expect("cancelled post-run hook should let the stream close promptly");
    assert!(completed.is_none());
}

#[tokio::test]
async fn timed_out_post_run_hook_does_not_block_later_hooks() {
    let timed_out_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let records = Arc::new(Mutex::new(Vec::new()));
    let hooks = HookRegistry::default()
        .with_post_run(Box::new(TimedOutPostRunHook {
            calls: timed_out_calls.clone(),
        }))
        .with_post_run(Box::new(RecordingPostRunHook {
            records: records.clone(),
        }));
    let engine = build_test_engine(vec!["done".to_string()]).with_hooks(hooks);
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "finish".to_string(),
        resume_state: None,
    };

    let events = tokio::time::timeout(
        Duration::from_secs(2),
        collect_events_with_request(&engine, req),
    )
    .await
    .expect("timed-out post-run hook should not keep the stream open");

    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some(output),
        }) if output == "done"
    ));
    assert_eq!(timed_out_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(records.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn panicking_post_run_hook_does_not_block_later_hooks() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let hooks = HookRegistry::default()
        .with_post_run(Box::new(PanickingPostRunHook))
        .with_post_run(Box::new(RecordingPostRunHook {
            records: records.clone(),
        }));
    let engine = build_test_engine(vec!["done".to_string()]).with_hooks(hooks);
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "finish".to_string(),
        resume_state: None,
    };

    let events = tokio::time::timeout(
        Duration::from_secs(2),
        collect_events_with_request(&engine, req),
    )
    .await
    .expect("panicking post-run hook should be isolated");

    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some(output),
        }) if output == "done"
    ));
    assert_eq!(records.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn engine_runs_tool_calls_through_pre_tool_hooks() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let engine = build_engine_with_counting_tool_and_hooks(
        vec![
            r#"{"tool":"counting","args":{"path":"src/lib.rs"}}"#.to_string(),
            "blocked".to_string(),
        ],
        workspace,
        calls.clone(),
        HookRegistry::default().with_pre_tool(Box::new(BlockingPreHook)),
    );

    let events = collect_events(&engine, "run counting").await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ToolCallFailed {
                error: ToolError::HookBlocked { reason },
                ..
            } if reason.contains("counting blocked")
        )
    }));
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
async fn latest_task_state_rejects_unsupported_schema_version() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = rove::state::store::StateStore::new(tmp.path());

    let state = TaskState {
        schema_version: 99,
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "future state".to_string(),
        step: 1,
        history: vec![],
        summary: None,
        plan: None,
    };

    store.write_task_state(&state).await.unwrap();
    let err = store.load_latest_task_state().await.unwrap_err();

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string()
            .contains("unsupported task_state schema_version 99")
    );
}

#[tokio::test]
async fn list_resumable_task_states_filters_by_session_and_newest_first() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = rove::state::store::StateStore::new(tmp.path());
    let target_session = SessionId::new();
    let other_session = SessionId::new();

    let older = TaskState {
        schema_version: 1,
        session_id: target_session,
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "older".to_string(),
        step: 1,
        history: vec![],
        summary: None,
        plan: None,
    };
    let unrelated = TaskState {
        schema_version: 1,
        session_id: other_session,
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "other".to_string(),
        step: 1,
        history: vec![],
        summary: None,
        plan: None,
    };
    let newer = TaskState {
        schema_version: 1,
        session_id: target_session,
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "newer".to_string(),
        step: 2,
        history: vec![],
        summary: None,
        plan: None,
    };

    store.write_task_state(&older).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    store.write_task_state(&unrelated).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    store.write_task_state(&newer).await.unwrap();

    let states = store
        .list_resumable_task_states(target_session)
        .await
        .unwrap();

    assert_eq!(states.len(), 2);
    assert_eq!(states[0].goal, "newer");
    assert_eq!(states[1].goal, "older");
    assert!(
        states
            .iter()
            .all(|state| state.session_id == target_session)
    );
}

#[tokio::test]
async fn list_task_states_returns_all_snapshots_newest_first() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = rove::state::store::StateStore::new(tmp.path());

    let older = TaskState {
        schema_version: 1,
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "older".to_string(),
        step: 1,
        history: vec![],
        summary: None,
        plan: None,
    };
    let newer = TaskState {
        schema_version: 1,
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "newer".to_string(),
        step: 2,
        history: vec![],
        summary: None,
        plan: None,
    };

    store.write_task_state(&older).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    store.write_task_state(&newer).await.unwrap();

    let states = store.list_task_states().await.unwrap();

    assert_eq!(states.len(), 2);
    assert_eq!(states[0].goal, "newer");
    assert_eq!(states[1].goal, "older");
}

#[tokio::test]
async fn load_task_state_reads_exact_run_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = rove::state::store::StateStore::new(tmp.path());
    let session_id = SessionId::new();
    let target_run_id = RunId::new();
    let other_run_id = RunId::new();

    let target = TaskState {
        schema_version: 1,
        session_id,
        job_id: JobId::new(),
        run_id: target_run_id,
        goal: "target".to_string(),
        step: 7,
        history: vec![user_message("target")],
        summary: Some("target summary".to_string()),
        plan: None,
    };
    let other = TaskState {
        schema_version: 1,
        session_id,
        job_id: JobId::new(),
        run_id: other_run_id,
        goal: "other".to_string(),
        step: 1,
        history: vec![],
        summary: None,
        plan: None,
    };

    store.write_task_state(&target).await.unwrap();
    store.write_task_state(&other).await.unwrap();

    let loaded = store.load_task_state(target_run_id).await.unwrap();

    assert_eq!(loaded.run_id, target_run_id);
    assert_eq!(loaded.goal, "target");
    assert_eq!(loaded.step, 7);
    assert_eq!(loaded.summary.as_deref(), Some("target summary"));
}

#[tokio::test]
async fn load_task_state_returns_not_found_for_missing_run() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = rove::state::store::StateStore::new(tmp.path());

    let err = store.load_task_state(RunId::new()).await.unwrap_err();

    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn start_run_binds_identity_and_filesystem_paths() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let run_id = RunId::new();
    let handle = store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();

    assert_eq!(handle.run_id, run_id);
    assert_eq!(
        handle.run_dir,
        tmp.path().join("runs").join(run_id.to_string())
    );
    assert!(handle.run_dir.exists());
    assert!(handle.trace_writer.path().ends_with("trace.jsonl"));

    let request = handle.request("hello".to_string(), None);
    assert_eq!(request.run_id, run_id);
    assert_eq!(request.user_message, "hello");
}

#[tokio::test]
async fn oneshot_persists_final_output_as_task_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let engine = build_test_engine_with_workspace(vec!["done".to_string()], workspace.clone());

    run_oneshot(&engine, "say done".to_string(), run, None, &state_store).await;

    let task_state: TaskState = serde_json::from_slice(
        &std::fs::read(
            workspace
                .state_dir
                .join("runs")
                .join(run_id.to_string())
                .join("task_state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(task_state.summary.as_deref(), Some("done"));
    let run_dir = workspace.state_dir.join("runs").join(run_id.to_string());
    assert!(!run_dir.join("task_state.json.tmp").exists());
    assert!(!run_dir.join("report.json.tmp").exists());
}

#[tokio::test]
async fn oneshot_writes_task_state_before_completion() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let engine = build_test_engine_with_workspace(vec!["done".to_string()], workspace.clone());

    run_oneshot(&engine, "say done".to_string(), run, None, &state_store).await;

    let run_dir = workspace.state_dir.join("runs").join(run_id.to_string());
    let task_state: TaskState =
        serde_json::from_slice(&std::fs::read(run_dir.join("task_state.json")).unwrap()).unwrap();
    assert_eq!(task_state.goal, "say done");
    assert_eq!(task_state.summary.as_deref(), Some("done"));
    assert!(
        task_state
            .history
            .iter()
            .any(|message| message.content == "say done")
    );
    assert!(
        task_state
            .history
            .iter()
            .any(|message| message.content == "done")
    );
}

#[tokio::test]
async fn resumed_run_includes_session_summary_in_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let captured_messages = Arc::new(Mutex::new(None));
    let model = Box::new(RecordingModelClient::new(captured_messages.clone()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let context_manager = ContextManager::with_max_history("system".to_string(), 2);
    let config = EngineConfig {
        max_steps: 1,
        plan_enabled: false,
    };
    let engine = Engine::with_workspace(
        model,
        registry,
        context_manager,
        config,
        workspace,
        ApprovalPolicy::Auto,
    );

    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "current task".to_string(),
        resume_state: Some(TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "resume".to_string(),
            step: 0,
            history: vec![],
            summary: Some("previous session summary".to_string()),
            plan: None,
        }),
    };

    let events = collect_events_with_request(&engine, req).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::RunCompleted { .. }))
    );

    let messages = captured_messages.lock().unwrap().clone().unwrap();
    assert_eq!(messages[0].content, "system");
    assert_eq!(
        messages[1].content,
        "Session summary: previous session summary"
    );
    assert_eq!(messages.last().unwrap().content, "current task");
}

#[tokio::test]
async fn engine_includes_durable_memory_index_in_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Project Facts](topics/project-facts.md) - project memory\n",
    )
    .unwrap();
    let captured_messages = Arc::new(Mutex::new(None));
    let model = Box::new(RecordingModelClient::new(captured_messages.clone()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::with_max_history("system".to_string(), 2),
        EngineConfig {
            max_steps: 1,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );

    let events = collect_events(&engine, "current task").await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::RunCompleted { .. }))
    );

    let messages = captured_messages.lock().unwrap().clone().unwrap();
    assert_eq!(messages[0].content, "system");
    assert!(messages[1].content.starts_with("Durable memory:\n"));
    assert!(messages[1].content.contains("# rove Memory"));
    assert!(messages[1].content.contains("Project Facts"));
    assert_eq!(messages.last().unwrap().content, "current task");
}

#[test]
fn read_memory_index_sync_enforces_hard_limits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let mut content = "# rove Memory\n\n".to_string();
    for topic in 0..250 {
        content.push_str(&format!(
            "- [topic {topic:03}](topics/topic-{topic:03}.md) - {}\n",
            "x".repeat(140)
        ));
    }
    std::fs::write(memory_dir.join("MEMORY.md"), content).unwrap();

    let loaded = read_memory_index_sync(&workspace).unwrap().unwrap();

    assert!(loaded.starts_with("# rove Memory"));
    assert!(loaded.lines().count() <= 200);
    assert!(loaded.len() <= 25_000);
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
async fn planner_accepts_json_inside_markdown_fence() {
    let engine = build_planner_test_engine(vec![
        "```json\n{\"goal\":\"fix docs\",\"steps\":[{\"id\":\"1\",\"title\":\"inspect docs\"}]}\n```"
            .to_string(),
        "step 1 done".to_string(),
    ]);

    let events = collect_events(&engine, "fix the docs").await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::PlanCreated { .. })),
        "missing PlanCreated event"
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepCompleted { step, .. } if step.id == "1"
        )
    }));
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
async fn planner_emits_step_failed_for_malformed_step_output() {
    let engine = build_planner_test_engine(vec![
        r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#.to_string(),
        r#"{"tool":"echo","args":"wrong"}"#.to_string(),
        r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#.to_string(),
        "step 1 done".to_string(),
    ]);

    let events = collect_events(&engine, "fix the docs").await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepFailed { step, reason, .. }
                if step.id == "1" && reason.contains("tool arguments must be")
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepCompleted { step, .. } if step.id == "1"
        )
    }));
}

#[tokio::test]
async fn planner_replans_after_step_failure() {
    let engine = build_planner_test_engine(vec![
        r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#.to_string(),
        r#"{"tool":"echo","args":"wrong"}"#.to_string(),
        r#"{"goal":"fix docs","steps":[{"id":"2","title":"inspect docs without a tool"}]}"#
            .to_string(),
        "replanned step done".to_string(),
    ]);

    let events = collect_events(&engine, "fix the docs").await;

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanCreated { .. }))
            .count(),
        2
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { step, .. } if step.id == "2"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepCompleted { step, .. } if step.id == "2"
        )
    }));
}

#[tokio::test]
async fn oneshot_persists_replanned_task_state() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let engine = build_planner_test_engine(vec![
        r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#.to_string(),
        r#"{"tool":"echo","args":"wrong"}"#.to_string(),
        r#"{"goal":"fix docs","steps":[{"id":"2","title":"inspect docs without a tool"}]}"#
            .to_string(),
        "replanned step done".to_string(),
    ]);

    run_oneshot(&engine, "fix the docs".to_string(), run, None, &state_store).await;

    let task_state: TaskState = serde_json::from_slice(
        &std::fs::read(
            workspace
                .state_dir
                .join("runs")
                .join(run_id.to_string())
                .join("task_state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let plan = task_state
        .plan
        .expect("re-planned task state should persist a plan");

    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].id, "2");
    assert!(plan.steps[0].done);
    assert_eq!(plan.current_step, 1);
    assert!(
        task_state.history.iter().any(|message| {
            message
                .content
                .contains("Planned step failed: inspect docs")
        }),
        "task_state should preserve why the original plan was replaced"
    );
}

#[tokio::test]
async fn resumed_run_uses_persisted_replanned_task_state() {
    let mut plan = TaskPlan {
        goal: "fix docs".to_string(),
        steps: vec![PlanStep {
            id: "2".to_string(),
            title: "inspect docs without a tool".to_string(),
            done: false,
        }],
        current_step: 0,
    };
    plan.steps[0].done = false;

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
            history: vec![Message {
                role: rove::core::types::Role::User,
                content: "previous step failed and was re-planned".to_string(),
            }],
            summary: None,
            plan: Some(plan),
        }),
    };
    let engine = build_planner_test_engine(vec!["resumed replanned step done".to_string()]);

    let events = collect_events_with_request(&engine, req).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, StreamEvent::PlanCreated { .. })),
        "resume should use the persisted re-planned plan without drafting a new one"
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { step, .. } if step.id == "2"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepCompleted { step, .. } if step.id == "2"
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
        cancel_token: CancellationToken::new(),
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
        cancel_token: CancellationToken::new(),
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

#[tokio::test]
async fn shell_tool_rejects_empty_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
    };

    let err = executor
        .run(
            &ctx,
            "shell",
            serde_json::json!({"command": "   \t\n"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidInput { reason } if reason.contains("empty shell commands")
    ));
}

#[tokio::test]
async fn shell_tool_rejects_nul_byte_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
    };

    let err = executor
        .run(
            &ctx,
            "shell",
            serde_json::json!({"command": "echo before\u{0}echo after"}),
            CallId::new(),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        ToolError::InvalidInput { reason } if reason.contains("NUL bytes")
    ));
}

#[tokio::test]
async fn shell_tool_runs_non_empty_command_when_approved() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ShellTool::new(workspace.root.clone())));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = ToolContext {
        workspace: &workspace,
        approval_policy: ApprovalPolicy::Auto,
        cancel_token: CancellationToken::new(),
    };

    let command = if cfg!(windows) {
        "Write-Output shell-ok"
    } else {
        "printf shell-ok"
    };
    let result = executor
        .run(
            &ctx,
            "shell",
            serde_json::json!({"command": command}),
            CallId::new(),
        )
        .await
        .unwrap();

    assert!(result.output.contains("shell-ok"));
}

#[tokio::test]
async fn run_with_cancel_completes_cancelled_while_tool_is_waiting() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"wait_forever","args":{}}"#.to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NeverCompletesTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 2,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "wait".to_string(),
        resume_state: None,
    };
    let cancel = CancellationToken::new();
    let stream = engine.run_with_cancel(req, None, cancel.clone());
    futures::pin_mut!(stream);

    let mut saw_tool_start = false;
    while let Some(event) = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("stream should reach the tool call")
    {
        if matches!(event, StreamEvent::ToolCallStarted { name, .. } if name == "wait_forever") {
            saw_tool_start = true;
            cancel.cancel();
            break;
        }
    }

    assert!(saw_tool_start, "tool call should start before cancellation");
    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("cancelled run should finish promptly")
        .expect("cancelled run should emit a terminal event");
    assert!(matches!(
        event,
        StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Cancelled,
            output: None,
        }
    ));
}

#[tokio::test]
async fn oneshot_with_cancel_returns_cancelled_reason_and_report() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let engine = build_test_engine_with_workspace(vec!["should not run".to_string()], workspace);
    let cancel = CancellationToken::new();
    cancel.cancel();

    let reason = run_oneshot_with_cancel(
        &engine,
        "cancel before model".to_string(),
        run,
        None,
        &state_store,
        cancel,
    )
    .await;

    assert_eq!(reason, TerminationReason::Cancelled);
    let report_path = state_store.run_store.run_dir(&run_id).join("report.json");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(report_path).unwrap()).unwrap();
    assert_eq!(report["status"], "cancelled");
    assert_eq!(report["termination_reason"], "cancelled");
}

#[test]
fn run_stream_exposes_request_ids_immediately() {
    let engine = build_test_engine(vec!["done".to_string()]);
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "inspect ids".to_string(),
        resume_state: None,
    };

    let stream = engine.run(req.clone(), None);

    assert_eq!(stream.session_id(), req.session_id);
    assert_eq!(stream.job_id(), req.job_id);
    assert_eq!(stream.run_id(), req.run_id);
}

#[tokio::test]
async fn run_stream_cancel_completes_cancelled_while_tool_is_waiting() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"wait_forever","args":{}}"#.to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NeverCompletesTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 2,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "wait".to_string(),
        resume_state: None,
    };
    let mut stream = engine.run(req, None);

    let mut saw_tool_start = false;
    while let Some(event) = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("stream should reach the tool call")
    {
        if matches!(event, StreamEvent::ToolCallStarted { name, .. } if name == "wait_forever") {
            saw_tool_start = true;
            stream.cancel();
            break;
        }
    }

    assert!(saw_tool_start, "tool call should start before cancellation");
    let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("cancelled run should finish promptly")
        .expect("cancelled run should emit a terminal event");
    assert!(matches!(
        event,
        StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Cancelled,
            output: None,
        }
    ));
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
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let engine = build_test_engine_with_workspace(vec!["done".to_string()], workspace.clone());

    run_oneshot(&engine, "say done".to_string(), run, None, &state_store).await;

    let run_dir = workspace.state_dir.join("runs").join(run_id.to_string());
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
