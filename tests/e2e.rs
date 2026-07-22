use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use rove::config::{AppConfig, AppConfigOverrides};
use rove::core::context::{ContextBudget, ContextManager};
use rove::core::engine::{Engine, EngineConfig};
use rove::core::events::StreamEvent;
use rove::core::execution::{
    PlanDecisionKind, PlanFinishReason, PlanIdentity, StepAttempt, StepCompletionBasis,
    StepLedgerState, StepRecord, StepRecordStatus,
};
use rove::core::types::{
    ApprovalDecision, ApprovalPolicy, CallId, JobId, Message, ModelToolSchema, PendingToolApproval,
    PendingUserInput, PlanStep, PromptCheckpoint, Role, RunId, RunRequest, SessionId, TaskPlan,
    TaskState, TerminationReason, ToolApprovalProvider, ToolApprovalRequest, ToolContext,
    ToolSchema, Usage, UserInputProvider, UserInputRequest,
};
use rove::core::workspace::{Workspace, WorkspaceKind};
use rove::errors::{ModelError, ToolError};
use rove::hooks::{
    HookRegistry, PostRunHook, PostRunHookContext, PostToolHook, PostToolHookContext, PreToolHook,
};
use rove::interfaces::cli::oneshot::{run_oneshot, run_oneshot_with_cancel};
use rove::memory::durable::read_memory_index_sync;
use rove::memory::paths::MemoryPaths;
use rove::models::traits::{ModelClient, ModelEvent};
use rove::state::report::RunReport;
use rove::state::store::StateStore;
use rove::tools::echo::EchoTool;
use rove::tools::fs::{FsReadTool, FsWriteTool};
use rove::tools::registry::ToolRegistry;
use rove::tools::request_input::RequestInputTool;
use rove::tools::runtime_context::{runtime_tool_context, runtime_tool_services};
use rove::tools::shell::ShellTool;
use rove::tools::traits::{Tool, ToolOutput};

fn user_message(content: &str) -> Message {
    Message::user(content)
}

fn sample_step_record(
    plan_id: &str,
    plan_revision_id: &str,
    step_id: &str,
    attempt: u32,
    status: StepRecordStatus,
) -> StepRecord {
    StepRecord {
        record_id: ulid::Ulid::new().to_string(),
        plan_id: plan_id.to_string(),
        plan_revision_id: plan_revision_id.to_string(),
        step_id: step_id.to_string(),
        attempt,
        status,
        started_at: "2026-07-20T00:00:00Z".to_string(),
        finished_at: "2026-07-20T00:00:01Z".to_string(),
        summary: "recorded step result".to_string(),
        completion_basis: StepCompletionBasis::DeterministicRule,
        evidence_refs: Vec::new(),
        tool_call_ids: Vec::new(),
        artifact_refs: Vec::new(),
        mutations: Vec::new(),
        model_turns_used: 1,
        tool_calls_used: 0,
        token_usage: Usage::default(),
        error_code: None,
        safe_error_summary: None,
        supersedes_record_id: None,
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
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = self
            .responses
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "No more responses configured".to_string());

        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta { text: response }),
            Ok(ModelEvent::Usage {
                usage: Usage {
                    prompt_tokens: 10,
                    completion_tokens: 5,
                    total_tokens: 15,
                    cached_tokens: 0,
                },
            }),
        ]))
    }

    fn model_id(&self) -> &str {
        "fake-model"
    }
}

struct StepFailureModelClient {
    call_count: std::sync::atomic::AtomicUsize,
}

impl StepFailureModelClient {
    fn new() -> Self {
        Self {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ModelClient for StepFailureModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let call = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match call {
            0 => Box::pin(futures::stream::iter([Ok(ModelEvent::TextDelta {
                text: r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#
                    .to_string(),
            })])),
            1 => Box::pin(futures::stream::iter([Err(ModelError::RequestFailed(
                "planned step model failed".to_string(),
            ))])),
            2 => Box::pin(futures::stream::iter([Ok(ModelEvent::TextDelta {
                text: r#"{"goal":"fix docs","steps":[{"id":"2","title":"inspect docs without a tool"}]}"#
                    .to_string(),
            })])),
            _ => Box::pin(futures::stream::iter([Ok(ModelEvent::TextDelta {
                text: "replanned step done".to_string(),
            })])),
        }
    }

    fn model_id(&self) -> &str {
        "step-failure-model"
    }
}

struct FailingAfterFirstCallModelClient {
    first_response: String,
    call_count: std::sync::atomic::AtomicUsize,
}

impl FailingAfterFirstCallModelClient {
    fn new(first_response: impl Into<String>) -> Self {
        Self {
            first_response: first_response.into(),
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ModelClient for FailingAfterFirstCallModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if idx == 0 {
            return Box::pin(futures::stream::iter([Ok(ModelEvent::TextDelta {
                text: self.first_response.clone(),
            })]));
        }
        Box::pin(futures::stream::iter([Err(ModelError::RequestFailed(
            "compaction model failed".to_string(),
        ))]))
    }

    fn model_id(&self) -> &str {
        "failing-after-first-call"
    }
}

struct CapturingFakeModelClient {
    responses: Vec<String>,
    call_count: std::sync::atomic::AtomicUsize,
    captured_messages: Arc<Mutex<Vec<Vec<Message>>>>,
}

impl CapturingFakeModelClient {
    fn new(responses: Vec<String>, captured_messages: Arc<Mutex<Vec<Vec<Message>>>>) -> Self {
        Self {
            responses,
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured_messages,
        }
    }
}

#[async_trait]
impl ModelClient for CapturingFakeModelClient {
    fn stream(
        &self,
        messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        self.captured_messages
            .lock()
            .unwrap()
            .push(messages.to_vec());
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = self
            .responses
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "No more responses configured".to_string());

        Box::pin(futures::stream::iter([Ok(ModelEvent::TextDelta {
            text: response,
        })]))
    }

    fn model_id(&self) -> &str {
        "capturing-fake-model"
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
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        *self.captured_messages.lock().unwrap() = Some(messages.to_vec());
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta {
                text: "done".to_string(),
            }),
            Ok(ModelEvent::Usage {
                usage: Usage {
                    prompt_tokens: 1,
                    completion_tokens: 1,
                    total_tokens: 2,
                    cached_tokens: 0,
                },
            }),
        ]))
    }

    fn model_id(&self) -> &str {
        "recording-model"
    }
}

struct NativeToolUseModelClient {
    call_count: std::sync::atomic::AtomicUsize,
}

impl NativeToolUseModelClient {
    fn new() -> Self {
        Self {
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ModelClient for NativeToolUseModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if idx == 0 {
            return Box::pin(futures::stream::iter([
                Ok(ModelEvent::ToolUseStart {
                    id: "native-call-1".to_string(),
                    name: "echo".to_string(),
                }),
                Ok(ModelEvent::ToolUseDone {
                    id: "native-call-1".to_string(),
                    name: "echo".to_string(),
                    args: serde_json::json!({ "message": "native hello" }),
                }),
                Ok(ModelEvent::Usage {
                    usage: Usage::default(),
                }),
            ]));
        }

        Box::pin(futures::stream::once(async {
            Ok(ModelEvent::TextDelta {
                text: "done with native tool".to_string(),
            })
        }))
    }

    fn model_id(&self) -> &str {
        "native-tool-use-model"
    }
}

struct ThinkingStatusModelClient;

#[async_trait]
impl ModelClient for ThinkingStatusModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::ThinkingDelta {
                text: "PRIVATE_CHAIN_OF_THOUGHT".to_string(),
            }),
            Ok(ModelEvent::TextDelta {
                text: "done".to_string(),
            }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
        ]))
    }

    fn model_id(&self) -> &str {
        "thinking-status-model"
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
            parallel_safe: false,
            capability: None,
        }
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::text("should never run"))
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
            parallel_safe: false,
            capability: None,
        }
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(ToolOutput::text("executed"))
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
            parallel_safe: false,
            capability: None,
        }
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        futures::future::pending::<()>().await;
        unreachable!("pending tool should only finish by cancellation")
    }
}

struct ProbeTool {
    name: &'static str,
    parallel_safe: bool,
    active: Arc<std::sync::atomic::AtomicUsize>,
    max_active: Arc<std::sync::atomic::AtomicUsize>,
}

impl ProbeTool {
    fn new(
        name: &'static str,
        parallel_safe: bool,
        active: Arc<std::sync::atomic::AtomicUsize>,
        max_active: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Self {
        Self {
            name,
            parallel_safe,
            active,
            max_active,
        }
    }
}

#[async_trait]
impl Tool for ProbeTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.to_string(),
            description: "A test probe tool for batch orchestration.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "delay_ms": { "type": "integer" }
                },
                "required": ["label", "delay_ms"]
            }),
            destructive: false,
            parallel_safe: self.parallel_safe,
            capability: None,
        }
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        let label = args
            .get("label")
            .and_then(|value| value.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: label".to_string(),
            })?;
        let delay_ms = args
            .get("delay_ms")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: delay_ms".to_string(),
            })?;
        let active_now = self
            .active
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        self.max_active
            .fetch_max(active_now, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        self.active
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);

        Ok(ToolOutput::text(label))
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

struct RecordingInputProvider {
    answer: String,
    requests: Arc<Mutex<Vec<(CallId, UserInputRequest)>>>,
}

struct PublicProviderInputTool;

#[async_trait]
impl Tool for PublicProviderInputTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "public_provider_input".to_string(),
            description: "Exercise the public user input provider API.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": { "type": "string" }
                },
                "required": ["prompt"]
            }),
            destructive: false,
            parallel_safe: false,
            capability: None,
        }
    }

    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext<'_>,
    ) -> Result<ToolOutput, ToolError> {
        let provider = runtime_tool_services(ctx)?
            .input_provider
            .as_ref()
            .ok_or_else(|| ToolError::ExecutionFailed {
                reason: "missing input provider".to_string(),
            })?;
        let prompt = args["prompt"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "missing prompt".to_string(),
            })?;
        let answer = provider
            .request_input(UserInputRequest {
                prompt: prompt.to_string(),
            })
            .await?;
        Ok(ToolOutput::text(answer))
    }
}

struct BlockingInputProvider {
    registered: Arc<Mutex<Vec<CallId>>>,
    dropped: Arc<AtomicBool>,
}

struct FailingApprovalRegistrationProvider;

