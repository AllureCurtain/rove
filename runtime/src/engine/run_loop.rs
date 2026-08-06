use std::sync::Arc;

use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::compaction::{CompactionRuntime, maybe_compact_history};
use crate::context::ContextManager;
use crate::engine::control::{SteerLifecycle, SteerMessage};
use crate::events::StreamEvent;
use crate::hooks::HookRegistry;
use crate::memory::paths::MemoryPaths;
use crate::memory::session::append_session_notes_to_dir_sync;
use crate::model_turn::{ModelTurnItem, run_model_turn};
use crate::prompt_metadata::{
    PromptBuildMetadata, prompt_cache_key, tool_signature, workspace_fingerprint,
};
use crate::tool_turn::{
    ToolAction, ToolTurnContext, ToolTurnItem, append_tool_history, run_tool_turn,
};
use crate::types::{
    Action, ApprovalDecision, ApprovalPolicy, Message, SessionId, TerminationReason,
    ToolApprovalProvider, ToolDescriptor, UserInputProvider,
};
use crate::workspace::Workspace;
use rove_core::ToolRegistry;
use rove_models::ModelClient;
use tokio::sync::Mutex as AsyncMutex;

/// Shared receiver for in-flight steer messages. Wrapped in Arc<AsyncMutex> so
/// it can be cloned into LoopContext and polled at each safe point without
/// holding a mutable borrow across await boundaries.
pub(crate) type SteerReceiver = Arc<AsyncMutex<tokio::sync::mpsc::Receiver<SteerMessage>>>;

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
    /// Inbound steer messages drained at the declared safe point (top of each
    /// loop iteration, BEFORE prompt construction for the next model turn).
    /// `None` for runs without a control plane (e.g. direct CLI exec).
    pub steer_rx: Option<SteerReceiver>,
    /// Tracks steers after the safe point and until the next model turn is
    /// actually handed to the model runner.
    pub steer_lifecycle: Option<SteerLifecycle>,
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
    tools: &[ToolDescriptor],
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
        if msg.role == crate::types::Role::Tool {
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
        if msg.role == crate::types::Role::Assistant {
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
                // Drain and surface dropped steers so the API/product store
                // can reconcile them as never-applied.
                if let Some(rx) = ctx.steer_rx.as_ref() {
                    let mut r = rx.lock().await;
                    while let Ok(msg) = r.try_recv() {
                        yield LoopItem::Event(StreamEvent::SteerDropped {
                            id: msg.id.0,
                            reason: "cancelled".to_string(),
                        });
                    }
                }
                yield LoopItem::Complete {
                    reason: TerminationReason::Cancelled,
                    output: None,
                };
                return;
            }

            // SAFE POINT — drain any steers queued since the last turn. Runs
            // *before* prompt assembly for the next model call so the injected
            // message is visible on the next turn. Never runs mid-turn.
            let mut accepted_steer_ids = Vec::new();
            if let Some(rx) = ctx.steer_rx.as_ref() {
                let mut r = rx.lock().await;
                while let Ok(msg) = r.try_recv() {
                    let id = msg.id.0;
                    state.working_memory.push(Message::user(msg.content.clone()));
                    if let Some(lifecycle) = ctx.steer_lifecycle.as_ref() {
                        lifecycle.accepted(id.clone()).await;
                    }
                    yield LoopItem::Event(StreamEvent::SteerAccepted {
                        id: id.clone(),
                        content: msg.content,
                    });
                    accepted_steer_ids.push(id);
                }
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
            let tool_schemas = ctx.registry.descriptors();
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
                ctx.registry.model_schemas(),
                cancel_token.clone(),
            );
            let mut steers_applied = false;
            let model_turn = loop {
                match turn_stream.next().await {
                    Some(ModelTurnItem::Event(event)) => {
                        // `run_model_turn` has now been successfully polled and
                        // constructed its provider stream. This is the earliest
                        // durable lifecycle boundary at which a steer can claim
                        // to have entered a model turn rather than merely a
                        // prepared prompt.
                        if !steers_applied {
                            for id in accepted_steer_ids.drain(..) {
                                if let Some(lifecycle) = ctx.steer_lifecycle.as_ref() {
                                    lifecycle.applied(&id).await;
                                }
                                yield LoopItem::Event(StreamEvent::SteerApplied { id });
                            }
                            steers_applied = true;
                        }
                        yield LoopItem::Event(event);
                    }
                    Some(ModelTurnItem::Finished(turn)) => {
                        // The current implementation emits a status event first,
                        // but keep the lifecycle correct if a future adapter
                        // produces a finished turn as its first item.
                        if !steers_applied {
                            for id in accepted_steer_ids.drain(..) {
                                if let Some(lifecycle) = ctx.steer_lifecycle.as_ref() {
                                    lifecycle.applied(&id).await;
                                }
                                yield LoopItem::Event(StreamEvent::SteerApplied { id });
                            }
                        }
                        break turn;
                    }
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
                    let action = ToolAction::Call(crate::types::ToolCallAction {
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
