use std::sync::Arc;

use async_stream::stream;
use futures::future::join_all;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::events::StreamEvent;
use crate::core::executor::Executor;
use crate::core::types::{
    ApprovalDecision, ApprovalPolicy, CallId, Message, ToolApprovalProvider, ToolApprovalRequest,
    ToolCallAction, ToolCallRef, ToolContext, ToolResult, UserInputProvider,
};
use crate::core::workspace::Workspace;
use crate::errors::ToolError;
use crate::hooks::HookRegistry;
use crate::memory::paths::MemoryPaths;
use crate::tools::registry::ToolRegistry;

const APPROVAL_REASON: &str = "destructive tool requires explicit approval";

#[derive(Debug)]
pub(crate) enum ToolAction {
    Call(ToolCallAction),
    Batch(Vec<ToolCallAction>),
}

#[derive(Clone)]
pub(crate) struct ToolTurnContext<'a> {
    pub registry: &'a ToolRegistry,
    pub workspace: &'a Workspace,
    pub memory_paths: &'a MemoryPaths,
    pub approval_policy: ApprovalPolicy,
    pub approval_decision: ApprovalDecision,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub hooks: HookRegistry,
    pub cancel_token: CancellationToken,
}

#[derive(Debug)]
pub(crate) struct ToolExecutionRecord {
    pub call: ToolCallAction,
    pub history_output: String,
    pub error_reason: Option<String>,
}

#[derive(Debug)]
pub(crate) struct ToolTurnOutcome {
    pub records: Vec<ToolExecutionRecord>,
}

impl ToolTurnOutcome {
    pub(crate) fn first_error_reason(&self) -> Option<String> {
        self.records
            .iter()
            .find_map(|record| record.error_reason.clone())
    }
}

#[derive(Debug)]
pub(crate) enum ToolTurnItem {
    Event(StreamEvent),
    Finished(ToolTurnOutcome),
    Cancelled,
}

#[derive(Debug)]
struct ToolExecution {
    call: ToolCallAction,
    result: Result<ToolResult, ToolError>,
}