#[async_trait]
impl ToolApprovalProvider for FailingApprovalRegistrationProvider {
    async fn begin_approval(
        &self,
        _request: ToolApprovalRequest,
    ) -> Result<PendingToolApproval, ToolError> {
        Err(ToolError::ExecutionFailed {
            reason: "approval registration failed".to_string(),
        })
    }
}

struct InputDropGuard(Arc<AtomicBool>);

impl Drop for InputDropGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl UserInputProvider for BlockingInputProvider {
    async fn begin_input(
        &self,
        input_id: CallId,
        _request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        self.registered.lock().unwrap().push(input_id);
        let guard = InputDropGuard(Arc::clone(&self.dropped));
        Ok(PendingUserInput::new(async move {
            let _guard = guard;
            std::future::pending::<Result<String, ToolError>>().await
        }))
    }
}

#[async_trait]
impl UserInputProvider for RecordingInputProvider {
    async fn begin_input(
        &self,
        input_id: CallId,
        request: UserInputRequest,
    ) -> Result<PendingUserInput, ToolError> {
        self.requests.lock().unwrap().push((input_id, request));
        let answer = self.answer.clone();
        Ok(PendingUserInput::new(async move { Ok(answer) }))
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

fn build_replanning_test_engine() -> Engine {
    let model = Box::new(StepFailureModelClient::new());
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    Engine::new(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: true,
        },
    )
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

fn tool_context(workspace: &Workspace, approval_policy: ApprovalPolicy) -> ToolContext<'_> {
    runtime_tool_context(
        CallId::new(),
        workspace,
        MemoryPaths::from_workspace(workspace, 8),
        approval_policy,
        None,
        CancellationToken::new(),
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

fn tool_lifecycle(events: &[StreamEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolCallStarted {
                name,
                args,
                tool_use_id,
                ..
            } => Some(format!("started:{name}:{args}:{tool_use_id:?}")),
            StreamEvent::ToolCallApprovalNeeded { name, args, .. } => {
                Some(format!("approval:{name}:{args}"))
            }
            StreamEvent::ToolCallCompleted { result, .. } => {
                Some(format!("completed:{}", result.output))
            }
            StreamEvent::ToolCallFailed { error, .. } => Some(format!("failed:{error}")),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn destructive_tool_is_blocked_when_policy_is_never() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FakeDestructiveTool));

    let executor = rove::core::executor::Executor::new(&registry);
    let ctx = tool_context(&workspace, ApprovalPolicy::Never);

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
    let ctx = tool_context(&workspace, ApprovalPolicy::Ask);

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

    let waiting_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                StreamEvent::ModelStatus { status, .. } if status == "waiting_for_approval"
            )
        })
        .expect("successful registration must preserve the waiting status event");
    let approval_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                StreamEvent::ToolCallApprovalNeeded { name, .. } if name == "danger"
            )
        })
        .expect("registered approval must publish its canonical event");
    assert!(waiting_index < approval_index);
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
async fn failed_approval_registration_emits_no_actionable_event_and_runs_no_tool() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"danger","args":{}}"#.to_string(),
        "blocked".to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FakeDestructiveTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 2,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Ask,
    )
    .with_approval_provider(Arc::new(FailingApprovalRegistrationProvider));

    let events = collect_events(&engine, "run danger").await;

    assert!(
        !events
            .iter()
            .any(|event| { matches!(event, StreamEvent::ToolCallApprovalNeeded { .. }) })
    );
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ModelStatus { status, .. } if status == "waiting_for_approval"
        )
    }));
    assert!(!events.iter().any(|event| {
        matches!(event, StreamEvent::ToolCallCompleted { result, .. } if result.output == "should never run")
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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
    let ctx = runtime_tool_context(
        CallId::new(),
        &workspace,
        MemoryPaths::from_workspace(&workspace, 8),
        ApprovalPolicy::Auto,
        None,
        cancel.clone(),
    );
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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
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
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let handle = store.start_run(session_id, job_id, run_id).unwrap();

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

    assert!(store.index.path().exists());
    let job = store
        .index
        .job_record(job_id)
        .unwrap()
        .expect("job should be indexed");
    assert_eq!(job.session_id, session_id);
    assert_eq!(job.status, "running");
    assert_eq!(job.run_id, Some(run_id));
    let run = store
        .index
        .run_record(run_id)
        .unwrap()
        .expect("run should be indexed");
    assert_eq!(run.session_id, session_id);
    assert_eq!(run.job_id, job_id);
    assert_eq!(run.status, "running");
    assert_eq!(run.run_dir, handle.run_dir);
    assert_eq!(run.trace_path, handle.trace_writer.path());
}

#[tokio::test]
async fn lazy_import_indexes_existing_task_state_artifacts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let state = TaskState {
        schema_version: 1,
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "legacy artifact".to_string(),
        step: 2,
        history: vec![user_message("legacy artifact")],
        summary: Some("legacy summary".to_string()),
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
    };
    let run_dir = tmp.path().join("runs").join(state.run_id.to_string());
    std::fs::create_dir_all(&run_dir).unwrap();
    let task_state_path = run_dir.join("task_state.json");
    std::fs::write(&task_state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();

    assert!(!store.index.path().exists());

    let states = store.list_task_states().await.unwrap();

    assert_eq!(states.len(), 1);
    assert_eq!(states[0].goal, "legacy artifact");
    assert!(store.index.path().exists());
    assert_eq!(
        store
            .index
            .task_state_path(state.run_id)
            .unwrap()
            .as_deref(),
        Some(task_state_path.as_path())
    );
    let records = store.index.list_task_state_records(None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].run_id, state.run_id);
    assert_eq!(records[0].path, task_state_path);
}

#[tokio::test]
async fn repair_index_explicitly_imports_legacy_task_state_artifacts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let state = TaskState {
        schema_version: 1,
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "repair me".to_string(),
        step: 3,
        history: vec![user_message("repair me")],
        summary: Some("legacy summary".to_string()),
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
    };
    let run_dir = tmp.path().join("runs").join(state.run_id.to_string());
    std::fs::create_dir_all(&run_dir).unwrap();
    let task_state_path = run_dir.join("task_state.json");
    std::fs::write(&task_state_path, serde_json::to_vec_pretty(&state).unwrap()).unwrap();

    let imported = store.repair_index().await.unwrap();

    assert_eq!(imported.task_state_count, 1);
    let records = store.index.list_task_state_records(None).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].run_id, state.run_id);
    assert_eq!(records[0].path, task_state_path);
}

#[tokio::test]
async fn repair_index_rebuilds_events_and_report_from_artifacts() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let run = store.start_run(session_id, job_id, run_id).unwrap();
    let started = StreamEvent::RunStarted {
        run_id,
        job_id,
        user_message: "repair trace".to_string(),
    };
    let completed = StreamEvent::RunCompleted {
        reason: TerminationReason::Final,
        output: Some("done".to_string()),
    };
    let record = sample_step_record(
        "repair-plan",
        "repair-revision",
        "repair-step",
        1,
        StepRecordStatus::Succeeded,
    );
    run.trace_writer.append(&started).unwrap();
    run.trace_writer
        .append(&StreamEvent::StepResult {
            record: Box::new(record.clone()),
        })
        .unwrap();
    run.trace_writer.append(&completed).unwrap();
    let step_ledger = StepLedgerState {
        active_plan_id: Some(record.plan_id.clone()),
        active_plan_revision_id: Some(record.plan_revision_id.clone()),
        active_plan_revision: 0,
        step_records: vec![record.clone()],
        active_step_attempt: None,
        plan_lifecycle: Default::default(),
    };
    let state = TaskState {
        schema_version: 1,
        session_id,
        job_id,
        run_id,
        goal: "repair trace".to_string(),
        step: 1,
        history: vec![user_message("repair trace")],
        summary: Some("done".to_string()),
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger,
    };
    store.write_task_state(&state).await.unwrap();
    let mut report = RunReport::new(
        session_id,
        job_id,
        run_id,
        tmp.path().to_path_buf(),
        WorkspaceKind::Folder,
        "fake".to_string(),
        TerminationReason::Final,
    );
    report.step_records.push(record);
    rove::state::report::write_report(&run.run_dir, &report).unwrap();
    std::fs::remove_file(store.index.path()).unwrap();

    let repaired = store.repair_index().await.unwrap();

    assert_eq!(repaired.task_state_count, 1);
    assert_eq!(repaired.event_count, 3);
    assert_eq!(repaired.report_count, 1);
    let indexed_events = store.index.event_records(run_id).unwrap();
    assert_eq!(indexed_events.len(), 3);
    assert_eq!(indexed_events[0].event_name, "run_started");
    assert_eq!(indexed_events[1].event_name, "step_result");
    assert_eq!(indexed_events[2].event_name, "run_completed");
    assert_eq!(store.index.last_event_seq(run_id).unwrap(), 3);
    let indexed_run = store.index.run_record(run_id).unwrap().unwrap();
    assert_eq!(indexed_run.status, "done");
    assert_eq!(indexed_run.last_event_seq, 3);
    assert!(store.index.report_record(run_id).unwrap().is_some());
    assert!(store.index.job_record(job_id).unwrap().is_some());
}

#[tokio::test]
async fn repair_index_reports_corrupted_trace_lines_without_aborting() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let run_dir = tmp.path().join("runs").join(run_id.to_string());
    std::fs::create_dir_all(&run_dir).unwrap();
    let state = TaskState {
        schema_version: 1,
        session_id,
        job_id,
        run_id,
        goal: "repair corrupted trace".to_string(),
        step: 1,
        history: vec![user_message("repair corrupted trace")],
        summary: None,
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
    };
    std::fs::write(
        run_dir.join("task_state.json"),
        serde_json::to_vec_pretty(&state).unwrap(),
    )
    .unwrap();
    let valid_event = serde_json::to_string(&StreamEvent::RunStarted {
        run_id,
        job_id,
        user_message: "repair corrupted trace".to_string(),
    })
    .unwrap();
    std::fs::write(
        run_dir.join("trace.jsonl"),
        format!("{valid_event}\nnot-json\n"),
    )
    .unwrap();

    let repaired = store.repair_index().await.unwrap();

    assert_eq!(repaired.task_state_count, 1);
    assert_eq!(repaired.event_count, 1);
    assert_eq!(repaired.corrupt_trace_line_count, 1);
    let indexed_events = store.index.event_records(run_id).unwrap();
    assert_eq!(indexed_events.len(), 1);
    assert_eq!(indexed_events[0].event_name, "run_started");
}

