use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use async_stream::stream;
use futures::future::join_all;
use futures::stream::{BoxStream, Stream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::core::context::{ContextManager, durable_memory_message, session_summary_message};
use crate::core::events::StreamEvent;
use crate::core::executor::Executor;
use crate::core::parser::parse_action;
use crate::core::planner::{Planner, PlannerError};
use crate::core::types::{
    Action, ApprovalDecision, ApprovalPolicy, CallId, JobId, Message, Role, RunId, RunRequest,
    SessionId, TaskPlan, TerminationReason, ToolApprovalProvider, ToolApprovalRequest,
    ToolCallAction, ToolContext, ToolResult, Usage, UserInputProvider,
};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::hooks::{HookRegistry, PostRunHookContext};
use crate::memory::layered::load_prompt_memory_sync;
use crate::models::traits::{ModelClient, ModelEvent};
use crate::state::trace::TraceWriter;
use crate::tools::registry::ToolRegistry;

/// A running engine stream plus immediate identity and cancellation handle.
pub struct RunStream<'e> {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    cancel_token: CancellationToken,
    inner: Pin<Box<dyn Stream<Item = StreamEvent> + Send + 'e>>,
}

impl<'e> RunStream<'e> {
    fn new(
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
        cancel_token: CancellationToken,
        inner: impl Stream<Item = StreamEvent> + Send + 'e,
    ) -> Self {
        Self {
            session_id,
            job_id,
            run_id,
            cancel_token,
            inner: Box::pin(inner),
        }
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn job_id(&self) -> JobId {
        self.job_id
    }

    pub fn run_id(&self) -> RunId {
        self.run_id
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }
}

impl Stream for RunStream<'_> {
    type Item = StreamEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().inner.as_mut().poll_next(cx)
    }
}

impl Unpin for RunStream<'_> {}

impl Drop for RunStream<'_> {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

/// Configuration for the engine's execution limits.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub max_steps: u32,
    pub plan_enabled: bool,
}

#[derive(Debug)]
struct ToolExecution {
    call: ToolCallAction,
    result: Result<ToolResult, ToolError>,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_steps: 20,
            plan_enabled: false,
        }
    }
}

/// The core engine that drives the agent loop.
///
/// Owns the model client, tool registry, context manager, and config.
/// Produces a `Stream<Item = StreamEvent>` that any interface can consume.
pub struct Engine {
    model: Box<dyn ModelClient>,
    registry: ToolRegistry,
    context_manager: ContextManager,
    config: EngineConfig,
    workspace: Workspace,
    approval_policy: ApprovalPolicy,
    approval_decision: ApprovalDecision,
    approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    input_provider: Option<Arc<dyn UserInputProvider>>,
    hooks: HookRegistry,
    memory_recall_limit: usize,
}

impl Engine {
    pub fn new(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
    ) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let workspace = Workspace::detect(&cwd).unwrap_or_else(|_| Workspace {
            root: cwd.clone(),
            kind: crate::core::workspace::WorkspaceKind::Folder,
            state_dir: cwd.join(".rove"),
        });

