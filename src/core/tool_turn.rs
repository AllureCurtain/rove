use std::sync::Arc;

use async_stream::stream;
use futures::future::join_all;
use futures::stream::{BoxStream, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::events::StreamEvent;
use crate::core::executor::Executor;
use crate::core::tool_input::RegisteredUserInput;
use crate::core::types::{
    ApprovalDecision, ApprovalPolicy, CallId, Message, PendingToolApproval, ToolApprovalProvider,
    ToolApprovalRequest, ToolCallAction, ToolCallRef, ToolContext, ToolExecutionMetadata,
    ToolExecutionStatus, ToolResult, ToolRiskLevel, UserInputProvider,
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

#[derive(Debug)]
enum ToolExecutionItem {
    InputNeeded(RegisteredUserInput),
    Finished(Box<ToolExecution>),
    Cancelled,
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

    async fn begin_approval(
        &self,
        call_id: CallId,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<PendingToolApproval, ToolError> {
        let request = ToolApprovalRequest {
            call_id,
            name: name.to_string(),
            args: args.clone(),
            reason: APPROVAL_REASON.to_string(),
        };
        if let Some(provider) = &self.approval_provider {
            provider.begin_approval(request).await
        } else {
            let decision = self.approval_decision;
            Ok(PendingToolApproval::new(async move { decision }))
        }
    }

    async fn execute_tool_call_with_decision(
        &self,
        call: ToolCallAction,
        approval_decision: ApprovalDecision,
        input_events: Option<mpsc::Sender<RegisteredUserInput>>,
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
            .run_with_input_events(
                &tool_context,
                &call.name,
                call.args.clone(),
                call.call_id,
                input_events,
            )
            .await;
        ToolExecution { call, result }
    }

    fn execute_tool_call_stream<'b>(
        &'b self,
        call: ToolCallAction,
        approval_decision: ApprovalDecision,
    ) -> BoxStream<'b, ToolExecutionItem> {
        Box::pin(stream! {
            let (input_events, mut input_requests) = mpsc::channel(1);
            let execution = self.execute_tool_call_with_decision(
                call,
                approval_decision,
                Some(input_events),
            );
            tokio::pin!(execution);
            let mut input_requests_open = true;

            loop {
                tokio::select! {
                    biased;
                    _ = self.cancel_token.cancelled() => {
                        yield ToolExecutionItem::Cancelled;
                        return;
                    }
                    request = input_requests.recv(), if input_requests_open => {
                        match request {
                            Some(request) => yield ToolExecutionItem::InputNeeded(request),
                            None => input_requests_open = false,
                        }
                    }
                    execution = &mut execution => {
                        yield ToolExecutionItem::Finished(Box::new(execution));
                        return;
                    }
                }
            }
        })
    }

    async fn execute_parallel_tool_batch(&self, calls: Vec<ToolCallAction>) -> Vec<ToolExecution> {
        join_all(
            calls.into_iter().map(|call| {
                self.execute_tool_call_with_decision(call, self.approval_decision, None)
            }),
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
                    let pending_approval = tokio::select! {
                        biased;
                        _ = ctx.cancel_token.cancelled() => {
                            yield ToolTurnItem::Cancelled;
                            return;
                        }
                        pending = ctx.begin_approval(call.call_id, &call.name, &call.args) => pending,
                    };
                    if let Ok(pending_approval) = pending_approval {
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
                            decision = pending_approval.resolve() => decision,
                        }
                    } else {
                        ApprovalDecision::Reject
                    }
                } else {
                    ctx.approval_decision
                };

                let mut execution_stream = ctx.execute_tool_call_stream(call, approval_decision);
                while let Some(item) = execution_stream.next().await {
                    match item {
                        ToolExecutionItem::InputNeeded(input) => {
                            yield ToolTurnItem::Event(StreamEvent::InputNeeded {
                                input_id: input.input_id,
                                prompt: input.request.prompt,
                            });
                            let _ = input.acknowledged.send(());
                        }
                        ToolExecutionItem::Finished(execution) => {
                            executions.push(*execution);
                            break;
                        }
                        ToolExecutionItem::Cancelled => {
                            yield ToolTurnItem::Cancelled;
                            return;
                        }
                    }
                }
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
                            let pending_approval = tokio::select! {
                                biased;
                                _ = ctx.cancel_token.cancelled() => {
                                    yield ToolTurnItem::Cancelled;
                                    return;
                                }
                                pending = ctx.begin_approval(call.call_id, &call.name, &call.args) => pending,
                            };
                            if let Ok(pending_approval) = pending_approval {
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
                                    decision = pending_approval.resolve() => decision,
                                }
                            } else {
                                ApprovalDecision::Reject
                            }
                        } else {
                            ctx.approval_decision
                        };
                        let mut execution_stream =
                            ctx.execute_tool_call_stream(call, approval_decision);
                        let mut failed = false;
                        while let Some(item) = execution_stream.next().await {
                            match item {
                                ToolExecutionItem::InputNeeded(input) => {
                                    yield ToolTurnItem::Event(StreamEvent::InputNeeded {
                                        input_id: input.input_id,
                                        prompt: input.request.prompt,
                                    });
                                    let _ = input.acknowledged.send(());
                                }
                                ToolExecutionItem::Finished(execution) => {
                                    failed = execution.result.is_err();
                                    executions.push(*execution);
                                    break;
                                }
                                ToolExecutionItem::Cancelled => {
                                    yield ToolTurnItem::Cancelled;
                                    return;
                                }
                            }
                        }
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
                    let metadata = failure_metadata(&error);
                    let reason = error.to_string();
                    records.push(ToolExecutionRecord {
                        call: execution.call.clone(),
                        history_output: format!("Error: {reason}"),
                        error_reason: Some(reason),
                    });
                    yield ToolTurnItem::Event(StreamEvent::ToolCallFailed {
                        call_id: execution.call.call_id,
                        error,
                        metadata,
                    });
                }
            }
        }

        yield ToolTurnItem::Finished(ToolTurnOutcome { records });
    })
}