#[tokio::test]
async fn repair_index_does_not_cleanup_expired_state_rows() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let run_dir = tmp.path().join("runs").join(run_id.to_string());
    std::fs::create_dir_all(&run_dir).unwrap();
    store
        .index
        .record_run_started(
            session_id,
            job_id,
            run_id,
            &run_dir,
            &run_dir.join("trace.jsonl"),
        )
        .unwrap();
    store
        .index
        .set_job_ttl(job_id, Some("2000-01-01T00:00:00Z".to_string()))
        .unwrap();

    let imported = store.repair_index().await.unwrap();

    assert_eq!(imported.task_state_count, 0);
    assert!(store.index.job_record(job_id).unwrap().is_some());
    assert!(store.index.run_record(run_id).unwrap().is_some());
    assert!(run_dir.exists());
}

#[tokio::test]
async fn cleanup_expired_state_rows_removes_only_expired_entries() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let session_id = SessionId::new();

    let expired_job = JobId::new();
    let expired_run = RunId::new();
    let active_job = JobId::new();
    let active_run = RunId::new();
    let expired_state = TaskState {
        schema_version: 1,
        session_id,
        job_id: expired_job,
        run_id: expired_run,
        goal: "expired".to_string(),
        step: 1,
        history: vec![],
        summary: None,
        checkpoint: None,
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
    };
    store.write_task_state(&expired_state).await.unwrap();
    let run_dir = tmp.path().join("runs").join(active_run.to_string());
    store
        .index
        .record_run_started(
            session_id,
            active_job,
            active_run,
            &run_dir,
            &run_dir.join("trace2.jsonl"),
        )
        .unwrap();

    store
        .index
        .set_job_ttl(expired_job, Some("2000-01-01T00:00:00Z".to_string()))
        .unwrap();
    store
        .index
        .set_job_ttl(active_job, Some("2999-01-01T00:00:00Z".to_string()))
        .unwrap();

    let removed = store.cleanup_expired().await.unwrap();

    assert_eq!(removed.job_count, 1);
    assert_eq!(removed.run_count, 1);
    assert_eq!(removed.task_state_count, 1);
    assert!(store.index.job_record(expired_job).unwrap().is_none());
    assert!(store.index.run_record(expired_run).unwrap().is_none());
    assert!(store.index.job_record(active_job).unwrap().is_some());
    assert!(store.index.run_record(active_run).unwrap().is_some());
}

#[test]
fn trace_writer_indexes_appended_events() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = StateStore::new(tmp.path());
    let session_id = SessionId::new();
    let job_id = JobId::new();
    let run_id = RunId::new();
    let handle = store.start_run(session_id, job_id, run_id).unwrap();
    let started = StreamEvent::RunStarted {
        run_id,
        job_id,
        user_message: "trace me".to_string(),
    };
    let completed = StreamEvent::RunCompleted {
        reason: TerminationReason::Final,
        output: Some("done".to_string()),
    };

    handle.trace_writer.append(&started).unwrap();
    handle.trace_writer.append(&completed).unwrap();

    let events = store.index.event_records(run_id).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].seq, 1);
    assert_eq!(events[0].event_name, "run_started");
    assert_eq!(events[1].seq, 2);
    assert_eq!(events[1].event_name, "run_completed");
    assert_eq!(store.index.last_event_seq(run_id).unwrap(), 2);
    let run = store.index.run_record(run_id).unwrap().unwrap();
    assert_eq!(run.last_event_seq, 2);
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
async fn oneshot_persists_native_tool_use_structured_history() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        Box::new(NativeToolUseModelClient::new()),
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    );

    run_oneshot(
        &engine,
        "echo through native tool use".to_string(),
        run,
        None,
        &state_store,
    )
    .await;

    let task_state = state_store.load_task_state(run_id).await.unwrap();
    let assistant_with_tools = task_state
        .history
        .iter()
        .find(|message| !message.tool_calls.is_empty())
        .expect("persisted assistant message should keep native tool calls");
    assert_eq!(assistant_with_tools.tool_calls.len(), 1);
    assert_eq!(assistant_with_tools.tool_calls[0].id, "native-call-1");
    assert_eq!(assistant_with_tools.tool_calls[0].name, "echo");
    assert_eq!(
        assistant_with_tools.tool_calls[0].args["message"],
        "native hello"
    );

    let tool_result = task_state
        .history
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some("native-call-1"))
        .expect("persisted tool result should keep native tool-use id");
    assert_eq!(tool_result.content, "native hello");
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
            checkpoint: None,
            plan: None,
            runtime_identity: None,
            step_ledger: Default::default(),
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
async fn engine_includes_session_memory_file_in_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let session_id = SessionId::new();
    let sessions_dir = workspace.root.join(".rove").join("memory").join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::write(
        sessions_dir.join(format!("{session_id}.md")),
        "ongoing session preference",
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
    let req = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "current task".to_string(),
        resume_state: None,
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
        "Session summary: ongoing session preference"
    );
    assert_eq!(messages.last().unwrap().content, "current task");
}

#[tokio::test]
async fn engine_honors_configured_session_memory_dir_for_read_and_write() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let session_id = SessionId::new();
    let session_dir = workspace.root.join("configured-session-memory");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(
        session_dir.join(format!("{session_id}.md")),
        "configured session preference",
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
        workspace.clone(),
        ApprovalPolicy::Auto,
    )
    .with_memory_paths(rove::memory::paths::MemoryPaths {
        session_dir: session_dir.clone(),
        durable_dir: workspace.state_dir.join("memory"),
        recall_limit: 8,
    });
    let req = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "current task".to_string(),
        resume_state: None,
    };

    let events = collect_events_with_request(&engine, req).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::RunCompleted { .. }))
    );

    let messages = captured_messages.lock().unwrap().clone().unwrap();
    assert_eq!(
        messages[1].content,
        "Session summary: configured session preference"
    );
    let summary = std::fs::read_to_string(session_dir.join(format!("{session_id}.md"))).unwrap();
    assert!(summary.contains("- Goal: current task"));
    assert!(summary.contains("- Status: final"));
    assert!(summary.contains("- Output: done"));
    assert!(
        !workspace
            .state_dir
            .join("memory")
            .join("sessions")
            .join(format!("{session_id}.md"))
            .exists()
    );
}

#[tokio::test]
async fn engine_writes_final_output_to_session_memory_file() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let session_id = SessionId::new();
    let engine = build_test_engine_with_workspace(
        vec!["learned session summary".to_string()],
        workspace.clone(),
    );
    let req = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "finish".to_string(),
        resume_state: None,
    };

    let events = collect_events_with_request(&engine, req).await;
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some(output),
        }) if output == "learned session summary"
    ));

    let session_memory_path = workspace
        .state_dir
        .join("memory")
        .join("sessions")
        .join(format!("{session_id}.md"));
    let summary = std::fs::read_to_string(session_memory_path).unwrap();
    assert!(summary.contains("- Goal: finish"));
    assert!(summary.contains("- Status: final"));
    assert!(summary.contains("- Output: learned session summary"));
}

#[tokio::test]
async fn engine_writes_deterministic_session_summary_with_tool_activity() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let session_id = SessionId::new();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    registry.register(Box::new(FsWriteTool::new(workspace.root.clone())));
    let engine = Engine::with_workspace(
        Box::new(FakeModelClient::new(vec![
            r#"{"tool":"fs_write","args":{"path":"note.txt","content":"remember this"}}"#
                .to_string(),
            "all set".to_string(),
        ])),
        registry,
        ContextManager::new("system".to_string()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    );
    let req = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "write a note".to_string(),
        resume_state: None,
    };

    let events = collect_events_with_request(&engine, req).await;
    assert!(events.iter().any(|event| matches!(
        event,
        StreamEvent::ToolCallCompleted { result, .. }
            if result.mutations.iter().any(|mutation| mutation.path == "note.txt")
    )));

    let session_memory_path = workspace
        .state_dir
        .join("memory")
        .join("sessions")
        .join(format!("{session_id}.md"));
    let summary = std::fs::read_to_string(session_memory_path).unwrap();
    assert!(summary.contains("- Goal: write a note"));
    assert!(summary.contains("- Status: final"));
    assert!(summary.contains("- Output: all set"));
    assert!(summary.contains("- Tools used: fs_write"));
    assert!(summary.contains("- Files changed: note.txt (create)"));

    let captured_messages = Arc::new(Mutex::new(None));
    let resume_engine = Engine::with_workspace(
        Box::new(RecordingModelClient::new(captured_messages.clone())),
        ToolRegistry::new(),
        ContextManager::with_max_history("system".to_string(), 2),
        EngineConfig {
            max_steps: 1,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );
    let resume_req = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "continue".to_string(),
        resume_state: None,
    };

    let _ = collect_events_with_request(&resume_engine, resume_req).await;
    let messages = captured_messages.lock().unwrap().clone().unwrap();
    assert!(messages.iter().any(|message| {
        message
            .content
            .contains("Session summary: # Session Summary")
            && message.content.contains("- Tools used: fs_write")
    }));
}

#[tokio::test]
async fn engine_includes_relevant_durable_memory_in_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let memory_dir = workspace.root.join(".rove").join("memory");
    let topics_dir = memory_dir.join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    std::fs::write(
        memory_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Project Facts](topics/project-facts.md) - project memory\n- [User Preferences](topics/user-preferences.md) - user memory\n",
    )
    .unwrap();
    std::fs::write(
        topics_dir.join("project-facts.md"),
        "---\ntitle: Project Facts\ntype: project\n---\n\nUse SQLite for the state index.\n",
    )
    .unwrap();
    std::fs::write(
        topics_dir.join("user-preferences.md"),
        "---\ntitle: User Preferences\ntype: user\n---\n\nPrefers terse responses.\n",
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

    let events = collect_events(&engine, "apply project facts").await;
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
    assert!(
        messages[1]
            .content
            .contains("Use SQLite for the state index.")
    );
    assert!(!messages[1].content.contains("User Preferences"));
    assert_eq!(messages.last().unwrap().content, "apply project facts");
}