        Self::with_workspace_and_approval_decision(
            model,
            registry,
            context_manager,
            config,
            workspace,
            ApprovalPolicy::Auto,
            ApprovalDecision::Approve,
        )
    }

    pub fn with_workspace(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
        workspace: Workspace,
        approval_policy: ApprovalPolicy,
    ) -> Self {
        Self::with_workspace_and_approval_decision(
            model,
            registry,
            context_manager,
            config,
            workspace,
            approval_policy,
            ApprovalDecision::Reject,
        )
    }

    pub fn with_workspace_and_approval_decision(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
        workspace: Workspace,
        approval_policy: ApprovalPolicy,
        approval_decision: ApprovalDecision,
    ) -> Self {
        Self {
            model,
            registry,
            context_manager,
            config,
            workspace,
            approval_policy,
            approval_decision,
            approval_provider: None,
            input_provider: None,
            hooks: HookRegistry::with_default_post_run_hooks(),
            memory_recall_limit: 8,
        }
    }

    pub fn with_hooks(mut self, hooks: HookRegistry) -> Self {
        self.hooks = hooks;
        self
    }

    pub fn with_approval_provider(
        mut self,
        approval_provider: Arc<dyn ToolApprovalProvider>,
    ) -> Self {
        self.approval_provider = Some(approval_provider);
        self
    }

    pub fn with_input_provider(mut self, input_provider: Arc<dyn UserInputProvider>) -> Self {
        self.input_provider = Some(input_provider);
        self
    }

    pub fn with_memory_recall_limit(mut self, memory_recall_limit: usize) -> Self {
        self.memory_recall_limit = memory_recall_limit;
        self
    }

    pub fn model_id(&self) -> &str {
        self.model.model_id()
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    fn tool_requires_approval(&self, tool_name: &str) -> bool {
        self.approval_policy == ApprovalPolicy::Ask
            && self
                .registry
                .schema(tool_name)
                .map(|schema| schema.destructive)
                .unwrap_or(false)
    }

    async fn resolve_approval(
        &self,
        call_id: crate::core::types::CallId,
        name: &str,
        args: &serde_json::Value,
        reason: &str,
    ) -> ApprovalDecision {
        if let Some(provider) = &self.approval_provider {
            provider
                .decide(ToolApprovalRequest {
                    call_id,
                    name: name.to_string(),
                    args: args.clone(),
                    reason: reason.to_string(),
                })
                .await
        } else {
            self.approval_decision
        }
    }

    fn effective_approval_policy(
        &self,
        tool_name: &str,
        approval_decision: ApprovalDecision,
    ) -> ApprovalPolicy {
        if self.approval_policy == ApprovalPolicy::Ask
            && self
                .registry
                .schema(tool_name)
                .map(|schema| schema.destructive)
                .unwrap_or(false)
            && approval_decision == ApprovalDecision::Approve
        {
            ApprovalPolicy::Auto
        } else {
            self.approval_policy
        }
    }

    fn can_run_parallel_batch(&self, calls: &[ToolCallAction]) -> bool {
        calls.len() > 1
            && calls
                .iter()
                .all(|call| self.tool_is_parallel_safe(&call.name))
    }

    fn tool_is_parallel_safe(&self, tool_name: &str) -> bool {
        let Ok(schema) = self.registry.schema(tool_name) else {
            return false;
        };
        !schema.destructive && schema.parallel_safe
    }

    async fn execute_tool_call_with_decision(
        &self,
        call: ToolCallAction,
        approval_decision: ApprovalDecision,
        cancel_token: CancellationToken,
    ) -> ToolExecution {
        let executor = Executor::with_hooks(&self.registry, self.hooks.clone());
        let tool_context = ToolContext {
            workspace: &self.workspace,
            approval_policy: self.effective_approval_policy(&call.name, approval_decision),
            cancel_token,
            input_provider: self.input_provider.clone(),
        };
        let result = executor
            .run(&tool_context, &call.name, call.args.clone(), call.call_id)
            .await;
        ToolExecution { call, result }
    }

    async fn execute_parallel_tool_batch(
        &self,
        calls: Vec<ToolCallAction>,
        cancel_token: CancellationToken,
    ) -> Vec<ToolExecution> {
        join_all(calls.into_iter().map(|call| {
            self.execute_tool_call_with_decision(call, self.approval_decision, cancel_token.clone())
        }))
        .await
    }

    async fn draft_plan(&self, goal: &str, history: &[Message]) -> Result<TaskPlan, PlannerError> {
        Planner::new()
            .draft(self.model.as_ref(), goal, history)
            .await
    }

    async fn replan_after_step_failure(
        &self,
        goal: &str,
        step_title: &str,
        reason: &str,
        history: &mut Vec<Message>,
    ) -> Result<TaskPlan, PlannerError> {
        history.push(Message {
            role: Role::User,
            content: planned_step_failure_message(step_title, reason),
        });
        self.draft_plan(goal, history).await
    }

    async fn run_post_run_hooks(
        &self,
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
        reason: TerminationReason,
        output: Option<String>,
        cancel_token: CancellationToken,
    ) {
        let ctx = PostRunHookContext {
            workspace: &self.workspace,
            session_id,
            job_id,
            run_id,
            reason,
            output,
            cancel_token,
        };
        self.hooks.run_post_run(&ctx).await;
    }

    /// Run the agent loop for a user message.
    ///
    /// Returns a stream of events. The stream completes when the run terminates.
    pub fn ask(&self, user_message: String, trace_writer: Option<TraceWriter>) -> RunStream<'_> {
        let req = RunRequest {
            session_id: crate::core::types::SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message,
            resume_state: None,
        };

        self.run(req, trace_writer)
    }

    /// Run the agent loop for an explicit request.
    ///
    /// The caller owns run identity so persisted artifacts and streamed events stay aligned.
    pub fn run(&self, req: RunRequest, trace_writer: Option<TraceWriter>) -> RunStream<'_> {
        self.run_with_cancel(req, trace_writer, CancellationToken::new())
    }

    /// Run the agent loop with an interface-owned cancellation token.
    pub fn run_with_cancel(
        &self,
        req: RunRequest,
        trace_writer: Option<TraceWriter>,
        cancel: CancellationToken,
    ) -> RunStream<'_> {
        let session_id = req.session_id;
        let job_id = req.job_id;
        let run_id = req.run_id;
        let user_message = req.user_message;
        let resume_state = req.resume_state;
        let stream_cancel = cancel.clone();

        RunStream::new(
            session_id,
            job_id,
            run_id,
            cancel,
            stream! {
                macro_rules! complete_run {
                    ($reason:expr, $output:expr) => {{
                        let reason = $reason;
                        let output = $output;
                        let event = StreamEvent::RunCompleted {
                            reason: reason.clone(),
                            output: output.clone(),
                        };
                        append_trace(&trace_writer, &event);
                        yield event;
                        self.run_post_run_hooks(
                            session_id,
                            job_id,
                            run_id,
                            reason,
                            output,
                            stream_cancel.clone(),
                        )
                        .await;
                        return;
                    }};
                }
                macro_rules! yield_traced {
                    ($event:expr) => {{
                        let event = $event;
                        append_trace(&trace_writer, &event);
                        yield event;
                    }};
                }

                let start_event = StreamEvent::RunStarted {
                    run_id,
                    job_id,
                    user_message: user_message.clone(),
                };
                if let Some(ref tw) = trace_writer {
                    let _ = tw.append(&start_event);
                }
                yield start_event;

                if stream_cancel.is_cancelled() {
                    complete_run!(TerminationReason::Cancelled, None);
                }

                let resume_checkpoint = resume_state
                    .as_ref()
                    .and_then(|state| state.checkpoint.as_ref());
                let mut history: Vec<Message> = resume_checkpoint
                    .map(|checkpoint| checkpoint.preserved_tail.clone())
                    .or_else(|| {
                        resume_state
                            .as_ref()
                            .map(|state| state.history.clone())
                    })
                    .unwrap_or_default();
                let compact_summary = resume_checkpoint
                    .and_then(|checkpoint| checkpoint.summary.clone());
                let resume_summary = resume_state
                    .as_ref()
                    .and_then(|state| state.summary.as_deref());
                let prompt_memory = load_prompt_memory_sync(
                    &self.workspace,
                    session_id,
                    resume_summary,
                    &user_message,
                    self.memory_recall_limit,
                )
                .unwrap_or_default();
                let mut working_memory: Vec<Message> = Vec::new();
                if let Some(index) = prompt_memory.durable_index {
                    working_memory.push(durable_memory_message(&index));
                }
                if let Some(summary) = prompt_memory.session_summary {
                    working_memory.push(session_summary_message(&summary));
                }
                let mut step: u32 = resume_checkpoint
                    .map(|checkpoint| checkpoint.last_step)
                    .or_else(|| resume_state.as_ref().map(|state| state.step))
                    .unwrap_or(0);
                let mut plan = resume_checkpoint
                    .and_then(|checkpoint| checkpoint.plan.clone())
                    .or_else(|| resume_state.as_ref().and_then(|state| state.plan.clone()));

                if self.config.plan_enabled {
                    if plan.is_none() {
                        let draft_result = tokio::select! {
                            biased;
                            _ = stream_cancel.cancelled() => {
                                complete_run!(TerminationReason::Cancelled, None);
                            }
                            result = self.draft_plan(&user_message, &history) => result,
                        };
                        match draft_result {
                            Ok(drafted) => {
                                let event = StreamEvent::PlanCreated {
                                    plan: drafted.clone(),
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&event);
                                }
                                yield event;
                                plan = Some(drafted);
                            }
                            Err(err) => {
                                complete_run!(
                                    TerminationReason::Error,
                                    Some(format!("Planner error: {err}"))
                                );
                            }
                        }
                    }

                    let mut final_output: Option<String> = None;
                    while let Some(ref mut active_plan) = plan {
                        if stream_cancel.is_cancelled() {
                            complete_run!(TerminationReason::Cancelled, None);
                        }

                        if active_plan.is_complete() {
                            break;
                        }

                        if step >= self.config.max_steps {
                            complete_run!(TerminationReason::StepLimit, final_output);
                        }

                        let Some(current_step) = active_plan.current_step().cloned() else {
                            break;
                        };
                        let current_index = active_plan.current_step;
                        let started = StreamEvent::PlanStepStarted {
                            step: current_step.clone(),
                            index: current_index,
                        };
                        if let Some(ref tw) = trace_writer {
                            let _ = tw.append(&started);
                        }
                        yield started;

                        step += 1;
                        let step_prompt = format!(
                            "Goal: {}\nCurrent step {}: {}\nComplete this step and report the result.",
                            active_plan.goal, current_step.id, current_step.title
                        );
                        let context = self.context_manager.build_with_checkpoint(
                            &step_prompt,
                            &working_memory,
                            compact_summary.as_deref(),
                            &history,
                        );
                        if context.over_hard_limit {
                            complete_run!(
                                TerminationReason::TokenLimit,
                                Some("context exceeds configured hard token budget".to_string())
                            );
                        }
                        let messages = context.messages;
                        let mut full_response = String::new();
                        let mut usage = Usage::default();
                        let mut native_tool_calls = Vec::new();
                        let mut model_stream: BoxStream<'_, _> = self.model.stream(
                            &messages,
                            &self.registry.schemas(),
                        );

                        loop {
                            let chunk_result = tokio::select! {
                                biased;
                                _ = stream_cancel.cancelled() => {
                                    complete_run!(TerminationReason::Cancelled, None);
                                }
                                chunk = model_stream.next() => chunk,
                            };
                            let Some(chunk_result) = chunk_result else {
                                break;
                            };
                            match chunk_result {
                                Ok(model_event) => match model_event {
                                    ModelEvent::TextDelta { text } => {
                                        if !text.is_empty() {
                                            full_response.push_str(&text);
                                            let event = StreamEvent::LlmChunk { delta: text };
                                            if let Some(ref tw) = trace_writer {
                                                let _ = tw.append(&event);
                                            }
                                            yield event;
                                        }
                                    }
                                    ModelEvent::Usage { usage: u } => {
                                        usage = u;
                                    }
                                    ModelEvent::Done => break,
                                    ModelEvent::ThinkingDelta { .. }
                                    | ModelEvent::ToolUseStart { .. }
                                    | ModelEvent::ToolUseDelta { .. } => {}
                                    | ModelEvent::ToolUseDone { name, args, .. } => {
                                        native_tool_calls.push(ToolCallAction {
                                            call_id: CallId::new(),
                                            name,
                                            args,
                                        });
                                    }
                                },
                                Err(e) => {
                                    complete_run!(
                                        TerminationReason::Error,
                                        Some(format!("Model error: {}", e))
                                    );
                                }
                            }
                        }

                        let msg_event = StreamEvent::LlmMessage {
                            full: full_response.clone(),
                            usage,
                        };
                        if let Some(ref tw) = trace_writer {
                            let _ = tw.append(&msg_event);
                        }
                        yield msg_event;

                        let action = action_from_tool_calls(native_tool_calls, &full_response);

                        match action {
                            Action::ToolCall { call_id, name, args } => {
                                let start_event = StreamEvent::ToolCallStarted {
                                    call_id,
                                    name: name.clone(),
                                    args: args.clone(),
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&start_event);
                                }
                                yield start_event;

                                let approval_reason = "destructive tool requires explicit approval";
                                let approval_decision = if self.tool_requires_approval(&name) {
                                    let approval_event = StreamEvent::ToolCallApprovalNeeded {
                                        call_id,
                                        name: name.clone(),
                                        args: args.clone(),
                                        reason: approval_reason.to_string(),
                                    };
                                    if let Some(ref tw) = trace_writer {
                                        let _ = tw.append(&approval_event);
                                    }
                                    yield approval_event;
                                    tokio::select! {
                                        biased;
                                        _ = stream_cancel.cancelled() => {
                                            complete_run!(TerminationReason::Cancelled, None);
                                        }
                                        decision = self.resolve_approval(call_id, &name, &args, approval_reason) => decision,
                                    }
                                } else {
                                    self.approval_decision
                                };

                                let executor =
                                    Executor::with_hooks(&self.registry, self.hooks.clone());
                                let tool_context = ToolContext {
                                    workspace: &self.workspace,
                                    approval_policy: self
                                        .effective_approval_policy(&name, approval_decision),
                                    cancel_token: stream_cancel.clone(),
                                    input_provider: self.input_provider.clone(),
                                };
                                let tool_result = tokio::select! {
                                    biased;
                                    _ = stream_cancel.cancelled() => {
                                        complete_run!(TerminationReason::Cancelled, None);
                                    }
                                    result = executor.run(&tool_context, &name, args, call_id) => result,
                                };
                                match tool_result {
                                    Ok(result) => {
                                        history.push(Message {
                                            role: Role::Assistant,
                                            content: full_response.clone(),
                                        });
                                        history.push(Message {
                                            role: Role::Tool,
                                            content: result.output.clone(),
                                        });

                                        let event = StreamEvent::ToolCallCompleted {
                                            call_id,
                                            result,
                                        };
                                        if let Some(ref tw) = trace_writer {
                                            let _ = tw.append(&event);
                                        }
                                        yield event;
                                    }
                                    Err(e) => {
                                        let reason = e.to_string();
                                        history.push(Message {
                                            role: Role::Assistant,
                                            content: full_response.clone(),
                                        });
                                        history.push(Message {
                                            role: Role::Tool,
                                            content: format!("Error: {reason}"),
                                        });

                                        let event = StreamEvent::ToolCallFailed {
                                            call_id,
                                            error: e,
                                        };
                                        if let Some(ref tw) = trace_writer {
                                            let _ = tw.append(&event);
                                        }
                                        yield event;

                                        let failed = StreamEvent::PlanStepFailed {
                                            step: current_step.clone(),
                                            index: current_index,
                                            reason: reason.clone(),
                                        };
                                        if let Some(ref tw) = trace_writer {
                                            let _ = tw.append(&failed);
                                        }
                                        yield failed;

                                        let replan_result = tokio::select! {
                                            biased;
                                            _ = stream_cancel.cancelled() => {
                                                complete_run!(TerminationReason::Cancelled, None);
                                            }
                                            result = self.replan_after_step_failure(
                                                &active_plan.goal,
                                                &current_step.title,
                                                &reason,
                                                &mut history,
                                            ) => result,
                                        };
                                        match replan_result {
                                            Ok(drafted) => {
                                                let event = StreamEvent::PlanCreated {
                                                    plan: drafted.clone(),
                                                };
                                                if let Some(ref tw) = trace_writer {
                                                    let _ = tw.append(&event);
                                                }
                                                yield event;
                                                *active_plan = drafted;
                                            }
                                            Err(err) => {
                                                complete_run!(
                                                    TerminationReason::Error,
                                                    Some(format!("Planner error: {err}"))
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            Action::ToolBatch { calls } => {
                                let mut executions = Vec::new();
                                if self.can_run_parallel_batch(&calls) {
                                    for call in &calls {
                                        yield_traced!(StreamEvent::ToolCallStarted {
                                            call_id: call.call_id,
                                            name: call.name.clone(),
                                            args: call.args.clone(),
                                        });
                                    }
                                    executions = tokio::select! {
                                        biased;
                                        _ = stream_cancel.cancelled() => {
                                            complete_run!(TerminationReason::Cancelled, None);
                                        }
                                        executions = self.execute_parallel_tool_batch(calls, stream_cancel.clone()) => executions,
                                    };
                                } else {
                                    let approval_reason = "destructive tool requires explicit approval";
                                    for call in calls {
                                        yield_traced!(StreamEvent::ToolCallStarted {
                                            call_id: call.call_id,
                                            name: call.name.clone(),
                                            args: call.args.clone(),
                                        });
                                        let approval_decision = if self.tool_requires_approval(&call.name) {
                                            yield_traced!(StreamEvent::ToolCallApprovalNeeded {
                                                call_id: call.call_id,
                                                name: call.name.clone(),
                                                args: call.args.clone(),
                                                reason: approval_reason.to_string(),
                                            });
                                            tokio::select! {
                                                biased;
                                                _ = stream_cancel.cancelled() => {
                                                    complete_run!(TerminationReason::Cancelled, None);
                                                }
                                                decision = self.resolve_approval(
                                                    call.call_id,
                                                    &call.name,
                                                    &call.args,
                                                    approval_reason
                                                ) => decision,
                                            }
                                        } else {
                                            self.approval_decision
                                        };
                                        let execution = tokio::select! {
                                            biased;
                                            _ = stream_cancel.cancelled() => {
                                                complete_run!(TerminationReason::Cancelled, None);
                                            }
                                            execution = self.execute_tool_call_with_decision(
                                                call,
                                                approval_decision,
                                                stream_cancel.clone()
                                            ) => execution,
                                        };
                                        let failed = execution.result.is_err();
                                        executions.push(execution);
                                        if failed {
                                            break;
                                        }
                                    }
                                }

                                let mut first_error_reason = None;
                                history.push(Message {
                                    role: Role::Assistant,
                                    content: full_response.clone(),
                                });
                                for execution in executions {
                                    match execution.result {
                                        Ok(result) => {
                                            history.push(Message {
                                                role: Role::Tool,
                                                content: result.output.clone(),
                                            });
                                            yield_traced!(StreamEvent::ToolCallCompleted {
                                                call_id: execution.call.call_id,
                                                result,
                                            });
                                        }
                                        Err(error) => {
                                            let reason = error.to_string();
                                            first_error_reason.get_or_insert_with(|| reason.clone());
                                            history.push(Message {
                                                role: Role::Tool,
                                                content: format!("Error: {reason}"),
                                            });
                                            yield_traced!(StreamEvent::ToolCallFailed {
                                                call_id: execution.call.call_id,
                                                error,
                                            });
                                        }
                                    }
                                }

                                if let Some(reason) = first_error_reason {
                                    let failed = StreamEvent::PlanStepFailed {
                                        step: current_step.clone(),
                                        index: current_index,
                                        reason: reason.clone(),
                                    };
                                    if let Some(ref tw) = trace_writer {
                                        let _ = tw.append(&failed);
                                    }
                                    yield failed;

                                    let replan_result = tokio::select! {
                                        biased;
                                        _ = stream_cancel.cancelled() => {
                                            complete_run!(TerminationReason::Cancelled, None);
                                        }
                                        result = self.replan_after_step_failure(
                                            &active_plan.goal,
                                            &current_step.title,
                                            &reason,
                                            &mut history,
                                        ) => result,
                                    };
                                    match replan_result {
                                        Ok(drafted) => {
                                            let event = StreamEvent::PlanCreated {
                                                plan: drafted.clone(),
                                            };
                                            if let Some(ref tw) = trace_writer {
                                                let _ = tw.append(&event);
                                            }
                                            yield event;
                                            *active_plan = drafted;
                                        }
                                        Err(err) => {
                                            complete_run!(
                                                TerminationReason::Error,
                                                Some(format!("Planner error: {err}"))
                                            );
                                        }
                                    }
                                }
                            }
                            Action::Final { text } => {
                                history.push(Message {
                                    role: Role::Assistant,
                                    content: text.clone(),
                                });
                                final_output = Some(text);
                                active_plan.mark_current_done();
                                let completed = StreamEvent::PlanStepCompleted {
                                    step: current_step,
                                    index: current_index,
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&completed);
                                }
                                yield completed;
                            }
                            Action::Malformed { reason } => {
                                history.push(Message {
                                    role: Role::Assistant,
                                    content: full_response.clone(),
                                });
                                history.push(Message {
                                    role: Role::User,
                                    content: format!(
                                        "Your previous output could not be parsed: {}. Please try again.",
                                        reason
                                    ),
                                });
                                let failed = StreamEvent::PlanStepFailed {
                                    step: current_step.clone(),
                                    index: current_index,
                                    reason: reason.clone(),
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&failed);
                                }
                                yield failed;

                                let replan_result = tokio::select! {
                                    biased;
                                    _ = stream_cancel.cancelled() => {
                                        complete_run!(TerminationReason::Cancelled, None);
                                    }
                                    result = self.replan_after_step_failure(
                                        &active_plan.goal,
                                        &current_step.title,
                                        &reason,
                                        &mut history,
                                    ) => result,
                                };
                                match replan_result {
                                    Ok(drafted) => {
                                        let event = StreamEvent::PlanCreated {
                                            plan: drafted.clone(),
                                        };
                                        if let Some(ref tw) = trace_writer {
                                            let _ = tw.append(&event);
                                        }
                                        yield event;
                                        *active_plan = drafted;
                                    }
                                    Err(err) => {
                                        complete_run!(
                                            TerminationReason::Error,
                                            Some(format!("Planner error: {err}"))
                                        );
                                    }
                                }
                            }
                        }
                    }

                    complete_run!(TerminationReason::Final, final_output);
                }

                loop {
                    if stream_cancel.is_cancelled() {
                        complete_run!(TerminationReason::Cancelled, None);
                    }

                    if step >= self.config.max_steps {
                        complete_run!(TerminationReason::StepLimit, None);
                    }
                    step += 1;

                    // 1. Build prompt
                    let context = self.context_manager.build_with_checkpoint(
                        &user_message,
                        &working_memory,
                        compact_summary.as_deref(),
                        &history,
                    );
                    if context.over_hard_limit {
                        complete_run!(
                            TerminationReason::TokenLimit,
                            Some("context exceeds configured hard token budget".to_string())
                        );
                    }
                    let messages = context.messages;

                    // 2. Call model (streaming)
                    let mut full_response = String::new();
                    let mut usage = Usage::default();
                    let mut native_tool_calls = Vec::new();
                    let mut model_stream: BoxStream<'_, _> = self.model.stream(
                        &messages,
                        &self.registry.schemas(),
                    );

                    loop {
                        let chunk_result = tokio::select! {
                            biased;
                            _ = stream_cancel.cancelled() => {
                                complete_run!(TerminationReason::Cancelled, None);
                            }
                            chunk = model_stream.next() => chunk,
                        };
                        let Some(chunk_result) = chunk_result else {
                            break;
                        };
                        match chunk_result {
                            Ok(model_event) => match model_event {
                                ModelEvent::TextDelta { text } => {
                                    if !text.is_empty() {
                                        full_response.push_str(&text);
                                        let event = StreamEvent::LlmChunk { delta: text };
                                        if let Some(ref tw) = trace_writer {
                                            let _ = tw.append(&event);
                                        }
                                        yield event;
                                    }
                                }
                                ModelEvent::Usage { usage: u } => {
                                    usage = u;
                                }
                                ModelEvent::Done => break,
                                ModelEvent::ThinkingDelta { .. }
                                | ModelEvent::ToolUseStart { .. }
                                | ModelEvent::ToolUseDelta { .. } => {}
                                | ModelEvent::ToolUseDone { name, args, .. } => {
                                    native_tool_calls.push(ToolCallAction {
                                        call_id: CallId::new(),
                                        name,
                                        args,
                                    });
                                }
                            },
                            Err(e) => {
                                complete_run!(
                                    TerminationReason::Error,
                                    Some(format!("Model error: {}", e))
                                );
                            }
                        }
                    }

                    // Emit full message event
                    let msg_event = StreamEvent::LlmMessage {
                        full: full_response.clone(),
                        usage: usage.clone(),
                    };
                    if let Some(ref tw) = trace_writer {
                        let _ = tw.append(&msg_event);
                    }
                    yield msg_event;

                    // 3. Parse action
                    let action = action_from_tool_calls(native_tool_calls, &full_response);

                    // 4. Handle action
                    match action {
                        Action::Final { text } => {
                            complete_run!(TerminationReason::Final, Some(text));
                        }
                        Action::ToolCall { call_id, name, args } => {
                            let start_event = StreamEvent::ToolCallStarted {
                                call_id,
                                name: name.clone(),
                                args: args.clone(),
                            };
                            if let Some(ref tw) = trace_writer {
                                let _ = tw.append(&start_event);
                            }
                            yield start_event;

                            let approval_reason = "destructive tool requires explicit approval";
                            let approval_decision = if self.tool_requires_approval(&name) {
                                let approval_event = StreamEvent::ToolCallApprovalNeeded {
                                    call_id,
                                    name: name.clone(),
                                    args: args.clone(),
                                    reason: approval_reason.to_string(),
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&approval_event);
                                }
                                yield approval_event;
                                tokio::select! {
                                    biased;
                                    _ = stream_cancel.cancelled() => {
                                        complete_run!(TerminationReason::Cancelled, None);
                                    }
                                    decision = self.resolve_approval(call_id, &name, &args, approval_reason) => decision,
                                }
                            } else {
                                self.approval_decision
                            };

                            let executor = Executor::with_hooks(&self.registry, self.hooks.clone());
                            let tool_context = ToolContext {
                                workspace: &self.workspace,
                                approval_policy: self.effective_approval_policy(&name, approval_decision),
                                cancel_token: stream_cancel.clone(),
                                input_provider: self.input_provider.clone(),
                            };
                            let tool_result = tokio::select! {
                                biased;
                                _ = stream_cancel.cancelled() => {
                                    complete_run!(TerminationReason::Cancelled, None);
                                }
                                result = executor.run(&tool_context, &name, args, call_id) => result,
                            };
                            match tool_result {
                                Ok(result) => {
                                    // Add assistant message + tool result to history
                                    history.push(Message {
                                        role: Role::Assistant,
                                        content: full_response.clone(),
                                    });
                                    history.push(Message {
                                        role: Role::Tool,
                                        content: result.output.clone(),
                                    });

                                    let event = StreamEvent::ToolCallCompleted {
                                        call_id,
                                        result,
                                    };
                                    if let Some(ref tw) = trace_writer {
                                        let _ = tw.append(&event);
                                    }
                                    yield event;
                                }
                                Err(e) => {
                                    // Feed error back to LLM
                                    history.push(Message {
                                        role: Role::Assistant,
                                        content: full_response.clone(),
                                    });
                                    history.push(Message {
                                        role: Role::Tool,
                                        content: format!("Error: {}", e),
                                    });

                                    let event = StreamEvent::ToolCallFailed {
                                        call_id,
                                        error: e,
                                    };
                                    if let Some(ref tw) = trace_writer {
                                        let _ = tw.append(&event);
                                    }
                                    yield event;
                                }
                            }
                        }
                        Action::ToolBatch { calls } => {
                            let mut executions = Vec::new();
                            if self.can_run_parallel_batch(&calls) {
                                for call in &calls {
                                    yield_traced!(StreamEvent::ToolCallStarted {
                                        call_id: call.call_id,
                                        name: call.name.clone(),
                                        args: call.args.clone(),
                                    });
                                }
                                executions = tokio::select! {
                                    biased;
                                    _ = stream_cancel.cancelled() => {
                                        complete_run!(TerminationReason::Cancelled, None);
                                    }
                                    executions = self.execute_parallel_tool_batch(calls, stream_cancel.clone()) => executions,
                                };
                            } else {
                                let approval_reason = "destructive tool requires explicit approval";
                                for call in calls {
                                    yield_traced!(StreamEvent::ToolCallStarted {
                                        call_id: call.call_id,
                                        name: call.name.clone(),
                                        args: call.args.clone(),
                                    });
                                    let approval_decision = if self.tool_requires_approval(&call.name) {
                                        yield_traced!(StreamEvent::ToolCallApprovalNeeded {
                                            call_id: call.call_id,
                                            name: call.name.clone(),
                                            args: call.args.clone(),
                                            reason: approval_reason.to_string(),
                                        });
                                        tokio::select! {
                                            biased;
                                            _ = stream_cancel.cancelled() => {
                                                complete_run!(TerminationReason::Cancelled, None);
                                            }
                                            decision = self.resolve_approval(
                                                call.call_id,
                                                &call.name,
                                                &call.args,
                                                approval_reason
                                            ) => decision,
                                        }
                                    } else {
                                        self.approval_decision
                                    };
                                    let execution = tokio::select! {
                                        biased;
                                        _ = stream_cancel.cancelled() => {
                                            complete_run!(TerminationReason::Cancelled, None);
                                        }
                                        execution = self.execute_tool_call_with_decision(
                                            call,
                                            approval_decision,
                                            stream_cancel.clone()
                                        ) => execution,
                                    };
                                    let failed = execution.result.is_err();
                                    executions.push(execution);
                                    if failed {
                                        break;
                                    }
                                }
                            }

                            history.push(Message {
                                role: Role::Assistant,
                                content: full_response.clone(),
                            });
                            for execution in executions {
                                match execution.result {
                                    Ok(result) => {
                                        history.push(Message {
                                            role: Role::Tool,
                                            content: result.output.clone(),
                                        });
                                        yield_traced!(StreamEvent::ToolCallCompleted {
                                            call_id: execution.call.call_id,
                                            result,
                                        });
                                    }
                                    Err(error) => {
                                        history.push(Message {
                                            role: Role::Tool,
                                            content: format!("Error: {error}"),
                                        });
                                        yield_traced!(StreamEvent::ToolCallFailed {
                                            call_id: execution.call.call_id,
                                            error,
                                        });
                                    }
                                }
                            }
                        }
                        Action::Malformed { reason } => {
                            // Feed parse failure back to LLM for self-correction
                            history.push(Message {
                                role: Role::Assistant,
                                content: full_response.clone(),
                            });
                            history.push(Message {
                                role: Role::User,
                                content: format!(
                                    "Your previous output could not be parsed: {}. Please try again.",
                                    reason
                                ),
                            });
                        }
                    }
                }
            },
        )
    }
}

pub(crate) fn planned_step_failure_message(step_title: &str, reason: &str) -> String {
    format!("Planned step failed: {step_title}. Reason: {reason}. Re-plan the remaining work.")
}

fn append_trace(trace_writer: &Option<TraceWriter>, event: &StreamEvent) {
    if let Some(tw) = trace_writer {
        let _ = tw.append(event);
    }
}

fn action_from_tool_calls(calls: Vec<ToolCallAction>, full_response: &str) -> Action {
    match calls.len() {
        0 => parse_action(full_response),
        1 => {
            let call = calls.into_iter().next().expect("one tool call");
            Action::ToolCall {
                call_id: call.call_id,
                name: call.name,
                args: call.args,
            }
        }
        _ => Action::ToolBatch { calls },
    }
}
