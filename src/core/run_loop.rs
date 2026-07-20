use std::sync::Arc;

use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::compaction::{CompactionRuntime, maybe_compact_history};
use crate::core::context::ContextManager;
use crate::core::events::StreamEvent;
use crate::core::model_turn::{ModelTurnItem, run_model_turn};
use crate::core::prompt_metadata::{
    PromptBuildMetadata, prompt_cache_key, tool_signature, workspace_fingerprint,
};
use crate::core::tool_turn::{
    ToolAction, ToolTurnContext, ToolTurnItem, append_tool_history, run_tool_turn,
};
use crate::core::types::{
    Action, ApprovalDecision, ApprovalPolicy, Message, SessionId, TerminationReason,
    ToolApprovalProvider, ToolSchema, UserInputProvider,
};
use crate::core::workspace::Workspace;
use crate::hooks::HookRegistry;
use crate::memory::paths::MemoryPaths;
use crate::memory::session::append_session_notes_to_dir_sync;
use crate::models::traits::ModelClient;
use crate::tools::registry::ToolRegistry;

#[derive(Clone)]
pub(crate) struct LoopContext<'a> {
    pub model: &'a dyn ModelClient,
    pub registry: &'a ToolRegistry,
    pub context_manager: &'a ContextManager,
    pub workspace: &'a Workspace,
    pub memory_paths: &'a MemoryPaths,
    pub session_id: SessionId,
    pub max_steps: u32,
    pub max_model_turns_per_step: u32,
    pub approval_policy: ApprovalPolicy,
    pub approval_decision: ApprovalDecision,
    pub approval_provider: Option<Arc<dyn ToolApprovalProvider>>,
    pub input_provider: Option<Arc<dyn UserInputProvider>>,
    pub hooks: HookRegistry,
    pub compaction: CompactionRuntime,
}

impl<'a> LoopContext<'a> {
    pub(crate) fn tool_turn_context(&self, cancel_token: CancellationToken) -> ToolTurnContext<'a> {
        ToolTurnContext {
            registry: self.registry,
            workspace: self.workspace,
            memory_paths: self.memory_paths,
            approval_policy: self.approval_policy,
            approval_decision: self.approval_decision,
            approval_provider: self.approval_provider.clone(),
            input_provider: self.input_provider.clone(),
            hooks: self.hooks.clone(),
            cancel_token,
        }
    }
}

pub(crate) fn enrich_prompt_metadata(
    ctx: &LoopContext<'_>,
    mut metadata: PromptBuildMetadata,
    tools: &[ToolSchema],
) -> PromptBuildMetadata {
    metadata.workspace_fingerprint = workspace_fingerprint(ctx.workspace);
    metadata.tool_signature = tool_signature(tools);
    metadata.prompt_cache_key = Some(prompt_cache_key(
        &metadata.stable_prefix_hash,
        &metadata.tool_signature,
    ));
    metadata
}

/// Extract durable-worthy notes from messages that are about to be compacted.
///
/// Looks for tool results that report file modifications, and assistant
/// messages that state decisions or plans, returning a deduplicated list of
/// short notes suitable for the session-summary flush.
pub(crate) fn extract_session_memory_notes(messages: &[Message]) -> Vec<String> {
    let mut notes = Vec::new();

    for msg in messages {
        let content = msg.content.trim();
        if content.is_empty() {
            continue;
        }
        // Tool results that mention file modifications.
        if msg.role == crate::core::types::Role::Tool {
            let lower = content.to_ascii_lowercase();
            if lower.contains("created")
                || lower.contains("wrote")
                || lower.contains("modified")
                || lower.contains("saved")
            {
                let snippet: String = content.chars().take(160).collect();
                notes.push(format!("tool result: {snippet}"));
            }
        }
        // Assistant messages that state decisions or intent.
        if msg.role == crate::core::types::Role::Assistant {
            let lower = content.to_ascii_lowercase();
            if lower.contains("i decided")
                || lower.contains("i will")
                || lower.contains("decision:")
                || lower.contains("approach:")
                || lower.contains("plan:")
            {
                let snippet: String = content.chars().take(200).collect();
                notes.push(format!("assistant note: {snippet}"));
            }
        }
    }

    notes.sort();
    notes.dedup();
    notes
}

pub(crate) struct RunLoopState {
    pub user_message: String,
    pub working_memory: Vec<Message>,
    pub compact_summary: Option<String>,
    pub history: Vec<Message>,
    pub step: u32,
}

#[derive(Debug)]
pub(crate) enum LoopItem {
    Event(StreamEvent),
    Complete {
        reason: TerminationReason,
        output: Option<String>,
    },
}