#[tokio::test]
async fn engine_honors_configured_durable_memory_dir_for_prompt_recall() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let durable_dir = workspace.root.join("configured-durable-memory");
    let topics_dir = durable_dir.join("topics");
    std::fs::create_dir_all(&topics_dir).unwrap();
    std::fs::write(
        durable_dir.join("MEMORY.md"),
        "# rove Memory\n\n- [Project Facts](topics/project-facts.md) - project memory\n",
    )
    .unwrap();
    std::fs::write(
        topics_dir.join("project-facts.md"),
        "---\ntitle: Project Facts\ntype: project\n---\n\nUse configured durable memory.\n",
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
        workspace.clone(),
        ApprovalPolicy::Auto,
    )
    .with_memory_paths(rove::memory::paths::MemoryPaths {
        session_dir: workspace.state_dir.join("memory").join("sessions"),
        durable_dir: durable_dir.clone(),
        recall_limit: 1,
    });

    let events = collect_events(&engine, "apply project facts").await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::RunCompleted { .. }))
    );

    let messages = captured_messages.lock().unwrap().clone().unwrap();
    assert!(
        messages[1]
            .content
            .contains("Use configured durable memory.")
    );
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
async fn engine_writes_completed_plan_steps_to_session_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let session_id = SessionId::new();
    let engine = Engine::with_workspace(
        Box::new(FakeModelClient::new(vec![
            r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"},{"id":"2","title":"write summary"}]}"#.to_string(),
            "step 1 done".to_string(),
            "step 2 done".to_string(),
        ])),
        ToolRegistry::new(),
        ContextManager::new("system".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: true,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    );
    let req = RunRequest {
        session_id,
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "fix the docs".to_string(),
        resume_state: None,
    };

    let events = collect_events_with_request(&engine, req).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanStepCompleted { .. }))
            .count(),
        2
    );

    let session_memory_path = workspace
        .state_dir
        .join("memory")
        .join("sessions")
        .join(format!("{session_id}.md"));
    let summary = std::fs::read_to_string(session_memory_path).unwrap();
    assert!(summary.contains("- Goal: fix the docs"));
    assert!(summary.contains("- Completed plan steps: 1 inspect docs; 2 write summary"));
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
async fn planner_uses_engine_configured_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    std::fs::create_dir_all(workspace.root.join(".rove")).unwrap();
    std::fs::create_dir_all(workspace.root.join("prompts")).unwrap();
    std::fs::write(
        workspace.root.join("prompts").join("custom-planner.md"),
        "CUSTOM PLANNER PROMPT",
    )
    .unwrap();
    std::fs::write(
        workspace.root.join(".rove").join("config.toml"),
        "[runtime]\nplanner_prompt_path = \"prompts/custom-planner.md\"\n",
    )
    .unwrap();
    let config = AppConfig::load(&workspace.root, AppConfigOverrides::default()).unwrap();

    let captured = Arc::new(Mutex::new(Vec::new()));
    let model = Box::new(CapturingFakeModelClient::new(
        vec![
            r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#.to_string(),
            "step 1 done".to_string(),
        ],
        captured.clone(),
    ));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: true,
        },
        workspace,
        ApprovalPolicy::Auto,
    )
    .with_planner_prompt(config.load_planner_prompt());

    let events = collect_events(&engine, "fix the docs").await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepCompleted { step, .. } if step.id == "1"
        )
    }));
    let captured = captured.lock().unwrap();
    assert_eq!(captured[0][0].content, "CUSTOM PLANNER PROMPT");
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
            checkpoint: None,
            plan: Some(plan),
            runtime_identity: None,
            step_ledger: Default::default(),
        }),
    };
    let engine = build_planner_test_engine(vec!["step 2 done".to_string()]);

    let events = collect_events_with_request(&engine, req).await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanCreated {
                plan_revision: Some(revision),
                ..
            } if revision.safe_reason_codes == ["legacy_plan_migrated"]
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { step, .. } if step.id == "2"
        )
    }));
}

#[tokio::test]
async fn planner_resume_checkpoint_does_not_repeat_completed_steps() {
    let checkpoint_plan = TaskPlan {
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
            history: vec![Message::user("previous plan work")],
            summary: None,
            checkpoint: Some(PromptCheckpoint {
                summary: None,
                preserved_tail: vec![Message::user("previous plan work")],
                plan: Some(checkpoint_plan),
                session_memory_pointer: None,
                durable_memory_pointer: None,
                last_step: 1,
                last_event_seq: Some(7),
                token_estimate: 12,
                compacted_history_messages: 0,
                compaction: Default::default(),
                runtime_identity: None,
                step_ledger: Default::default(),
            }),
            plan: None,
            runtime_identity: None,
            step_ledger: Default::default(),
        }),
    };
    let engine = build_planner_test_engine(vec!["step 2 done".to_string()]);

    let events = collect_events_with_request(&engine, req).await;

    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                StreamEvent::PlanCreated {
                    plan_revision: Some(revision),
                    ..
                } if revision.safe_reason_codes == ["legacy_plan_migrated"]
            )
        }),
        "resume should wrap the checkpoint plan without asking the model to draft a new one"
    );
    assert!(!events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { step, .. } if step.id == "1"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { step, .. } if step.id == "2"
        )
    }));
}

#[tokio::test]
async fn planner_resume_closes_unknown_in_flight_attempt_without_replay() {
    let identity = PlanIdentity::fresh();
    let attempt = StepAttempt {
        plan_id: identity.plan_id.clone(),
        plan_revision_id: identity.plan_revision_id.clone(),
        step_id: "1".to_string(),
        attempt: 1,
        started_at: "2026-07-20T00:00:00Z".to_string(),
    };
    let plan = TaskPlan {
        goal: "do not replay".to_string(),
        steps: vec![PlanStep {
            id: "1".to_string(),
            title: "unknown external mutation".to_string(),
            done: false,
        }],
        current_step: 0,
    };
    let mut step_ledger = StepLedgerState::default();
    step_ledger.set_plan_identity(&identity);
    step_ledger.active_step_attempt = Some(attempt.clone());
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "do not replay".to_string(),
        resume_state: Some(TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "do not replay".to_string(),
            step: 1,
            history: Vec::new(),
            summary: None,
            checkpoint: None,
            plan: Some(plan),
            runtime_identity: None,
            step_ledger,
        }),
    };
    let engine = build_planner_test_engine(vec!["must not be called".to_string()]);

    let events = collect_events_with_request(&engine, req).await;

    assert!(!events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { .. }
                | StreamEvent::ToolCallStarted { .. }
                | StreamEvent::LlmMessage { .. }
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::StepResult { record }
                if record.status == StepRecordStatus::Interrupted
                    && record.plan_id == attempt.plan_id
                    && record.plan_revision_id == attempt.plan_revision_id
                    && record.step_id == attempt.step_id
                    && record.attempt == attempt.attempt
                    && record.error_code.as_deref() == Some("interrupted")
        )
    }));
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Error,
            ..
        })
    ));
}

#[tokio::test]
async fn planner_resume_applies_terminal_success_without_replaying_the_step() {
    let identity = PlanIdentity::fresh();
    let record = sample_step_record(
        &identity.plan_id,
        &identity.plan_revision_id,
        "1",
        1,
        StepRecordStatus::Succeeded,
    );
    let plan = TaskPlan {
        goal: "resume terminal result".to_string(),
        steps: vec![PlanStep {
            id: "1".to_string(),
            title: "already completed".to_string(),
            done: true,
        }],
        current_step: 1,
    };
    let mut step_ledger = StepLedgerState::default();
    step_ledger.set_plan_identity(&identity);
    step_ledger.step_records.push(record.clone());
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "resume terminal result".to_string(),
        resume_state: Some(TaskState {
            schema_version: 1,
            session_id: SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            goal: "resume terminal result".to_string(),
            step: 1,
            history: Vec::new(),
            summary: None,
            checkpoint: None,
            plan: Some(plan),
            runtime_identity: None,
            step_ledger,
        }),
    };
    let engine = build_planner_test_engine(vec!["must not be called".to_string()]);

    let events = collect_events_with_request(&engine, req).await;

    assert!(!events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepStarted { .. }
                | StreamEvent::StepResult { .. }
                | StreamEvent::LlmMessage { .. }
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepCompleted { step, .. } if step.id == "1"
        )
    }));
    assert_eq!(
        events
            .iter()
            .filter_map(|event| match event {
                StreamEvent::PlanDecision { record } => Some(record.as_ref()),
                _ => None,
            })
            .filter(|decision| decision.trigger_step_record_id == record.record_id)
            .count(),
        1,
        "resume must fill the missing decision exactly once"
    );
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            ..
        })
    ));
}

#[tokio::test]
async fn planned_step_returns_tool_result_to_model_before_completion() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let model = Box::new(CapturingFakeModelClient::new(
        vec![
            r#"{"goal":"echo ping","steps":[{"id":"1","title":"echo ping"}]}"#.to_string(),
            r#"{"tool":"echo","args":{"message":"ping"}}"#.to_string(),
            "The echo returned: ping".to_string(),
        ],
        captured.clone(),
    ));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::new(
        model,
        registry,
        ContextManager::with_max_history("You are a test agent.".to_string(), 0),
        EngineConfig {
            max_steps: 5,
            plan_enabled: true,
        },
    );

    let events = collect_events(&engine, "echo ping").await;

    let prompts = captured.lock().unwrap();
    assert_eq!(prompts.len(), 3, "planner plus two step model turns");
    assert!(
        prompts[2]
            .iter()
            .any(|message| { message.role == Role::Tool && message.content == "ping" }),
        "the second step turn must receive the tool result even with a zero history window"
    );

    let tool_completed = events
        .iter()
        .position(|event| matches!(event, StreamEvent::ToolCallCompleted { .. }))
        .unwrap();
    let post_tool_prompt = events
        .iter()
        .enumerate()
        .skip(tool_completed + 1)
        .find_map(|(index, event)| {
            matches!(event, StreamEvent::PromptBuilt { .. }).then_some(index)
        })
        .expect("step runner should build another prompt after the tool result");
    let step_completed = events
        .iter()
        .position(|event| matches!(event, StreamEvent::PlanStepCompleted { .. }))
        .unwrap();
    assert!(tool_completed < post_tool_prompt);
    assert!(post_tool_prompt < step_completed);
}

