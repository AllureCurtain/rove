use std::sync::Arc;

use async_stream::stream;
use futures::future::join_all;
use futures::stream::{BoxStream, StreamExt};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agents::AgentRuntimeProfile;
use crate::agents::instructions::normalize_workspace_target;
use crate::environment::ExecutionEnvironment;
use crate::events::StreamEvent;
use crate::executor::Executor;
use crate::hooks::HookRegistry;
use crate::memory::paths::MemoryPaths;
use crate::state::tool_artifacts::{ArtifactLedgerEntry, ToolArtifactStore};
use crate::tool_input::RegisteredUserInput;
use crate::tools::runtime_context::runtime_tool_context_with_mode_and_artifacts;
use crate::types::{
    ApprovalDecision, ApprovalPolicy, CallId, Message, PendingToolApproval, ToolApprovalProvider,
    ToolApprovalRequest, ToolCallAction, ToolCallRef, ToolExecutionMetadata, ToolExecutionStatus,
    ToolResult, ToolRiskLevel, UserInputProvider,
};
use crate::workspace::Workspace;
use rove_core::ToolError;
use rove_core::ToolRegistry;
use rove_models::{InternalCallId, ToolResultStatus};

const APPROVAL_REASON: &str = "destructive tool requires explicit approval";

fn event_args(mode: crate::types::RunMode, args: &serde_json::Value) -> serde_json::Value {
    if matches!(mode, crate::types::RunMode::Review) {
        serde_json::json!({"redacted": true})
    } else {
        args.clone()
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ToolAction {
    Call(ToolCallAction),
    Batch(Vec<ToolCallAction>),
}

impl ToolAction {
    pub(crate) fn calls(&self) -> Vec<&ToolCallAction> {
        match self {
            Self::Call(call) => vec![call],
            Self::Batch(calls) => calls.iter().collect(),
        }
    }

    fn into_calls(self) -> Vec<ToolCallAction> {
        match self {
            Self::Call(call) => vec![call],
            Self::Batch(calls) => calls,
        }
    }
}

/// Concrete local-workspace targets declared by first-party structured tools.
///
/// This is context routing only; each tool still performs its authoritative
/// argument and workspace-boundary validation. MCP paths are deliberately not
/// interpreted as local paths because remote resource names have no local
/// filesystem authority.
pub(crate) fn workspace_target_paths(action: &ToolAction) -> Vec<String> {
    const MAX_TARGET_PATHS: usize = 64;
    let mut paths = Vec::new();
    for call in action.calls() {
        let fields: &[&str] = match call.name.as_str() {
            "read_file" | "write_file" | "edit_file" | "delete_path" | "list_directory"
            | "glob_paths" | "search_code" => &["path"],
            "move_path" => &["from", "to"],
            "workspace_checkpoint" | "workspace_diff" | "workspace_rewind" | "run_shell" => {
                &["paths"]
            }
            _ => &[],
        };
        for field in fields {
            let Some(value) = call.args.get(*field) else {
                continue;
            };
            match value {
                serde_json::Value::String(path) => push_target_path(&mut paths, path),
                serde_json::Value::Array(values) => {
                    for path in values.iter().filter_map(serde_json::Value::as_str) {
                        push_target_path(&mut paths, path);
                        if paths.len() >= MAX_TARGET_PATHS {
                            break;
                        }
                    }
                }
                _ => {}
            }
            if paths.len() >= MAX_TARGET_PATHS {
                break;
            }
        }
        if paths.len() >= MAX_TARGET_PATHS {
            break;
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn push_target_path(paths: &mut Vec<String>, raw: &str) {
    if let Some(path) = normalize_workspace_target(raw) {
        paths.push(path);
    }
}

/// Close a model-requested tool call without dispatching it. This preserves
/// provider tool-call/result correlation and durable session integrity while a
/// Runtime-owned precondition is installed for the next model turn.
pub(crate) fn defer_tool_turn(
    action: ToolAction,
    reason: String,
) -> BoxStream<'static, ToolTurnItem> {
    Box::pin(stream! {
        let mut records = Vec::new();
        for call in action.into_calls() {
            yield ToolTurnItem::Event(StreamEvent::ToolCallStarted {
                call_id: call.call_id,
                tool_use_id: call.tool_use_id.clone(),
                name: call.name.clone(),
                args: call.args.clone(),
            });
            let error = ToolError::PreconditionRequired {
                reason: reason.clone(),
            };
            let metadata = failure_metadata(&error);
            records.push(ToolExecutionRecord {
                call: call.clone(),
                history_output: format!("Error: {error}"),
                error_reason: Some(error.to_string()),
                artifacts: Vec::new(),
            });
            yield ToolTurnItem::Event(StreamEvent::ToolCallFailed {
                call_id: call.call_id,
                error,
                metadata,
            });
        }
        yield ToolTurnItem::Finished(ToolTurnOutcome { records });
    })
}

#[derive(Clone)]
pub(crate) struct ToolTurnContext<'a> {
    pub registry: &'a ToolRegistry,
    pub workspace: &'a Workspace,
    pub environment: Arc<dyn ExecutionEnvironment>,
    pub memory_paths: &'a MemoryPaths,
    pub approval_policy: ApprovalPolicy,
    pub approval_decision: ApprovalDecision,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub hooks: HookRegistry,
    pub cancel_token: CancellationToken,
    /// Durable artifact authority for this run, passed to every tool so a
    /// rich result can retain its payloads.
    pub tool_artifacts: Option<Arc<ToolArtifactStore>>,
    pub agent_profile: Option<Arc<AgentRuntimeProfile>>,
    pub run_mode: crate::types::RunMode,
}

#[derive(Debug)]
pub(crate) struct ToolExecutionRecord {
    pub call: ToolCallAction,
    pub history_output: String,
    pub error_reason: Option<String>,
    pub artifacts: Vec<rove_core::ToolArtifactRef>,
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
                .descriptor(tool_name)
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
                .descriptor(tool_name)
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
            // Dynamic input requests need the serial event/ack path. Tool
            // schemas cannot currently declare whether execution may ask.
            && self.input_provider.is_none()
            && calls
                .iter()
                .all(|call| self.tool_is_parallel_safe(&call.name))
    }

    fn tool_is_parallel_safe(&self, tool_name: &str) -> bool {
        let Ok(schema) = self.registry.descriptor(tool_name) else {
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
        if let Err(error) = self.check_agent_capability(&call.name) {
            return ToolExecution {
                call,
                result: Err(error),
            };
        }
        let executor = Executor::with_hooks(self.registry, self.hooks.clone());
        let tool_context = runtime_tool_context_with_mode_and_artifacts(
            call.call_id,
            self.workspace,
            self.memory_paths.clone(),
            self.effective_approval_policy(&call.name, approval_decision),
            self.input_provider.clone(),
            self.cancel_token.clone(),
            self.environment.clone(),
            self.tool_artifacts.clone(),
            self.run_mode,
        );
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

    fn check_agent_capability(&self, tool_name: &str) -> Result<(), ToolError> {
        let Some(profile) = self.agent_profile.as_ref() else {
            return Ok(());
        };
        let descriptor = self.registry.descriptor(tool_name)?;
        match descriptor.capability_id.as_deref() {
            Some(capability) if profile.effective_capabilities.contains(capability) => Ok(()),
            Some(capability) => Err(ToolError::PermissionDenied {
                reason: format!("active Agent profile does not permit capability '{capability}'"),
            }),
            None if profile.is_legacy() => Ok(()),
            None => Err(ToolError::PermissionDenied {
                reason: "active Agent profile refuses tools without a capability identity"
                    .to_string(),
            }),
        }
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
                    args: event_args(ctx.run_mode, &call.args),
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
                        yield ToolTurnItem::Event(StreamEvent::ModelStatus {
                            status: "waiting_for_approval".to_string(),
                            message: "Waiting for tool approval".to_string(),
                        });
                        yield ToolTurnItem::Event(StreamEvent::ToolCallApprovalNeeded {
                            call_id: call.call_id,
                            name: call.name.clone(),
                            args: event_args(ctx.run_mode, &call.args),
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
                            args: event_args(ctx.run_mode, &call.args),
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
                            args: event_args(ctx.run_mode, &call.args),
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
                                yield ToolTurnItem::Event(StreamEvent::ModelStatus {
                                    status: "waiting_for_approval".to_string(),
                                    message: "Waiting for tool approval".to_string(),
                                });
                                yield ToolTurnItem::Event(StreamEvent::ToolCallApprovalNeeded {
                                    call_id: call.call_id,
                                    name: call.name.clone(),
                                    args: event_args(ctx.run_mode, &call.args),
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
                    let mut persisted_call = execution.call.clone();
                    persisted_call.args = event_args(ctx.run_mode, &persisted_call.args);
                    records.push(ToolExecutionRecord {
                        call: persisted_call,
                        history_output: result.output.clone(),
                        error_reason: None,
                        artifacts: result
                            .envelope
                            .as_ref()
                            .map(|envelope| envelope.artifacts.clone())
                            .unwrap_or_default(),
                    });
                    if let Some(envelope) = result.envelope.as_ref() {
                        for artifact in &envelope.artifacts {
                            yield ToolTurnItem::Event(StreamEvent::ToolArtifactStored {
                                call_id: execution.call.call_id,
                                artifact: Box::new(artifact.clone()),
                            });
                        }
                    }
                    if let Some(store) = ctx.tool_artifacts.as_ref()
                        && let Ok(entries) = store.ledger().await
                    {
                        let call_id = execution.call.call_id.to_string();
                        for entry in entries {
                            if let ArtifactLedgerEntry::Rejected {
                                call_id: rejected_call_id,
                                block_ordinal,
                                reason,
                                observed_bytes,
                                ..
                            } = entry
                                && rejected_call_id == call_id
                            {
                                yield ToolTurnItem::Event(StreamEvent::ToolArtifactRejected {
                                    call_id: execution.call.call_id,
                                    block_ordinal,
                                    reason: reason.code().to_string(),
                                    observed_bytes,
                                });
                            }
                        }
                    }
                    yield ToolTurnItem::Event(StreamEvent::ToolCallCompleted {
                        call_id: execution.call.call_id,
                        result,
                    });
                }
                Err(error) => {
                    let metadata = failure_metadata(&error);
                    let reason = error.to_string();
                    let mut persisted_call = execution.call.clone();
                    persisted_call.args = event_args(ctx.run_mode, &persisted_call.args);
                    records.push(ToolExecutionRecord {
                        call: persisted_call,
                        history_output: format!("Error: {reason}"),
                        error_reason: Some(reason),
                        artifacts: Vec::new(),
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
    let deferred = matches!(error, ToolError::PreconditionRequired { .. });
    ToolExecutionMetadata {
        status: if deferred {
            ToolExecutionStatus::Rejected
        } else {
            ToolExecutionStatus::Error
        },
        error_code: Some(error.error_code().to_string()),
        security_event_type: security_event_type(error),
        risk_level: if deferred {
            ToolRiskLevel::Low
        } else {
            ToolRiskLevel::High
        },
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
        let internal_call_id = record
            .call
            .tool_use_id
            .as_deref()
            .and_then(|id| InternalCallId::new(id.to_string()).ok())
            .unwrap_or_else(|| {
                InternalCallId::new(record.call.call_id.to_string())
                    .expect("runtime call id is bounded")
            });
        let status = if record.error_reason.is_some() {
            ToolResultStatus::Error
        } else {
            ToolResultStatus::Ok
        };
        let mut message = Message::tool_with_status(
            record.history_output.clone(),
            record.call.tool_use_id.clone(),
            Some(internal_call_id),
            Some(record.call.name.clone()),
            status,
        );
        if !record.artifacts.is_empty() {
            message.content_blocks.push(rove_models::ContentBlock::text(
                record.history_output.clone(),
            ));
            message
                .content_blocks
                .extend(record.artifacts.iter().map(|artifact| {
                    rove_models::ContentBlock::RichReference {
                        kind: "tool_artifact".to_string(),
                        reference: artifact.artifact_id.to_string(),
                        mime_type: artifact.mime_type.clone(),
                        title: Some(format!(
                            "{} bytes sha256:{}",
                            artifact.byte_length, artifact.sha256
                        )),
                    }
                }));
        }
        history.push(message);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use futures::StreamExt;
    use tokio_util::sync::CancellationToken;

    use super::{
        ToolAction, ToolTurnContext, ToolTurnItem, defer_tool_turn, run_tool_turn,
        workspace_target_paths,
    };
    use crate::environment::local_environment;
    use crate::events::StreamEvent;
    use crate::hooks::HookRegistry;
    use crate::memory::paths::MemoryPaths;
    use crate::state::tool_artifacts::{ArtifactClaim, ToolArtifactStore};
    use crate::tools::echo::EchoTool;
    use crate::types::{
        ApprovalDecision, ApprovalPolicy, CallId, PendingUserInput, ToolCallAction,
        ToolExecutionStatus, UserInputProvider, UserInputRequest,
    };
    use crate::workspace::Workspace;
    use rove_core::{
        ArtifactTrust, Sensitivity, Tool, ToolArtifactKind, ToolArtifactSource, ToolContext,
        ToolDescriptor, ToolError, ToolOutput, ToolOutputEnvelope, ToolRegistry,
    };

    struct ImmediateInputProvider;

    struct ArtifactFixtureTool {
        store: Arc<ToolArtifactStore>,
    }

    #[async_trait]
    impl Tool for ArtifactFixtureTool {
        fn schema(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "artifact_fixture".to_string(),
                description: "Emit one artifact and one quota rejection".to_string(),
                parameters: serde_json::json!({"type":"object"}),
                destructive: false,
                parallel_safe: false,
                capability_id: None,
                capability: None,
            }
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            context: &ToolContext<'_>,
        ) -> Result<ToolOutput, ToolError> {
            let source = |block_ordinal| ToolArtifactSource {
                run_id: self.store.run_id(),
                call_id: context.call_id.to_string(),
                block_ordinal,
                captured_at: "2026-08-10T00:00:00Z".to_string(),
                ..ToolArtifactSource::default()
            };
            let artifact = self
                .store
                .put(
                    ToolArtifactKind::Resource,
                    b"retained",
                    source(0),
                    ArtifactClaim::default(),
                    Sensitivity::Normal,
                    ArtifactTrust::Untrusted,
                )
                .await
                .unwrap();
            let _ = self
                .store
                .put(
                    ToolArtifactKind::Resource,
                    b"",
                    source(1),
                    ArtifactClaim::default(),
                    Sensitivity::Normal,
                    ArtifactTrust::Untrusted,
                )
                .await;
            Ok(ToolOutput::from_envelope(ToolOutputEnvelope {
                summary_text: "artifact result".to_string(),
                artifacts: vec![artifact],
                ..ToolOutputEnvelope::default()
            }))
        }
    }

    #[async_trait]
    impl UserInputProvider for ImmediateInputProvider {
        async fn begin_input(
            &self,
            _input_id: CallId,
            _request: UserInputRequest,
        ) -> Result<PendingUserInput, ToolError> {
            Ok(PendingUserInput::new(async { Ok("answer".to_string()) }))
        }
    }

    #[test]
    fn interactive_input_provider_forces_batches_through_the_serial_event_path() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let memory_paths = MemoryPaths::from_workspace(&workspace, 8);
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));
        let calls = vec![
            ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: None,
                name: "echo".to_string(),
                args: serde_json::json!({"message":"one"}),
            },
            ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: None,
                name: "echo".to_string(),
                args: serde_json::json!({"message":"two"}),
            },
        ];
        let base = ToolTurnContext {
            registry: &registry,
            workspace: &workspace,
            environment: local_environment(&workspace),
            memory_paths: &memory_paths,
            approval_policy: ApprovalPolicy::Auto,
            approval_decision: ApprovalDecision::Approve,
            approval_provider: None,
            input_provider: None,
            hooks: HookRegistry::default(),
            cancel_token: CancellationToken::new(),
            tool_artifacts: None,
            agent_profile: None,
            run_mode: crate::types::RunMode::Normal,
        };

        assert!(base.can_run_parallel_batch(&calls));
        assert!(
            !ToolTurnContext {
                input_provider: Some(Arc::new(ImmediateInputProvider)),
                ..base
            }
            .can_run_parallel_batch(&calls)
        );
    }

    #[tokio::test]
    async fn failed_tool_call_event_includes_execution_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let memory_paths = MemoryPaths::from_workspace(&workspace, 8);
        let registry = ToolRegistry::new();
        let ctx = ToolTurnContext {
            registry: &registry,
            workspace: &workspace,
            environment: local_environment(&workspace),
            memory_paths: &memory_paths,
            approval_policy: ApprovalPolicy::Auto,
            approval_decision: ApprovalDecision::Approve,
            approval_provider: None,
            input_provider: None,
            hooks: HookRegistry::default(),
            cancel_token: CancellationToken::new(),
            tool_artifacts: None,
            agent_profile: None,
            run_mode: crate::types::RunMode::Normal,
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
                ToolTurnItem::Event(crate::events::StreamEvent::ToolCallFailed {
                    metadata,
                    ..
                }) if metadata.status == ToolExecutionStatus::Error
                    && metadata.error_code.as_deref() == Some("unknown_tool")
            )
        }));
    }

    #[tokio::test]
    async fn artifact_events_precede_the_completed_tool_event_without_payload_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(tmp.path()).unwrap();
        let memory_paths = MemoryPaths::from_workspace(&workspace, 8);
        let store = Arc::new(ToolArtifactStore::new(
            tmp.path().join("runs").join("run-artifact-events"),
        ));
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(ArtifactFixtureTool {
            store: Arc::clone(&store),
        }));
        let call_id = CallId::new();
        let ctx = ToolTurnContext {
            registry: &registry,
            workspace: &workspace,
            environment: local_environment(&workspace),
            memory_paths: &memory_paths,
            approval_policy: ApprovalPolicy::Auto,
            approval_decision: ApprovalDecision::Approve,
            approval_provider: None,
            input_provider: None,
            hooks: HookRegistry::default(),
            cancel_token: CancellationToken::new(),
            tool_artifacts: Some(Arc::clone(&store)),
            agent_profile: None,
            run_mode: crate::types::RunMode::Normal,
        };
        let items = run_tool_turn(
            ctx,
            ToolAction::Call(ToolCallAction {
                call_id,
                tool_use_id: None,
                name: "artifact_fixture".to_string(),
                args: serde_json::json!({}),
            }),
        )
        .collect::<Vec<_>>()
        .await;

        let stored = items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ToolTurnItem::Event(StreamEvent::ToolArtifactStored { call_id: id, artifact })
                        if *id == call_id && artifact.byte_length == 8
                )
            })
            .unwrap();
        let rejected = items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ToolTurnItem::Event(StreamEvent::ToolArtifactRejected {
                        call_id: id,
                        block_ordinal: 1,
                        reason,
                        observed_bytes: 0,
                    }) if *id == call_id && reason == "artifact_empty_payload"
                )
            })
            .unwrap();
        let completed = items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ToolTurnItem::Event(StreamEvent::ToolCallCompleted { call_id: id, .. })
                        if *id == call_id
                )
            })
            .unwrap();
        assert!(stored < rejected && rejected < completed);
        assert!(
            !serde_json::to_string(
                &items
                    .iter()
                    .filter_map(|item| match item {
                        ToolTurnItem::Event(event) => Some(event),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            )
            .unwrap()
            .contains("cmV0YWluZWQ=")
        );
    }

    #[test]
    fn structured_workspace_targets_are_normalized_and_bounded() {
        let action = ToolAction::Batch(vec![
            ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: None,
                name: "move_path".to_string(),
                args: serde_json::json!({
                    "from": ".\\apps\\web\\old.ts",
                    "to": "apps/web/new.ts"
                }),
            },
            ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: None,
                name: "read_file".to_string(),
                args: serde_json::json!({"path":"../outside"}),
            },
        ]);

        assert_eq!(
            workspace_target_paths(&action),
            vec!["apps/web/new.ts", "apps/web/old.ts"]
        );
    }

    #[tokio::test]
    async fn a_deferred_call_closes_correlation_without_dispatching_a_tool() {
        let call_id = CallId::new();
        let items = defer_tool_turn(
            ToolAction::Call(ToolCallAction {
                call_id,
                tool_use_id: Some("toolu-overlay".to_string()),
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"apps/web/page.tsx","content":"x"}),
            }),
            "scoped instructions activated".to_string(),
        )
        .collect::<Vec<_>>()
        .await;

        assert!(matches!(
            &items[0],
            ToolTurnItem::Event(StreamEvent::ToolCallStarted { call_id: id, .. }) if *id == call_id
        ));
        assert!(matches!(
            &items[1],
            ToolTurnItem::Event(StreamEvent::ToolCallFailed { error, metadata, .. })
                if error.error_code() == "precondition_required"
                    && metadata.status == ToolExecutionStatus::Rejected
        ));
        assert!(matches!(
            &items[2],
            ToolTurnItem::Finished(outcome)
                if outcome.records.len() == 1
                    && outcome.records[0].error_reason.is_some()
        ));
    }
}