impl<'a> ToolTurnContext<'a> {
    fn tool_requires_approval(&self, tool_name: &str) -> bool {
        self.approval_policy == ApprovalPolicy::Ask
            && self
                .registry
                .schema(tool_name)
                .map(|schema| schema.destructive)
                .unwrap_or(false)
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

    async fn resolve_approval(
        &self,
        call_id: CallId,
        name: &str,
        args: &serde_json::Value,
    ) -> ApprovalDecision {
        if let Some(provider) = &self.approval_provider {
            provider
                .decide(ToolApprovalRequest {
                    call_id,
                    name: name.to_string(),
                    args: args.clone(),
                    reason: APPROVAL_REASON.to_string(),
                })
                .await
        } else {
            self.approval_decision
        }
    }

    async fn execute_tool_call_with_decision(
        &self,
        call: ToolCallAction,
        approval_decision: ApprovalDecision,
    ) -> ToolExecution {
        let executor = Executor::with_hooks(self.registry, self.hooks.clone());
        let tool_context = ToolContext {
            workspace: self.workspace,
            memory_paths: self.memory_paths.clone(),
            approval_policy: self.effective_approval_policy(&call.name, approval_decision),
            cancel_token: self.cancel_token.clone(),
            input_provider: self.input_provider.clone(),
        };
        let result = executor
            .run(&tool_context, &call.name, call.args.clone(), call.call_id)
            .await;
        ToolExecution { call, result }
    }

    async fn execute_parallel_tool_batch(&self, calls: Vec<ToolCallAction>) -> Vec<ToolExecution> {
        join_all(
            calls
                .into_iter()
                .map(|call| self.execute_tool_call_with_decision(call, self.approval_decision)),
        )
        .await
    }
}

pub(crate) fn run_tool_turn<'a>(
    ctx: ToolTurnContext<'a>,
    action: ToolAction,
) -> BoxStream<'a, ToolTurnItem> {
    Box::pin(stream! {
        let mut executions = Vec::new();
        match action {
            ToolAction::Call(call) => {
                yield ToolTurnItem::Event(StreamEvent::ToolCallStarted {
                    call_id: call.call_id,
                    tool_use_id: call.tool_use_id.clone(),
                    name: call.name.clone(),
                    args: call.args.clone(),
                });

                let approval_decision = if ctx.tool_requires_approval(&call.name) {
                    yield ToolTurnItem::Event(StreamEvent::ModelStatus {
                        status: "waiting_for_approval".to_string(),
                        message: "Waiting for tool approval".to_string(),
                    });
                    yield ToolTurnItem::Event(StreamEvent::ToolCallApprovalNeeded {
                        call_id: call.call_id,
                        name: call.name.clone(),
                        args: call.args.clone(),
                        reason: APPROVAL_REASON.to_string(),
                    });
                    tokio::select! {
                        biased;
                        _ = ctx.cancel_token.cancelled() => {
                            yield ToolTurnItem::Cancelled;
                            return;
                        }
                        decision = ctx.resolve_approval(call.call_id, &call.name, &call.args) => decision,
                    }
                } else {
                    ctx.approval_decision
                };

                let execution = tokio::select! {
                    biased;
                    _ = ctx.cancel_token.cancelled() => {
                        yield ToolTurnItem::Cancelled;
                        return;
                    }
                    execution = ctx.execute_tool_call_with_decision(call, approval_decision) => execution,
                };
                executions.push(execution);
            }
            ToolAction::Batch(calls) => {
                if ctx.can_run_parallel_batch(&calls) {
                    for call in &calls {
                        yield ToolTurnItem::Event(StreamEvent::ToolCallStarted {
                            call_id: call.call_id,
                            tool_use_id: call.tool_use_id.clone(),
                            name: call.name.clone(),
                            args: call.args.clone(),
                        });
                    }
                    executions = tokio::select! {
                        biased;
                        _ = ctx.cancel_token.cancelled() => {
                            yield ToolTurnItem::Cancelled;
                            return;
                        }
                        executions = ctx.execute_parallel_tool_batch(calls) => executions,
                    };
                } else {
                    for call in calls {
                        yield ToolTurnItem::Event(StreamEvent::ToolCallStarted {
                            call_id: call.call_id,
                            tool_use_id: call.tool_use_id.clone(),
                            name: call.name.clone(),
                            args: call.args.clone(),
                        });
                        let approval_decision = if ctx.tool_requires_approval(&call.name) {
                            yield ToolTurnItem::Event(StreamEvent::ModelStatus {
                                status: "waiting_for_approval".to_string(),
                                message: "Waiting for tool approval".to_string(),
                            });
                            yield ToolTurnItem::Event(StreamEvent::ToolCallApprovalNeeded {
                                call_id: call.call_id,
                                name: call.name.clone(),
                                args: call.args.clone(),
                                reason: APPROVAL_REASON.to_string(),
                            });
                            tokio::select! {
                                biased;
                                _ = ctx.cancel_token.cancelled() => {
                                    yield ToolTurnItem::Cancelled;
                                    return;
                                }
                                decision = ctx.resolve_approval(call.call_id, &call.name, &call.args) => decision,
                            }
                        } else {
                            ctx.approval_decision
                        };
                        let execution = tokio::select! {
                            biased;
                            _ = ctx.cancel_token.cancelled() => {
                                yield ToolTurnItem::Cancelled;
                                return;
                            }
                            execution = ctx.execute_tool_call_with_decision(call, approval_decision) => execution,
                        };
                        let failed = execution.result.is_err();
                        executions.push(execution);
                        if failed {
                            break;
                        }
                    }
                }
            }
        }

        let mut records = Vec::new();
        for execution in executions {
            match execution.result {
                Ok(result) => {
                    records.push(ToolExecutionRecord {
                        call: execution.call.clone(),
                        history_output: result.output.clone(),
                        error_reason: None,
                    });
                    yield ToolTurnItem::Event(StreamEvent::ToolCallCompleted {
                        call_id: execution.call.call_id,
                        result,
                    });
                }
                Err(error) => {
                    let reason = error.to_string();
                    records.push(ToolExecutionRecord {
                        call: execution.call.clone(),
                        history_output: format!("Error: {reason}"),
                        error_reason: Some(reason),
                    });
                    yield ToolTurnItem::Event(StreamEvent::ToolCallFailed {
                        call_id: execution.call.call_id,
                        error,
                    });
                }
            }
        }

        yield ToolTurnItem::Finished(ToolTurnOutcome { records });
    })
}

pub(crate) fn append_tool_history(
    history: &mut Vec<Message>,
    full_response: &str,
    outcome: &ToolTurnOutcome,
) {
    let tool_refs: Vec<ToolCallRef> = outcome
        .records
        .iter()
        .filter_map(|record| {
            record.call.tool_use_id.as_ref().map(|id| ToolCallRef {
                id: id.clone(),
                name: record.call.name.clone(),
                args: record.call.args.clone(),
            })
        })
        .collect();
    if tool_refs.is_empty() {
        history.push(Message::assistant(full_response.to_string()));
    } else {
        history.push(Message::assistant_with_tool_calls(
            full_response.to_string(),
            tool_refs,
        ));
    }
    for record in &outcome.records {
        history.push(Message::tool(
            record.history_output.clone(),
            record.call.tool_use_id.clone(),
        ));
    }
}