#[tokio::test]
async fn planned_step_emits_complete_step_record_before_compatibility_completion() {
    let engine = build_planner_test_engine(vec![
        r#"{"goal":"echo ping","steps":[{"id":"1","title":"echo ping"}]}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"ping"}}"#.to_string(),
        "The echo returned: ping".to_string(),
    ]);

    let events = collect_events(&engine, "echo ping").await;
    let started = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::PlanStepStarted { attempt, .. } => Some(attempt),
            _ => None,
        })
        .expect("planned step should publish its stable attempt identity");
    let (record_index, record) = events
        .iter()
        .enumerate()
        .find_map(|(index, event)| match event {
            StreamEvent::StepResult { record } => Some((index, record)),
            _ => None,
        })
        .expect("planned step should emit a terminal step_result");
    let completed_index = events
        .iter()
        .position(|event| matches!(event, StreamEvent::PlanStepCompleted { .. }))
        .unwrap();
    let (decision_index, decision) = events
        .iter()
        .enumerate()
        .find_map(|(index, event)| match event {
            StreamEvent::PlanDecision { record } => Some((index, record.as_ref())),
            _ => None,
        })
        .expect("terminal step should emit a plan decision");

    assert!(record_index < decision_index);
    assert!(decision_index < completed_index);
    assert_eq!(decision.trigger_step_record_id, record.record_id);
    assert_eq!(decision.decision.kind, PlanDecisionKind::Finish);
    assert_eq!(
        decision.decision.finish_reason,
        Some(PlanFinishReason::Completed)
    );
    assert!(started.is_complete());
    assert_eq!(record.plan_id, started.plan_id);
    assert_eq!(record.plan_revision_id, started.plan_revision_id);
    assert_eq!(record.step_id, "1");
    assert_eq!(record.attempt, 1);
    assert_eq!(record.status, StepRecordStatus::Succeeded);
    assert_eq!(record.summary, "The echo returned: ping");
    assert_eq!(
        record.completion_basis,
        StepCompletionBasis::ModelConclusion
    );
    assert_eq!(record.model_turns_used, 2);
    assert_eq!(record.tool_calls_used, 1);
    assert_eq!(record.tool_call_ids.len(), 1);
    assert_eq!(record.evidence_refs.len(), 1);
    assert_eq!(record.token_usage.total_tokens, 30);
    record.validate().unwrap();
}

#[tokio::test]
async fn replanning_retains_failed_record_and_advances_revision_identity() {
    let engine = build_replanning_test_engine();

    let events = collect_events(&engine, "fix the docs").await;
    let initial_revision = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::PlanCreated {
                plan_revision: Some(revision),
                ..
            } => Some(revision.as_ref()),
            _ => None,
        })
        .expect("initial plan should carry revision zero");
    let revised = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::PlanRevised { revision, .. } => Some(revision.as_ref()),
            _ => None,
        })
        .expect("recoverable failure should create a child revision");
    let records: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::StepResult { record } => Some(record),
            _ => None,
        })
        .collect();
    let replace_decision = events
        .iter()
        .find_map(|event| match event {
            StreamEvent::PlanDecision { record }
                if record.decision.kind == PlanDecisionKind::ReplaceRemaining =>
            {
                Some(record.as_ref())
            }
            _ => None,
        })
        .expect("recoverable failure should emit a replace decision");

    assert_eq!(initial_revision.plan_id, revised.plan_id);
    assert_ne!(initial_revision.revision_id, revised.revision_id);
    assert_eq!(initial_revision.revision, 0);
    assert_eq!(revised.revision, 1);
    assert_eq!(
        revised.parent_revision_id.as_deref(),
        Some(initial_revision.revision_id.as_str())
    );
    assert_eq!(
        revised.trigger_step_record_id.as_deref(),
        Some(records[0].record_id.as_str())
    );
    assert_eq!(revised.decision_id, replace_decision.decision.decision_id);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanCreated { .. }))
            .count(),
        1,
        "replanning must not masquerade as a second initial plan"
    );
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].status, StepRecordStatus::Failed);
    assert_eq!(
        records[0].error_code.as_deref(),
        Some("step_recoverable_failure")
    );
    assert_eq!(records[1].status, StepRecordStatus::Succeeded);
    assert_eq!(records[0].plan_id, records[1].plan_id);
    assert_ne!(records[0].plan_revision_id, records[1].plan_revision_id);

    let failed_record_index = events
        .iter()
        .position(|event| {
            matches!(event, StreamEvent::StepResult { record } if record.record_id == records[0].record_id)
        })
        .unwrap();
    let replace_decision_index = events
        .iter()
        .position(|event| {
            matches!(event, StreamEvent::PlanDecision { record } if record.decision.kind == PlanDecisionKind::ReplaceRemaining)
        })
        .unwrap();
    let compatibility_failure_index = events
        .iter()
        .position(|event| matches!(event, StreamEvent::PlanStepFailed { .. }))
        .unwrap();
    let revised_index = events
        .iter()
        .position(|event| matches!(event, StreamEvent::PlanRevised { .. }))
        .unwrap();
    assert!(failed_record_index < replace_decision_index);
    assert!(replace_decision_index < compatibility_failure_index);
    assert!(compatibility_failure_index < revised_index);
}

#[tokio::test]
async fn planned_step_model_turn_budget_exhaustion_is_explicit() {
    let repeated_tool_call = r#"{"tool":"echo","args":{"message":"keep going"}}"#.to_string();
    let engine = build_planner_test_engine(vec![
        r#"{"goal":"bounded step","steps":[{"id":"1","title":"keep calling"}]}"#.to_string(),
        repeated_tool_call.clone(),
        repeated_tool_call.clone(),
        repeated_tool_call.clone(),
        repeated_tool_call,
    ]);

    let events = collect_events(&engine, "run a bounded step").await;

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::ToolCallStarted { .. }))
            .count(),
        4
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, StreamEvent::PlanStepCompleted { .. }))
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepFailed { reason, .. }
                if reason == "step model-turn budget exhausted (max_model_turns_per_step=4)"
        )
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::StepResult { record }
                if record.status == StepRecordStatus::BudgetExhausted
                    && record.error_code.as_deref()
                        == Some("step_model_turn_budget_exhausted")
        )
    }));
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::StepLimit,
            output: Some(output),
        }) if output.contains("max_model_turns_per_step=4")
    ));
}

#[tokio::test]
async fn planned_permission_denial_emits_blocked_step_record() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"goal":"run danger","steps":[{"id":"1","title":"run danger"}]}"#.to_string(),
        r#"{"tool":"danger","args":{}}"#.to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(FakeDestructiveTool));
    let engine = Engine::with_workspace_and_approval_decision(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: true,
        },
        workspace,
        ApprovalPolicy::Ask,
        ApprovalDecision::Reject,
    );

    let events = collect_events(&engine, "run danger").await;
    let result_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                StreamEvent::StepResult { record }
                    if record.status == StepRecordStatus::Blocked
                        && record.error_code.as_deref() == Some("permission_denied")
            )
        })
        .expect("permission denial should produce a blocked terminal record");
    let failed_index = events
        .iter()
        .position(|event| matches!(event, StreamEvent::PlanStepFailed { .. }))
        .unwrap();

    assert!(result_index < failed_index);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanCreated { .. }))
            .count(),
        1,
        "a denied mutation must not be retried through compatibility replanning"
    );
}

#[tokio::test]
async fn planned_context_limit_emits_budget_exhausted_step_record() {
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"goal":"bounded context","steps":[{"id":"1","title":"inspect"}]}"#.to_string(),
    ]));
    let engine = Engine::new(
        model,
        ToolRegistry::new(),
        ContextManager::with_token_budget(
            "system".to_string(),
            ContextBudget {
                soft_limit_tokens: 1,
                hard_limit_tokens: 1,
                reserved_tokens: 0,
            },
        ),
        EngineConfig {
            max_steps: 3,
            plan_enabled: true,
        },
    );

    let events = collect_events(&engine, "bounded context").await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::StepResult { record }
                if record.status == StepRecordStatus::BudgetExhausted
                    && record.error_code.as_deref() == Some("context_token_limit")
                    && record.model_turns_used == 0
        )
    }));
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::TokenLimit,
            ..
        })
    ));
}

#[tokio::test]
async fn planner_repairs_recoverable_tool_error_within_the_same_step() {
    let engine = build_planner_test_engine(vec![
        r#"{"goal":"fix docs","steps":[{"id":"1","title":"inspect docs"}]}"#.to_string(),
        r#"{"tool":"echo","args":"wrong"}"#.to_string(),
        "step 1 done".to_string(),
    ]);

    let events = collect_events(&engine, "fix the docs").await;

    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::ToolCallFailed { error, .. }
                if error.to_string().contains("tool arguments must be")
        )
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanCreated { .. }))
            .count(),
        1,
        "a recoverable tool error should not replace the plan before the model sees it"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, StreamEvent::PlanStepFailed { .. }))
    );
    assert!(events.iter().any(|event| {
        matches!(
            event,
            StreamEvent::PlanStepCompleted { step, .. } if step.id == "1"
        )
    }));
}

