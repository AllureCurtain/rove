use std::sync::Arc;

use async_stream::stream;
use futures::stream::{BoxStream, Stream, StreamExt};

use crate::core::context::ContextManager;
use crate::core::events::StreamEvent;
use crate::core::executor::Executor;
use crate::core::parser::parse_action;
use crate::core::planner::Planner;
use crate::core::types::{
    Action, ApprovalDecision, ApprovalPolicy, JobId, Message, Role, RunId, RunRequest,
    TerminationReason, ToolApprovalProvider, ToolApprovalRequest, ToolContext, Usage,
};
use crate::core::workspace::Workspace;
use crate::models::traits::ModelClient;
use crate::state::trace::TraceWriter;
use crate::tools::registry::ToolRegistry;

/// Configuration for the engine's execution limits.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub max_steps: u32,
    pub plan_enabled: bool,
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
        }
    }

    pub fn with_approval_provider(
        mut self,
        approval_provider: Arc<dyn ToolApprovalProvider>,
    ) -> Self {
        self.approval_provider = Some(approval_provider);
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

    /// Run the agent loop for a user message.
    ///
    /// Returns a stream of events. The stream completes when the run terminates.
    pub fn ask(
        &self,
        user_message: String,
        trace_writer: Option<TraceWriter>,
    ) -> impl Stream<Item = StreamEvent> + '_ {
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
    pub fn run(
        &self,
        req: RunRequest,
        trace_writer: Option<TraceWriter>,
    ) -> impl Stream<Item = StreamEvent> + '_ {
        let job_id = req.job_id;
        let run_id = req.run_id;
        let user_message = req.user_message;
        let resume_state = req.resume_state;

        stream! {
            let start_event = StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: user_message.clone(),
            };
            if let Some(ref tw) = trace_writer {
                let _ = tw.append(&start_event);
            }
            yield start_event;

            let mut history: Vec<Message> = resume_state
                .as_ref()
                .map(|state| state.history.clone())
                .unwrap_or_default();
            let mut step: u32 = resume_state.as_ref().map(|state| state.step).unwrap_or(0);
            let mut plan = resume_state.as_ref().and_then(|state| state.plan.clone());

            if self.config.plan_enabled {
                if plan.is_none() {
                    let planner = Planner::new();
                    match planner.draft(self.model.as_ref(), &user_message, &history).await {
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
                            let event = StreamEvent::RunCompleted {
                                reason: TerminationReason::Error,
                                output: Some(format!("Planner error: {err}")),
                            };
                            if let Some(ref tw) = trace_writer {
                                let _ = tw.append(&event);
                            }
                            yield event;
                            return;
                        }
                    }
                }

                let mut final_output: Option<String> = None;
                while let Some(ref mut active_plan) = plan {
                    if active_plan.is_complete() {
                        break;
                    }

                    if step >= self.config.max_steps {
                        let event = StreamEvent::RunCompleted {
                            reason: TerminationReason::StepLimit,
                            output: final_output,
                        };
                        if let Some(ref tw) = trace_writer {
                            let _ = tw.append(&event);
                        }
                        yield event;
                        return;
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
                    let working_memory: Vec<Message> = Vec::new();
                    let messages = self.context_manager.build(&step_prompt, &working_memory, &history);
                    let mut full_response = String::new();
                    let mut usage = Usage::default();
                    let mut model_stream: BoxStream<'_, _> = self.model.stream(
                        &messages,
                        &self.registry.schemas(),
                    );

                    while let Some(chunk_result) = model_stream.next().await {
                        match chunk_result {
                            Ok(chunk) => {
                                if !chunk.delta.is_empty() {
                                    full_response.push_str(&chunk.delta);
                                    let event = StreamEvent::LlmChunk {
                                        delta: chunk.delta,
                                    };
                                    if let Some(ref tw) = trace_writer {
                                        let _ = tw.append(&event);
                                    }
                                    yield event;
                                }
                                if let Some(u) = chunk.usage {
                                    usage = u;
                                }
                            }
                            Err(e) => {
                                let event = StreamEvent::RunCompleted {
                                    reason: TerminationReason::Error,
                                    output: Some(format!("Model error: {}", e)),
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&event);
                                }
                                yield event;
                                return;
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

                    match parse_action(&full_response) {
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
                                self.resolve_approval(call_id, &name, &args, approval_reason)
                                    .await
                            } else {
                                self.approval_decision
                            };

                            let executor = Executor::new(&self.registry);
                            let tool_context = ToolContext {
                                workspace: &self.workspace,
                                approval_policy: self
                                    .effective_approval_policy(&name, approval_decision),
                            };
                            match executor.run(&tool_context, &name, args, call_id).await {
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
                        }
                    }
                }

                let event = StreamEvent::RunCompleted {
                    reason: TerminationReason::Final,
                    output: final_output,
                };
                if let Some(ref tw) = trace_writer {
                    let _ = tw.append(&event);
                }
                yield event;
                return;
            }

            loop {
                if step >= self.config.max_steps {
                    let event = StreamEvent::RunCompleted {
                        reason: TerminationReason::StepLimit,
                        output: None,
                    };
                    if let Some(ref tw) = trace_writer {
                        let _ = tw.append(&event);
                    }
                    yield event;
                    return;
                }
                step += 1;

                // 1. Build prompt
                let working_memory: Vec<Message> = Vec::new();
                let messages = self.context_manager.build(&user_message, &working_memory, &history);

                // 2. Call model (streaming)
                let mut full_response = String::new();
                let mut usage = Usage::default();
                let mut model_stream: BoxStream<'_, _> = self.model.stream(
                    &messages,
                    &self.registry.schemas(),
                );

                while let Some(chunk_result) = model_stream.next().await {
                    match chunk_result {
                        Ok(chunk) => {
                            if !chunk.delta.is_empty() {
                                full_response.push_str(&chunk.delta);
                                let event = StreamEvent::LlmChunk {
                                    delta: chunk.delta,
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&event);
                                }
                                yield event;
                            }
                            if let Some(u) = chunk.usage {
                                usage = u;
                            }
                        }
                        Err(e) => {
                            let event = StreamEvent::RunCompleted {
                                reason: TerminationReason::Error,
                                output: Some(format!("Model error: {}", e)),
                            };
                            if let Some(ref tw) = trace_writer {
                                let _ = tw.append(&event);
                            }
                            yield event;
                            return;
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
                let action = parse_action(&full_response);

                // 4. Handle action
                match action {
                    Action::Final { text } => {
                        let event = StreamEvent::RunCompleted {
                            reason: TerminationReason::Final,
                            output: Some(text),
                        };
                        if let Some(ref tw) = trace_writer {
                            let _ = tw.append(&event);
                        }
                        yield event;
                        return;
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
                            self.resolve_approval(call_id, &name, &args, approval_reason)
                                .await
                        } else {
                            self.approval_decision
                        };

                        let executor = Executor::new(&self.registry);
                        let tool_context = ToolContext {
                            workspace: &self.workspace,
                            approval_policy: self.effective_approval_policy(&name, approval_decision),
                        };
                        match executor.run(&tool_context, &name, args, call_id).await {
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
        }
    }
}