pub(crate) fn run_unplanned_loop<'a>(
    ctx: LoopContext<'a>,
    mut state: RunLoopState,
    cancel_token: CancellationToken,
) -> BoxStream<'a, LoopItem> {
    Box::pin(stream! {
        let mut compaction = ctx.compaction.clone();
        loop {
            if cancel_token.is_cancelled() {
                yield LoopItem::Complete {
                    reason: TerminationReason::Cancelled,
                    output: None,
                };
                return;
            }

            if state.step >= ctx.max_steps {
                yield LoopItem::Complete {
                    reason: TerminationReason::StepLimit,
                    output: None,
                };
                return;
            }
            state.step += 1;

            let context = ctx.context_manager.build_with_checkpoint(
                &state.user_message,
                &state.working_memory,
                state.compact_summary.as_deref(),
                &state.history,
            );
            let tool_schemas = ctx.registry.schemas();
            yield LoopItem::Event(StreamEvent::PromptBuilt {
                metadata: enrich_prompt_metadata(&ctx, context.metadata.clone(), &tool_schemas),
            });
            if context.over_hard_limit {
                yield LoopItem::Complete {
                    reason: TerminationReason::TokenLimit,
                    output: Some("context exceeds configured hard token budget".to_string()),
                };
                return;
            }
            if context.auto_compaction_needed && context.dropped_history_messages > 0 {
                let compacted_count = context.dropped_history_messages.min(state.history.len());

                // Pre-compaction flush: extract durable-worthy notes from the
                // messages about to be compacted and persist them to session
                // memory before the detail is summarized away.
                let mut flush_notes = Vec::new();
                if compacted_count > 0 {
                    let candidate_notes = extract_session_memory_notes(&state.history[..compacted_count]);
                    if !candidate_notes.is_empty()
                        && append_session_notes_to_dir_sync(
                            &ctx.memory_paths.session_dir,
                            ctx.session_id,
                            &candidate_notes,
                        )
                        .is_ok()
                    {
                        flush_notes = candidate_notes;
                        yield LoopItem::Event(StreamEvent::MemoryFlushed {
                            notes: flush_notes.clone(),
                        });
                    }
                }

                if let Some(update) = maybe_compact_history(
                    &mut compaction,
                    ctx.model,
                    &state.history[..compacted_count],
                    flush_notes,
                    cancel_token.clone(),
                )
                .await
                {
                    let summary_for_event = update.summary.clone();
                    if let Some(summary) = update.summary {
                        state.compact_summary = Some(summary);
                    }
                    yield LoopItem::Event(StreamEvent::PromptCompacted {
                        summary: summary_for_event,
                        state: update.state,
                    });
                }
            }

            let mut turn_stream = run_model_turn(
                ctx.model,
                context.messages,
                tool_schemas,
                cancel_token.clone(),
            );
            let model_turn = loop {
                match turn_stream.next().await {
                    Some(ModelTurnItem::Event(event)) => yield LoopItem::Event(event),
                    Some(ModelTurnItem::Finished(turn)) => break turn,
                    Some(ModelTurnItem::Cancelled) => {
                        yield LoopItem::Complete {
                            reason: TerminationReason::Cancelled,
                            output: None,
                        };
                        return;
                    }
                    Some(ModelTurnItem::Failed(err)) => {
                        yield LoopItem::Complete {
                            reason: TerminationReason::Error,
                            output: Some(format!("Model error: {err}")),
                        };
                        return;
                    }
                    None => {
                        yield LoopItem::Complete {
                            reason: TerminationReason::Error,
                            output: Some("model turn ended without a response".to_string()),
                        };
                        return;
                    }
                }
            };

            match model_turn.action {
                Action::Final { text } => {
                    yield LoopItem::Complete {
                        reason: TerminationReason::Final,
                        output: Some(text),
                    };
                    return;
                }
                Action::ToolCall {
                    call_id,
                    tool_use_id,
                    name,
                    args,
                } => {
                    let action = ToolAction::Call(crate::core::types::ToolCallAction {
                        call_id,
                        tool_use_id,
                        name,
                        args,
                    });
                    let mut tool_stream = run_tool_turn(
                        ctx.tool_turn_context(cancel_token.clone()),
                        action,
                    );
                    let outcome = loop {
                        match tool_stream.next().await {
                            Some(ToolTurnItem::Event(event)) => yield LoopItem::Event(event),
                            Some(ToolTurnItem::Finished(outcome)) => break outcome,
                            Some(ToolTurnItem::Cancelled) => {
                                drop(tool_stream);
                                yield LoopItem::Complete {
                                    reason: TerminationReason::Cancelled,
                                    output: None,
                                };
                                return;
                            }
                            None => {
                                yield LoopItem::Complete {
                                    reason: TerminationReason::Error,
                                    output: Some("tool turn ended without a result".to_string()),
                                };
                                return;
                            }
                        }
                    };
                    append_tool_history(&mut state.history, &model_turn.full_response, &outcome);
                }
                Action::ToolBatch { calls } => {
                    let mut tool_stream = run_tool_turn(
                        ctx.tool_turn_context(cancel_token.clone()),
                        ToolAction::Batch(calls),
                    );
                    let outcome = loop {
                        match tool_stream.next().await {
                            Some(ToolTurnItem::Event(event)) => yield LoopItem::Event(event),
                            Some(ToolTurnItem::Finished(outcome)) => break outcome,
                            Some(ToolTurnItem::Cancelled) => {
                                drop(tool_stream);
                                yield LoopItem::Complete {
                                    reason: TerminationReason::Cancelled,
                                    output: None,
                                };
                                return;
                            }
                            None => {
                                yield LoopItem::Complete {
                                    reason: TerminationReason::Error,
                                    output: Some("tool turn ended without a result".to_string()),
                                };
                                return;
                            }
                        }
                    };
                    append_tool_history(&mut state.history, &model_turn.full_response, &outcome);
                }
                Action::Malformed { reason } => {
                    state.history.push(Message::assistant(model_turn.full_response));
                    state.history.push(Message::user(format!(
                        "Your previous output could not be parsed: {}. Please try again.",
                        reason
                    )));
                }
            }
        }
    })
}