#[tokio::test]
async fn planner_replans_after_step_failure() {
    let engine = build_replanning_test_engine();

    let events = collect_events(&engine, "fix the docs").await;

    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanCreated { .. }))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, StreamEvent::PlanRevised { .. }))
            .count(),
        1
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
    let engine = build_replanning_test_engine();

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
    let persisted_records = task_state.step_ledger.step_records.clone();
    let persisted_decisions = task_state.step_ledger.plan_lifecycle.decisions.clone();
    let persisted_revisions = task_state.step_ledger.plan_lifecycle.revisions.clone();
    assert_eq!(persisted_records.len(), 2);
    assert_eq!(persisted_decisions.len(), 2);
    assert_eq!(persisted_revisions.len(), 2);
    assert_eq!(persisted_records[0].status, StepRecordStatus::Failed);
    assert_eq!(persisted_records[1].status, StepRecordStatus::Succeeded);
    assert!(task_state.step_ledger.active_step_attempt.is_none());
    assert_eq!(
        task_state
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.step_ledger.step_record_count),
        Some(2)
    );
    assert_eq!(
        task_state
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.step_ledger.plan_lifecycle.decision_count),
        Some(2)
    );
    assert_eq!(
        task_state
            .checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.step_ledger.plan_lifecycle.revision_count),
        Some(2)
    );
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

    let report = state_store.load_report(run_id).await.unwrap();
    assert_eq!(report.step_records, persisted_records);
    assert_eq!(report.plan_decisions, persisted_decisions);
    assert_eq!(report.plan_revisions, persisted_revisions);
    let trace = std::fs::read_to_string(state_store.run_store.run_dir(&run_id).join("trace.jsonl"))
        .unwrap();
    let traced_records: Vec<_> = trace
        .lines()
        .map(|line| serde_json::from_str::<StreamEvent>(line).unwrap())
        .filter_map(|event| match event {
            StreamEvent::StepResult { record } => Some(*record),
            _ => None,
        })
        .collect();
    assert_eq!(traced_records, persisted_records);
    let traced_decisions: Vec<_> = trace
        .lines()
        .map(|line| serde_json::from_str::<StreamEvent>(line).unwrap())
        .filter_map(|event| match event {
            StreamEvent::PlanDecision { record } => Some(*record),
            _ => None,
        })
        .collect();
    assert_eq!(traced_decisions, persisted_decisions);
    let traced_revisions: Vec<_> = trace
        .lines()
        .map(|line| serde_json::from_str::<StreamEvent>(line).unwrap())
        .filter_map(|event| match event {
            StreamEvent::PlanCreated {
                plan_revision: Some(revision),
                ..
            }
            | StreamEvent::PlanRevised { revision, .. } => Some(*revision),
            _ => None,
        })
        .collect();
    assert_eq!(traced_revisions, persisted_revisions);
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
            history: vec![Message::user("previous step failed and was re-planned")],
            summary: None,
            checkpoint: None,
            plan: Some(plan),
            runtime_identity: None,
            step_ledger: Default::default(),
        }),
    };
    let engine = build_planner_test_engine(vec!["resumed replanned step done".to_string()]);

    let events = collect_events_with_request(&engine, req).await;

    assert!(
        events.iter().any(|event| {
            matches!(
                event,
                StreamEvent::PlanCreated {
                    plan_revision: Some(revision),
                    ..
                } if revision.safe_reason_codes == ["legacy_plan_migrated"]
            )
        }),
        "resume should wrap the persisted re-planned plan without asking the model to draft a new one"
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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
    let ctx = tool_context(&workspace, ApprovalPolicy::Never);

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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
    let ctx = tool_context(&workspace, ApprovalPolicy::Auto);

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
async fn planned_cancellation_closes_the_in_flight_step_record() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"goal":"wait","steps":[{"id":"1","title":"wait for tool"}]}"#.to_string(),
        r#"{"tool":"wait_forever","args":{}}"#.to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(NeverCompletesTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: true,
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
    let mut events = Vec::new();

    while let Some(event) = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("planned run should reach the tool call")
    {
        let should_cancel = matches!(
            &event,
            StreamEvent::ToolCallStarted { name, .. } if name == "wait_forever"
        );
        events.push(event);
        if should_cancel {
            cancel.cancel();
            break;
        }
    }
    while let Some(event) = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("cancelled planned run should finish promptly")
    {
        events.push(event);
    }

    let result_index = events
        .iter()
        .position(|event| {
            matches!(
                event,
                StreamEvent::StepResult { record }
                    if record.status == StepRecordStatus::Cancelled
                        && record.error_code.as_deref() == Some("cancelled")
            )
        })
        .expect("cancelled attempt should have a terminal record");
    let completed_index = events
        .iter()
        .position(|event| matches!(event, StreamEvent::RunCompleted { .. }))
        .unwrap();
    assert!(result_index < completed_index);
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Cancelled,
            ..
        })
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
fn context_manager_fits_history_by_token_budget() {
    let context = ContextManager::with_token_budget(
        "s".to_string(),
        ContextBudget {
            soft_limit_tokens: 40,
            hard_limit_tokens: 60,
            reserved_tokens: 10,
        },
    );
    let memory = vec![user_message("m")];
    let history = vec![
        user_message(&"old ".repeat(160)),
        user_message("recent one"),
        user_message("recent two"),
    ];

    let built = context.build_with_checkpoint("c", &memory, None, &history);
    let contents: Vec<_> = built
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();

    assert!(built.dropped_history_messages > 0);
    assert!(!built.over_hard_limit);
    assert!(contents.contains(&"s"));
    assert!(contents.contains(&"m"));
    assert!(contents.contains(&"recent one"));
    assert!(contents.contains(&"recent two"));
    assert!(contents.contains(&"c"));
    assert!(
        !contents
            .iter()
            .any(|content| content.starts_with("old old old"))
    );
}

#[test]
fn compaction_policy_requests_auto_compaction_after_soft_limit() {
    let budget = ContextBudget {
        soft_limit_tokens: 20,
        hard_limit_tokens: 80,
        reserved_tokens: 5,
    };
    let build = rove::core::context::ContextBuild {
        messages: vec![user_message("large context")],
        token_estimate: 24,
        included_history_messages: 3,
        dropped_history_messages: 2,
        over_hard_limit: false,
        auto_compaction_needed: true,
        metadata: Default::default(),
    };

    let decision = rove::core::context::CompactionPolicy::default().decide(&build, budget);

    assert_eq!(
        decision.mode,
        rove::core::context::CompactionMode::Automatic
    );
    assert!(!decision.circuit_open);
}

#[test]
fn compaction_policy_opens_circuit_after_repeated_failures() {
    let budget = ContextBudget {
        soft_limit_tokens: 20,
        hard_limit_tokens: 80,
        reserved_tokens: 5,
    };
    let build = rove::core::context::ContextBuild {
        messages: vec![user_message("large context")],
        token_estimate: 24,
        included_history_messages: 3,
        dropped_history_messages: 2,
        over_hard_limit: false,
        auto_compaction_needed: true,
        metadata: Default::default(),
    };

    let decision = rove::core::context::CompactionPolicy {
        consecutive_failures: 3,
        failure_threshold: 3,
    }
    .decide(&build, budget);

    assert_eq!(decision.mode, rove::core::context::CompactionMode::Disabled);
    assert!(decision.circuit_open);
}

#[tokio::test]
async fn oneshot_persists_prompt_checkpoint() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"echo","args":{"message":"one"}}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"two"}}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"three"}}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"four"}}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"five"}}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"six"}}"#.to_string(),
        "done".to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 8,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    );

    run_oneshot(
        &engine,
        "build checkpoint".to_string(),
        run,
        None,
        &state_store,
    )
    .await;

    let run_dir = workspace.state_dir.join("runs").join(run_id.to_string());
    let task_state: TaskState =
        serde_json::from_slice(&std::fs::read(run_dir.join("task_state.json")).unwrap()).unwrap();
    let checkpoint = task_state
        .checkpoint
        .expect("task_state should include a prompt checkpoint");
    assert_eq!(checkpoint.last_step, task_state.step);
    assert!(!checkpoint.preserved_tail.is_empty());
    assert!(checkpoint.preserved_tail.len() <= 12);
    assert!(checkpoint.compacted_history_messages > 0);
    assert_eq!(checkpoint.summary.as_deref(), Some("done"));
    assert!(checkpoint.session_memory_pointer.is_some());
    assert!(checkpoint.durable_memory_pointer.is_some());
    let last_event_seq = checkpoint
        .last_event_seq
        .expect("checkpoint should record last event sequence");
    assert_eq!(
        last_event_seq,
        state_store.index.last_event_seq(run_id).unwrap()
    );
    assert!(last_event_seq > 1);
    assert!(checkpoint.token_estimate > 0);
    assert_eq!(
        checkpoint.compaction.mode,
        rove::core::types::PromptCompactionMode::Deterministic
    );
    assert!(!checkpoint.compaction.circuit_open);
}

#[tokio::test]
async fn model_compaction_stores_generated_summary_in_checkpoint() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"echo","args":{"message":"one"}}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"two"}}"#.to_string(),
        "MODEL GENERATED COMPACTION SUMMARY".to_string(),
        "done".to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::with_max_history("You are a test agent.".to_string(), 2),
        EngineConfig {
            max_steps: 4,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    )
    .with_model_compaction(true, 3);

    run_oneshot(
        &engine,
        "build model checkpoint".to_string(),
        run,
        None,
        &state_store,
    )
    .await;

    let run_dir = workspace.state_dir.join("runs").join(run_id.to_string());
    let task_state: TaskState =
        serde_json::from_slice(&std::fs::read(run_dir.join("task_state.json")).unwrap()).unwrap();
    let checkpoint = task_state
        .checkpoint
        .expect("task_state should include a prompt checkpoint");
    // v2 structured compaction: the fake model's free text ("MODEL GENERATED
    // COMPACTION SUMMARY") has no section headings, so StructuredSummary::parse
    // treats it as unguided prose and stores it as the goal. The stored
    // checkpoint summary is the structured render of that, not the raw text.
    assert_eq!(
        checkpoint.summary.as_deref(),
        Some("Compact summary:\nGoal: MODEL GENERATED COMPACTION SUMMARY")
    );
    assert_eq!(
        checkpoint.compaction.mode,
        rove::core::types::PromptCompactionMode::ModelGenerated
    );
    assert_eq!(checkpoint.compaction.model.as_deref(), Some("fake-model"));
    assert_eq!(checkpoint.compaction.source_message_count, 2);
    assert!(!checkpoint.compaction.degraded);
    assert!(!checkpoint.compaction.circuit_open);
}

#[tokio::test]
async fn compaction_flushes_tool_notes_to_session_memory_before_summarizing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let session_id = SessionId::new();
    let run_id = RunId::new();
    let run = state_store
        .start_run(session_id, JobId::new(), run_id)
        .unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"echo","args":{"message":"created file src/memory/session.rs"}}"#.to_string(),
        "MODEL GENERATED COMPACTION SUMMARY".to_string(),
        "done".to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::with_max_history("You are a test agent.".to_string(), 0),
        EngineConfig {
            max_steps: 3,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    )
    .with_model_compaction(true, 3);

    run_oneshot(
        &engine,
        "build model checkpoint".to_string(),
        run,
        None,
        &state_store,
    )
    .await;

    let session_summary_path = workspace
        .state_dir
        .join("memory")
        .join("sessions")
        .join(format!("{session_id}.md"));
    let session_summary = std::fs::read_to_string(session_summary_path)
        .expect("pre-compaction flush should write the session summary file");
    assert!(
        session_summary.contains("## Flush at "),
        "flush block should include a timestamp, got: {session_summary}"
    );
    assert!(
        session_summary.contains("tool result: created file src/memory/session.rs"),
        "flush should preserve the soon-to-be-compacted tool result, got: {session_summary}"
    );
}

#[tokio::test]
async fn planned_compaction_flushes_tool_notes_to_session_memory_before_summarizing() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let session_id = SessionId::new();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"goal":"planned flush","steps":[{"id":"1","title":"make durable note"},{"id":"2","title":"finish"}]}"#.to_string(),
        r#"{"tool":"echo","args":{"message":"modified file src/core/plan_loop.rs"}}"#.to_string(),
        "MODEL GENERATED COMPACTION SUMMARY".to_string(),
        "done".to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::with_max_history("You are a test agent.".to_string(), 0),
        EngineConfig {
            max_steps: 4,
            plan_enabled: true,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    )
    .with_model_compaction(true, 3);

    let events = collect_events_with_request(
        &engine,
        RunRequest {
            session_id,
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message: "run planned flush".to_string(),
            resume_state: None,
        },
    )
    .await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::MemoryFlushed { .. })),
        "planned loop should emit MemoryFlushed before compaction"
    );
    let session_summary_path = workspace
        .state_dir
        .join("memory")
        .join("sessions")
        .join(format!("{session_id}.md"));
    let session_summary = std::fs::read_to_string(session_summary_path)
        .expect("planned pre-compaction flush should write the session summary file");
    assert!(
        session_summary.contains("## Flush at "),
        "planned flush block should include a timestamp, got: {session_summary}"
    );
    assert!(
        session_summary.contains("tool result: modified file src/core/plan_loop.rs"),
        "planned flush should preserve the soon-to-be-compacted tool result, got: {session_summary}"
    );
}

