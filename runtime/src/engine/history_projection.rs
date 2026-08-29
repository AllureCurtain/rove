//! Derivation of model-visible history items from the engine event stream.
//!
//! Codex alignment Phase 2: instead of asking resume to heuristically
//! reclassify audit events, the engine derives every model-visible item once —
//! at the single choke point where durable trace lines are written — and
//! persists it explicitly as a `TraceEntry::History` line. The derivation
//! rules intentionally mirror `state::artifacts::RunArtifactRecorder` so the
//! trace history stream and the persisted snapshot stay reconcilable.
//!
//! Pure presentation/audit events yield no items and never reach a model
//! request, mirroring codex's `ResponseItem` vs `EventMsg` separation.

use std::collections::HashMap;

use rove_core::history::HistoryItem;
use rove_core::{CallId, ToolExecutionMetadata, ToolExecutionStatus};
use rove_models::{InternalCallId, Message, ToolCallRef, ToolResultStatus};

use crate::events::StreamEvent;

#[derive(Debug)]
struct PendingTool {
    tool_use_id: Option<String>,
    internal_call_id: InternalCallId,
    name: String,
}

/// Stateful projector over one run's event stream.
///
/// Events must be fed in emission order; the returned items are the exact
/// model-visible additions the corresponding events represent.
#[derive(Debug, Default)]
pub(crate) struct HistoryProjector {
    user_message_emitted: bool,
    pending_tools: HashMap<CallId, PendingTool>,
    pending_steers: HashMap<String, String>,
    pending_messages: HashMap<String, String>,
}

impl HistoryProjector {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Project one event into zero or more model-visible history items.
    pub(crate) fn project(&mut self, event: &StreamEvent) -> Vec<HistoryItem> {
        match event {
            StreamEvent::RunStarted { user_message, .. } => {
                if self.user_message_emitted {
                    return Vec::new();
                }
                self.user_message_emitted = true;
                vec![HistoryItem::Message(Message::user(user_message.clone()))]
            }
            StreamEvent::LlmMessage {
                full, tool_calls, ..
            } => {
                vec![HistoryItem::Message(assistant_message(full, tool_calls))]
            }
            // An accepted steer is not yet part of prompt history; only an
            // applied steer enters the model-visible conversation.
            StreamEvent::SteerAccepted { id, content } => {
                self.pending_steers.insert(id.clone(), content.clone());
                Vec::new()
            }
            StreamEvent::SteerApplied { id } => match self.pending_steers.remove(id) {
                Some(content) => vec![HistoryItem::Message(Message::user(content))],
                None => Vec::new(),
            },
            StreamEvent::MessageQueued { id, content } => {
                self.pending_messages.insert(id.clone(), content.clone());
                Vec::new()
            }
            StreamEvent::MessageAppliedCurrentRun { id } => {
                match self.pending_messages.remove(id) {
                    Some(content) => vec![HistoryItem::Message(Message::user(content))],
                    None => Vec::new(),
                }
            }
            StreamEvent::ToolCallStarted {
                call_id,
                tool_use_id,
                name,
                ..
            } => {
                self.pending_tools.insert(
                    *call_id,
                    PendingTool {
                        tool_use_id: tool_use_id.clone(),
                        internal_call_id: internal_call_id_for(call_id),
                        name: name.clone(),
                    },
                );
                Vec::new()
            }
            StreamEvent::ToolCallCompleted { call_id, result } => {
                let pending = self.pending_tools.remove(call_id);
                let message = Message::tool_with_status(
                    result.output.clone(),
                    pending.as_ref().and_then(|tool| tool.tool_use_id.clone()),
                    Some(
                        pending
                            .as_ref()
                            .map(|tool| tool.internal_call_id.clone())
                            .unwrap_or_else(|| internal_call_id_for(call_id)),
                    ),
                    pending.as_ref().map(|tool| tool.name.clone()),
                    canonical_status(&result.metadata.status),
                );
                vec![HistoryItem::Message(message)]
            }
            StreamEvent::ToolCallFailed {
                call_id,
                error,
                metadata,
            } => {
                let pending = self.pending_tools.remove(call_id);
                let message = Message::tool_with_status(
                    format!("Error: {error}"),
                    pending.as_ref().and_then(|tool| tool.tool_use_id.clone()),
                    Some(
                        pending
                            .as_ref()
                            .map(|tool| tool.internal_call_id.clone())
                            .unwrap_or_else(|| internal_call_id_for(call_id)),
                    ),
                    pending.as_ref().map(|tool| tool.name.clone()),
                    canonical_failure_status(metadata),
                );
                vec![HistoryItem::Message(message)]
            }
            _ => Vec::new(),
        }
    }
}

fn assistant_message(full: &str, tool_calls: &[ToolCallRef]) -> Message {
    if tool_calls.is_empty() {
        Message::assistant(full.to_string())
    } else {
        Message::assistant_with_tool_calls(full.to_string(), tool_calls.to_vec())
    }
}

fn internal_call_id_for(call_id: &CallId) -> InternalCallId {
    InternalCallId::new(call_id.to_string()).unwrap_or_else(|_| {
        InternalCallId::new(format!("runtime-call-{call_id}")).expect("runtime call id is bounded")
    })
}