fn failure_metadata(error: &ToolError) -> ToolExecutionMetadata {
    ToolExecutionMetadata {
        status: ToolExecutionStatus::Error,
        error_code: Some(error.error_code().to_string()),
        security_event_type: security_event_type(error),
        risk_level: ToolRiskLevel::High,
        read_only: false,
        affected_paths: Vec::new(),
        workspace_changed: false,
        diff_summary: Vec::new(),
    }
}

fn security_event_type(error: &ToolError) -> Option<String> {
    match error {
        ToolError::PermissionDenied { reason } if reason.contains("path escapes workspace") => {
            Some("path_escape".to_string())
        }
        ToolError::PermissionDenied { .. } => Some("approval_denied".to_string()),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use tokio_util::sync::CancellationToken;

    use super::{ToolAction, ToolTurnContext, ToolTurnItem, run_tool_turn};
    use crate::core::types::{
        ApprovalDecision, ApprovalPolicy, CallId, ToolCallAction, ToolExecutionStatus,
    };
    use crate::core::workspace::Workspace;
    use crate::hooks::HookRegistry;
    use crate::memory::paths::MemoryPaths;
    use crate::tools::registry::ToolRegistry;

    #[tokio::test]
    async fn failed_tool_call_event_includes_execution_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let memory_paths = MemoryPaths::from_workspace(&workspace, 8);
        let registry = ToolRegistry::new();
        let ctx = ToolTurnContext {
            registry: &registry,
            workspace: &workspace,
            memory_paths: &memory_paths,
            approval_policy: ApprovalPolicy::Auto,
            approval_decision: ApprovalDecision::Approve,
            approval_provider: None,
            input_provider: None,
            hooks: HookRegistry::default(),
            cancel_token: CancellationToken::new(),
        };
        let action = ToolAction::Call(ToolCallAction {
            call_id: CallId::new(),
            tool_use_id: None,
            name: "missing_tool".to_string(),
            args: serde_json::json!({}),
        });

        let events = run_tool_turn(ctx, action).collect::<Vec<_>>().await;

        assert!(events.iter().any(|item| {
            matches!(
                item,
                ToolTurnItem::Event(crate::core::events::StreamEvent::ToolCallFailed {
                    metadata,
                    ..
                }) if metadata.status == ToolExecutionStatus::Error
                    && metadata.error_code.as_deref() == Some("unknown_tool")
            )
        }));
    }
}