#[tokio::test]
async fn failing_model_compaction_falls_back_to_deterministic_summary_with_circuit_metadata() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let state_store = StateStore::new(&workspace.state_dir);
    let run_id = RunId::new();
    let run = state_store
        .start_run(SessionId::new(), JobId::new(), run_id)
        .unwrap();
    let model = Box::new(FailingAfterFirstCallModelClient::new(
        r#"{"tool":"echo","args":{"message":"one"}}"#,
    ));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::with_max_history("You are a test agent.".to_string(), 1),
        EngineConfig {
            max_steps: 2,
            plan_enabled: false,
        },
        workspace.clone(),
        ApprovalPolicy::Auto,
    )
    .with_model_compaction(true, 1);

    run_oneshot(
        &engine,
        "build fallback checkpoint".to_string(),
        run,
        None,
        &state_store,
    )
    .await;

    let run_dir = workspace.state_dir.join("runs").join(run_id.to_string());
    let task_state: TaskState =
        serde_json::from_slice(&std::fs::read(run_dir.join("task_state.json")).unwrap()).unwrap();
    let checkpoint = task_state
        .checkpoint
        .expect("task_state should include a prompt checkpoint");
    assert_eq!(
        checkpoint.compaction.mode,
        rove::core::types::PromptCompactionMode::Degraded
    );
    // v2 structured fallback: deterministic_structured_summary renders as a
    // non-empty "Compact summary" (Goal/Key results/etc.) rather than the old
    // flat "N earlier message(s) compacted" line. The exact sections depend on
    // what survived into the compacted window, so we only assert the envelope
    // and non-emptiness here; the metadata assertions below pin the behavior.
    let summary = checkpoint
        .summary
        .expect("degraded summary must be present");
    assert!(
        summary.starts_with("Compact summary:\n"),
        "degraded summary should be a structured render, got: {summary}"
    );
    assert!(
        !summary.trim().is_empty(),
        "degraded summary must not be empty: got: {summary}"
    );
    assert!(checkpoint.compaction.degraded);
    assert!(checkpoint.compaction.circuit_open);
    assert_eq!(checkpoint.compaction.consecutive_failures, 1);
    assert!(
        checkpoint
            .compaction
            .last_error
            .as_deref()
            .unwrap()
            .contains("compaction model failed")
    );
}

#[tokio::test]
async fn resumed_run_prefers_prompt_checkpoint_tail_and_summary() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let captured_messages = Arc::new(Mutex::new(None));
    let model = Box::new(RecordingModelClient::new(captured_messages.clone()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::with_max_history("system".to_string(), 20),
        EngineConfig {
            max_steps: 1,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );
    let checkpoint_plan = TaskPlan {
        goal: "checkpoint goal".to_string(),
        steps: vec![PlanStep {
            id: "1".to_string(),
            title: "checkpoint step".to_string(),
            done: false,
        }],
        current_step: 0,
    };
    let resume_state = TaskState {
        schema_version: 1,
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        goal: "resume".to_string(),
        step: 2,
        history: vec![user_message("full history should not appear")],
        summary: None,
        checkpoint: Some(PromptCheckpoint {
            summary: Some("checkpoint summary".to_string()),
            preserved_tail: vec![user_message("checkpoint tail")],
            plan: Some(checkpoint_plan),
            session_memory_pointer: Some(".rove/memory/sessions/test.md".to_string()),
            durable_memory_pointer: Some(".rove/memory/MEMORY.md".to_string()),
            last_step: 0,
            last_event_seq: Some(42),
            token_estimate: 12,
            compacted_history_messages: 1,
            compaction: Default::default(),
            runtime_identity: None,
            step_ledger: Default::default(),
        }),
        plan: None,
        runtime_identity: None,
        step_ledger: Default::default(),
    };
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "current task".to_string(),
        resume_state: Some(resume_state),
    };

    let events = collect_events_with_request(&engine, req).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::RunCompleted { .. }))
    );

    let messages = captured_messages.lock().unwrap().clone().unwrap();
    let contents: Vec<_> = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert!(contents.contains(&"Compact summary: checkpoint summary"));
    assert!(contents.contains(&"checkpoint tail"));
    assert!(contents.contains(&"current task"));
    assert!(!contents.contains(&"full history should not appear"));
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

    let indexed_report = state_store
        .index
        .report_record(run_id)
        .unwrap()
        .expect("report should be indexed");
    assert_eq!(indexed_report.path, run_dir.join("report.json"));
    assert_eq!(indexed_report.status, "success");
    assert_eq!(indexed_report.termination_reason, "final");
    let indexed_run = state_store
        .index
        .run_record(run_id)
        .unwrap()
        .expect("run should be indexed");
    assert_eq!(indexed_run.status, "done");
    assert_eq!(
        indexed_run.report_path.as_deref(),
        Some(indexed_report.path.as_path())
    );
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
async fn engine_emits_safe_model_status_without_raw_thinking_text() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        Box::new(ThinkingStatusModelClient),
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig::default(),
        workspace,
        ApprovalPolicy::Auto,
    );

    let events = collect_events(&engine, "think safely").await;

    assert!(
        events.iter().any(|event| matches!(
            event,
            StreamEvent::ModelStatus {
                status,
                message,
            } if status == "thinking" && message == "Model is thinking"
        )),
        "expected a safe thinking status event"
    );
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(
        !serialized.contains("PRIVATE_CHAIN_OF_THOUGHT"),
        "raw thinking text must not be exposed"
    );
}

#[tokio::test]
async fn planned_and_unplanned_runs_emit_equivalent_tool_lifecycle_events() {
    let unplanned = build_test_engine(vec![
        r#"{"tool": "echo", "args": {"message": "ping"}}"#.to_string(),
        "The echo returned: ping".to_string(),
    ]);
    let planned = build_planner_test_engine(vec![
        r#"{"goal":"echo ping","steps":[{"id":"1","title":"echo ping"}]}"#.to_string(),
        r#"{"tool": "echo", "args": {"message": "ping"}}"#.to_string(),
        "The echo returned: ping".to_string(),
    ]);

    let unplanned_events = collect_events(&unplanned, "echo ping").await;
    let planned_events = collect_events(&planned, "echo ping").await;

    assert_eq!(
        tool_lifecycle(&planned_events),
        tool_lifecycle(&unplanned_events)
    );
}

#[tokio::test]
async fn engine_runs_parallel_safe_tool_batch_concurrently_with_ordered_writeback() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ProbeTool::new(
        "probe",
        true,
        active.clone(),
        max_active.clone(),
    )));
    let engine = Engine::with_workspace(
        Box::new(FakeModelClient::new(vec![
            r#"{"tools":[{"tool":"probe","args":{"label":"slow","delay_ms":80}},{"tool":"probe","args":{"label":"fast","delay_ms":10}}]}"#
                .to_string(),
            "done".to_string(),
        ])),
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );

    let events = collect_events(&engine, "run probe batch").await;
    let completed: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolCallCompleted { result, .. } => Some(result.output.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(completed, vec!["slow", "fast"]);
    assert_eq!(active.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(max_active.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Final,
            output: Some(output),
        }) if output == "done"
    ));
}

#[tokio::test]
async fn engine_runs_non_parallel_safe_tool_batch_serially() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ProbeTool::new(
        "serial_probe",
        false,
        active.clone(),
        max_active.clone(),
    )));
    let engine = Engine::with_workspace(
        Box::new(FakeModelClient::new(vec![
            r#"{"tools":[{"tool":"serial_probe","args":{"label":"first","delay_ms":20}},{"tool":"serial_probe","args":{"label":"second","delay_ms":20}}]}"#
                .to_string(),
            "done".to_string(),
        ])),
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );

    let events = collect_events(&engine, "run serial probe batch").await;
    let completed: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolCallCompleted { result, .. } => Some(result.output.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(completed, vec!["first", "second"]);
    assert_eq!(active.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(max_active.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Final,
            output: Some(output),
        }) if output == "done"
    ));
}

#[tokio::test]
async fn engine_executes_native_model_tool_use() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        Box::new(NativeToolUseModelClient::new()),
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 3,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );

    let events = collect_events(&engine, "echo through native tool use").await;

    assert!(events.iter().any(|event| matches!(
        event,
        StreamEvent::ToolCallStarted { name, .. } if name == "echo"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        StreamEvent::ToolCallCompleted { result, .. } if result.output == "native hello"
    )));
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Final,
            output: Some(output),
        }) if output == "done with native tool"
    ));
}

#[tokio::test]
async fn engine_routes_request_input_tool_to_input_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool": "request_input", "args": {"prompt": "Which branch should I use?"}}"#
            .to_string(),
        "I will use main.".to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(RequestInputTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    )
    .with_input_provider(Arc::new(RecordingInputProvider {
        answer: "Use main.".to_string(),
        requests: requests.clone(),
    }));

    let events = collect_events(&engine, "ask a clarifying question").await;

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].1.prompt, "Which branch should I use?");
    let input_id = requests[0].0;
    let started_id = events.iter().find_map(|event| match event {
        StreamEvent::ToolCallStarted { call_id, name, .. } if name == "request_input" => {
            Some(*call_id)
        }
        _ => None,
    });
    let input_events: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::InputNeeded {
                input_id, prompt, ..
            } => Some((*input_id, prompt.as_str())),
            _ => None,
        })
        .collect();
    let completed_id = events.iter().find_map(|event| match event {
        StreamEvent::ToolCallCompleted { call_id, result } if result.output == "Use main." => {
            Some(*call_id)
        }
        _ => None,
    });
    assert_eq!(started_id, Some(input_id));
    assert_eq!(input_events, vec![(input_id, "Which branch should I use?")]);
    assert_eq!(completed_id, Some(input_id));
    let started_index = events
        .iter()
        .position(|event| {
            matches!(event, StreamEvent::ToolCallStarted { call_id, .. } if *call_id == input_id)
        })
        .unwrap();
    let input_index = events
        .iter()
        .position(|event| {
            matches!(event, StreamEvent::InputNeeded { input_id: event_id, .. } if *event_id == input_id)
        })
        .unwrap();
    let completed_index = events
        .iter()
        .position(|event| {
            matches!(event, StreamEvent::ToolCallCompleted { call_id, .. } if *call_id == input_id)
        })
        .unwrap();
    assert!(started_index < input_index);
    assert!(input_index < completed_index);
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: rove::core::types::TerminationReason::Final,
            output: Some(output),
        }) if output == "I will use main."
    ));
}