fn canonical_status(status: &ToolExecutionStatus) -> ToolResultStatus {
    match status {
        ToolExecutionStatus::Ok => ToolResultStatus::Ok,
        ToolExecutionStatus::Rejected => ToolResultStatus::Rejected,
        ToolExecutionStatus::PartialSuccess => ToolResultStatus::Partial,
        ToolExecutionStatus::Error => ToolResultStatus::Error,
    }
}

fn canonical_failure_status(metadata: &ToolExecutionMetadata) -> ToolResultStatus {
    match metadata.status {
        ToolExecutionStatus::Rejected => ToolResultStatus::Rejected,
        ToolExecutionStatus::PartialSuccess => ToolResultStatus::Partial,
        _ => ToolResultStatus::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_core::ToolResult;
    use rove_models::{Role, Usage};

    fn llm_message_event(text: &str, calls: Vec<ToolCallRef>) -> StreamEvent {
        StreamEvent::LlmMessage {
            full: text.to_string(),
            usage: Usage::default(),
            tool_calls: calls,
            assistant_turn: None,
        }
    }

    fn ok_result(call_id: CallId) -> ToolResult {
        ToolResult {
            call_id,
            output: "file body".to_string(),
            mutations: Vec::new(),
            metadata: ToolExecutionMetadata::default(),
            envelope: None,
        }
    }

    /// The soul of the Phase 2 contract: replaying projected items must
    /// reproduce exactly the model-visible conversation the kernel held.
    #[test]
    fn projected_items_replay_into_the_kernel_conversation() {
        let mut projector = HistoryProjector::new();
        let started = StreamEvent::RunStarted {
            run_id: crate::types::RunId::new(),
            job_id: crate::types::JobId::new(),
            user_message: "fix the bug".to_string(),
        };
        // Presentation-only noise that must not leak into history.
        let chunk = StreamEvent::LlmChunk {
            delta: "thi".to_string(),
        };
        let assistant = llm_message_event(
            "on it",
            vec![ToolCallRef {
                id: "call_1".to_string(),
                name: "fs_read".to_string(),
                args: serde_json::json!({"path": "a.rs"}),
            }],
        );
        let tool_started = StreamEvent::ToolCallStarted {
            call_id: CallId::new(),
            tool_use_id: Some("call_1".to_string()),
            name: "fs_read".to_string(),
            args: serde_json::json!({"path": "a.rs"}),
        };
        let completed = CallId::new();
        let tool_completed = StreamEvent::ToolCallCompleted {
            call_id: completed,
            result: ok_result(completed),
        };
        let final_message = llm_message_event("done", Vec::new());

        let mut items = Vec::new();
        for event in [
            &started,
            &chunk,
            &assistant,
            &tool_started,
            &tool_completed,
            &final_message,
        ] {
            items.extend(projector.project(event));
        }

        let messages = rove_core::history::history_to_messages(&items);
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "fix the bug");
        assert_eq!(messages[1].content, "on it");
        assert_eq!(messages[1].tool_calls.len(), 1);
        assert_eq!(messages[2].role, Role::Tool);
        assert_eq!(
            messages[2].internal_call_id,
            Some(rove_models::InternalCallId::new(completed.to_string()).unwrap())
        );
        assert_eq!(messages[3].content, "done");
    }

    /// A steer enters history exactly when it is applied, not when accepted.
    #[test]
    fn steer_enters_history_only_when_applied() {
        let mut projector = HistoryProjector::new();
        assert!(
            projector
                .project(&StreamEvent::RunStarted {
                    run_id: crate::types::RunId::new(),
                    job_id: crate::types::JobId::new(),
                    user_message: "goal".to_string(),
                })
                .len()
                == 1
        );
        assert!(
            projector
                .project(&StreamEvent::SteerAccepted {
                    id: "s-1".to_string(),
                    content: "also add tests".to_string(),
                })
                .is_empty()
        );
        let applied = projector.project(&StreamEvent::SteerApplied {
            id: "s-1".to_string(),
        });
        assert_eq!(applied.len(), 1);

        let messages = rove_core::history::history_to_messages(&applied);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[0].content, "also add tests");
    }

    /// Failed tool calls still produce a canonical tool message so a resumed
    /// provider conversation stays structurally valid.
    #[test]
    fn failed_tool_calls_project_canonical_tool_messages() {
        let mut projector = HistoryProjector::new();
        let call = CallId::new();
        assert!(
            projector
                .project(&StreamEvent::ToolCallStarted {
                    call_id: call,
                    tool_use_id: None,
                    name: "shell".to_string(),
                    args: serde_json::json!({}),
                })
                .is_empty()
        );
        let failed = StreamEvent::ToolCallFailed {
            call_id: call,
            error: rove_core::ToolError::InvalidArgs {
                reason: "timeout".to_string(),
            },
            metadata: ToolExecutionMetadata::default(),
        };
        let messages = rove_core::history::history_to_messages(&projector.project(&failed));
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::Tool);
        assert!(messages[0].content.contains("timeout"));
        assert_eq!(
            messages[0].tool_result_status,
            Some(rove_models::ToolResultStatus::Error)
        );
    }

    /// Only the first run-start user message becomes history; duplicate
    /// lifecycle replays stay idempotent.
    #[test]
    fn run_start_is_idempotent() {
        let mut projector = HistoryProjector::new();
        let event = StreamEvent::RunStarted {
            run_id: crate::types::RunId::new(),
            job_id: crate::types::JobId::new(),
            user_message: "once".to_string(),
        };
        assert_eq!(projector.project(&event).len(), 1);
        assert!(projector.project(&event).is_empty());
    }
}