#[tokio::test]
async fn custom_tool_public_input_provider_call_uses_canonical_lifecycle() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"public_provider_input","args":{"prompt":"Which branch should I use?"}}"#
            .to_string(),
        "done".to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(PublicProviderInputTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("You are a test agent.".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    )
    .with_input_provider(Arc::new(RecordingInputProvider {
        answer: "Use main.".to_string(),
        requests: requests.clone(),
    }));

    let events = collect_events(&engine, "ask through a custom tool").await;
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let input_id = requests[0].0;
    let input_count = events
        .iter()
        .filter(|event| {
            matches!(event, StreamEvent::InputNeeded { input_id: event_id, .. } if *event_id == input_id)
        })
        .count();
    assert_eq!(input_count, 1);
    assert!(events.iter().any(|event| {
        matches!(event, StreamEvent::ToolCallStarted { call_id, name, .. } if *call_id == input_id && name == "public_provider_input")
    }));
    assert!(events.iter().any(|event| {
        matches!(event, StreamEvent::ToolCallCompleted { call_id, result } if *call_id == input_id && result.output == "Use main.")
    }));
}

#[tokio::test]
async fn cancelling_after_input_needed_drops_the_pending_responder() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let model = Box::new(FakeModelClient::new(vec![
        r#"{"tool":"request_input","args":{"prompt":"Which branch?"}}"#.to_string(),
    ]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(RequestInputTool));
    let registered = Arc::new(Mutex::new(Vec::new()));
    let dropped = Arc::new(AtomicBool::new(false));
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
    )
    .with_input_provider(Arc::new(BlockingInputProvider {
        registered: registered.clone(),
        dropped: dropped.clone(),
    }));
    let req = RunRequest {
        session_id: SessionId::new(),
        job_id: JobId::new(),
        run_id: RunId::new(),
        user_message: "wait for input".to_string(),
        resume_state: None,
    };
    let cancel = CancellationToken::new();
    let stream = engine.run_with_cancel(req, None, cancel.clone());
    futures::pin_mut!(stream);

    let mut saw_input = false;
    while let Some(event) = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("stream should reach input")
    {
        if matches!(event, StreamEvent::InputNeeded { .. }) {
            saw_input = true;
            cancel.cancel();
            break;
        }
    }
    assert!(saw_input);

    let completed = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(event) = stream.next().await {
            if matches!(
                event,
                StreamEvent::RunCompleted {
                    reason: TerminationReason::Cancelled,
                    ..
                }
            ) {
                return true;
            }
        }
        false
    })
    .await
    .expect("cancelled input run should finish promptly");
    assert!(completed);
    assert_eq!(registered.lock().unwrap().len(), 1);
    assert!(dropped.load(Ordering::SeqCst));
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

/// Model client that emits a native tool call on the first invocation, then
/// records the messages it receives on the second invocation (to verify the
/// structured tool-use fields round-trip correctly through history).
struct RoundTripRecordingModel {
    call_count: std::sync::atomic::AtomicUsize,
    captured: Arc<Mutex<Option<Vec<Message>>>>,
}

impl RoundTripRecordingModel {
    fn new(captured: Arc<Mutex<Option<Vec<Message>>>>) -> Self {
        Self {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured,
        }
    }
}

#[async_trait]
impl ModelClient for RoundTripRecordingModel {
    fn stream(
        &self,
        messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if idx == 0 {
            return Box::pin(futures::stream::iter([
                Ok(ModelEvent::ToolUseStart {
                    id: "toolu_roundtrip_1".to_string(),
                    name: "echo".to_string(),
                }),
                Ok(ModelEvent::ToolUseDone {
                    id: "toolu_roundtrip_1".to_string(),
                    name: "echo".to_string(),
                    args: serde_json::json!({ "message": "round-trip test" }),
                }),
                Ok(ModelEvent::Usage {
                    usage: Usage::default(),
                }),
            ]));
        }

        *self.captured.lock().unwrap() = Some(messages.to_vec());
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta {
                text: "round-trip complete".to_string(),
            }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
        ]))
    }

    fn model_id(&self) -> &str {
        "round-trip-model"
    }
}

#[tokio::test]
async fn native_tool_use_populates_structured_history_fields() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let captured = Arc::new(Mutex::new(None));
    let model = Box::new(RoundTripRecordingModel::new(captured.clone()));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(EchoTool));
    let engine = Engine::with_workspace(
        model,
        registry,
        ContextManager::new("test".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );

    let events = collect_events(&engine, "trigger native tool use").await;

    // Verify the run completed successfully
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            ..
        })
    ));

    // Verify the messages sent to the model on the second call include
    // structured tool-use fields
    let messages = captured
        .lock()
        .unwrap()
        .take()
        .expect("model was called twice");

    // Find the assistant message with tool_calls
    let assistant_with_tools = messages
        .iter()
        .find(|m| !m.tool_calls.is_empty())
        .expect("assistant message should have tool_calls populated");
    assert_eq!(assistant_with_tools.tool_calls.len(), 1);
    assert_eq!(assistant_with_tools.tool_calls[0].id, "toolu_roundtrip_1");
    assert_eq!(assistant_with_tools.tool_calls[0].name, "echo");
    assert_eq!(
        assistant_with_tools.tool_calls[0].args["message"],
        "round-trip test"
    );

    // Find the tool result message with tool_call_id
    let tool_result = messages
        .iter()
        .find(|m| m.tool_call_id.is_some())
        .expect("tool result message should have tool_call_id populated");
    assert_eq!(
        tool_result.tool_call_id.as_deref(),
        Some("toolu_roundtrip_1")
    );
    assert!(tool_result.content.contains("round-trip test"));
}

/// Model client that emits a batch of native tool calls on the first turn,
/// then captures messages on the second turn to verify structured history.
struct NativeBatchModel {
    call_count: std::sync::atomic::AtomicUsize,
    captured: Arc<Mutex<Option<Vec<Message>>>>,
}

impl NativeBatchModel {
    fn new(captured: Arc<Mutex<Option<Vec<Message>>>>) -> Self {
        Self {
            call_count: std::sync::atomic::AtomicUsize::new(0),
            captured,
        }
    }
}

#[async_trait]
impl ModelClient for NativeBatchModel {
    fn stream(
        &self,
        messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if idx == 0 {
            return Box::pin(futures::stream::iter([
                Ok(ModelEvent::ToolUseStart {
                    id: "batch_call_1".to_string(),
                    name: "probe".to_string(),
                }),
                Ok(ModelEvent::ToolUseDone {
                    id: "batch_call_1".to_string(),
                    name: "probe".to_string(),
                    args: serde_json::json!({ "label": "alpha", "delay_ms": 60 }),
                }),
                Ok(ModelEvent::ToolUseStart {
                    id: "batch_call_2".to_string(),
                    name: "probe".to_string(),
                }),
                Ok(ModelEvent::ToolUseDone {
                    id: "batch_call_2".to_string(),
                    name: "probe".to_string(),
                    args: serde_json::json!({ "label": "beta", "delay_ms": 60 }),
                }),
                Ok(ModelEvent::Usage {
                    usage: Usage::default(),
                }),
            ]));
        }

        *self.captured.lock().unwrap() = Some(messages.to_vec());
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta {
                text: "batch done".to_string(),
            }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
        ]))
    }

    fn model_id(&self) -> &str {
        "native-batch-model"
    }
}

#[tokio::test]
async fn native_multi_tool_call_executes_concurrently_and_round_trips() {
    let tmp = tempfile::TempDir::new().unwrap();
    let workspace = Workspace::detect(tmp.path()).unwrap();
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let max_active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = Arc::new(Mutex::new(None));

    let mut registry = ToolRegistry::new();
    registry.register(Box::new(ProbeTool::new(
        "probe",
        true,
        active.clone(),
        max_active.clone(),
    )));

    let engine = Engine::with_workspace(
        Box::new(NativeBatchModel::new(captured.clone())),
        registry,
        ContextManager::new("test".to_string()),
        EngineConfig {
            max_steps: 5,
            plan_enabled: false,
        },
        workspace,
        ApprovalPolicy::Auto,
    );

    let events = collect_events(&engine, "trigger native batch").await;

    // Both tools ran concurrently — max_active should be 2
    assert_eq!(max_active.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(active.load(std::sync::atomic::Ordering::SeqCst), 0);

    // Both tool results should appear in events
    let completed: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            StreamEvent::ToolCallCompleted { result, .. } => Some(result.output.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(completed.len(), 2);
    assert!(completed.contains(&"alpha"));
    assert!(completed.contains(&"beta"));

    // Run completed successfully
    assert!(matches!(
        events.last(),
        Some(StreamEvent::RunCompleted {
            reason: TerminationReason::Final,
            output: Some(output),
        }) if output == "batch done"
    ));

    // Verify structured history round-trip
    let messages = captured.lock().unwrap().take().expect("model called twice");

    // Assistant message should have both tool_calls
    let assistant_msg = messages
        .iter()
        .find(|m| !m.tool_calls.is_empty())
        .expect("assistant message should have tool_calls");
    assert_eq!(assistant_msg.tool_calls.len(), 2);
    let ids: Vec<&str> = assistant_msg
        .tool_calls
        .iter()
        .map(|tc| tc.id.as_str())
        .collect();
    assert!(ids.contains(&"batch_call_1"));
    assert!(ids.contains(&"batch_call_2"));

    // Both tool result messages should have tool_call_id set
    let tool_results: Vec<_> = messages
        .iter()
        .filter(|m| m.tool_call_id.is_some())
        .collect();
    assert_eq!(tool_results.len(), 2);
    let result_ids: Vec<&str> = tool_results
        .iter()
        .map(|m| m.tool_call_id.as_deref().unwrap())
        .collect();
    assert!(result_ids.contains(&"batch_call_1"));
    assert!(result_ids.contains(&"batch_call_2"));
}
