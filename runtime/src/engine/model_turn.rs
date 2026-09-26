use std::time::Duration;

use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use rove_core::AgentEvent;

use crate::events::StreamEvent;
use crate::types::{Message, ModelToolSchema, RunMode};
use rove_models::ModelClient;
use rove_models::ModelError;

pub(crate) type ModelTurn = rove_core::model_turn::ModelTurn;

/// How long a cancelled model turn keeps draining its in-flight request before
/// the runtime gives up on it and persists what it already had.
///
/// This is the whole abort-salvage policy, and it is deliberately a named
/// constant rather than a configuration knob: it bounds how long a user's stop
/// can take to become a terminal fact, so it belongs to the runtime's cancel
/// contract instead of to a deployment. The window is only ever opened for a
/// turn that has already produced non-empty text, so a stop before the first
/// token — and every tool-path cancellation — stays exactly as fast as before.
pub(crate) const ABORT_SALVAGE_WINDOW: Duration = Duration::from_millis(1_500);

#[derive(Debug)]
pub(crate) enum ModelTurnItem {
    Event(StreamEvent),
    Finished(ModelTurn),
    Cancelled,
    Failed(ModelError),
}

/// Drive one model call, salvaging visible text when the run is cancelled.
///
/// Cancellation is observed on the caller's run token, but the turn underneath
/// is stopped through a separate, runtime-owned token. That split is what makes
/// a bounded salvage window possible at all: the cancel signal is acknowledged
/// immediately while the request that is already in flight is still allowed to
/// finish, and nothing else about cancellation moves. A child token could not
/// do this — cancelling the parent cancels the child, so the in-flight request
/// would die at the same instant the cancel was observed.
///
/// The window waits for text only. It never runs a tool, starts a model call,
/// waits for an approval, or dispatches anything: the salvaged message is
/// bookkeeping for what the user has already seen.
pub(crate) fn run_model_turn<'a>(
    model: &'a dyn ModelClient,
    messages: Vec<Message>,
    tools: Vec<ModelToolSchema>,
    cancel_token: CancellationToken,
    run_mode: RunMode,
) -> BoxStream<'a, ModelTurnItem> {
    Box::pin(stream! {
        let turn_cancel = CancellationToken::new();
        let mut core_stream =
            rove_core::model_turn::run_model_turn(model, messages, tools, turn_cancel.clone());
        // Text the user has actually been shown, accumulated from the durable
        // projection of the turn. This — not the raw provider stream — is what
        // decides whether there is anything worth salvaging, so review turns
        // (whose text is never published) keep the pre-R2b behavior.
        let mut visible = String::new();
        let mut cancel_observed = false;
        // Whether the window below is open, and until when. Both are cleared
        // with the flag so a repeated cancel can never re-arm the same stop.
        let mut salvage_until: Option<tokio::time::Instant> = None;
        // Whether this stop salvages at all: it has text to keep and the window
        // is therefore allowed to open. A stop with nothing on screen must stay
        // exactly as fast, and as silent, as it was before salvage existed.
        let mut salvage_armed = false;

        loop {
            let item = tokio::select! {
                biased;
                // A second cancel after the first only means "still cancelled":
                // the window is not restarted, because a duplicate stop must not
                // write a second partial or a second event.
                _ = cancel_token.cancelled(), if !cancel_observed => {
                    cancel_observed = true;
                    if visible.trim().is_empty() {
                        // Nothing on screen to keep: stop right now, exactly as
                        // this path behaved before salvage existed.
                        turn_cancel.cancel();
                    } else {
                        salvage_armed = true;
                        salvage_until =
                            Some(tokio::time::Instant::now() + ABORT_SALVAGE_WINDOW);
                    }
                    continue;
                }
                // The window expired without the request finishing. Stop the
                // turn and persist the accumulated partial below.
                _ = async {
                    match salvage_until {
                        Some(until) => tokio::time::sleep_until(until).await,
                        None => std::future::pending::<()>().await,
                    }
                }, if salvage_until.is_some() => {
                    salvage_until = None;
                    turn_cancel.cancel();
                    continue;
                }
                item = core_stream.next() => item,
            };

            let Some(item) = item else {
                // The model-turn contract always ends with a terminal item; a
                // silent end keeps the pre-salvage behavior for that case.
                return;
            };

            match item {
                rove_core::model_turn::ModelTurnItem::Event(event) => {
                    if let Some(durable) = durable_model_event(event, run_mode) {
                        // Only text published before the stop counts as "the user
                        // already saw this".
                        match &durable {
                            StreamEvent::LlmChunk { delta } if !cancel_observed => {
                                visible.push_str(delta);
                            }
                            _ => {}
                        }
                        // Once a stop is salvaging, the turn's remaining output
                        // is captured for the persisted message but is not
                        // re-published as new stream deltas: cancellation
                        // already told every consumer to stop streaming.
                        if !salvage_armed {
                            yield ModelTurnItem::Event(durable);
                        }
                    }
                }
                rove_core::model_turn::ModelTurnItem::Finished(turn) => {
                    if salvage_armed {
                        // The in-flight request finished inside the window. Its
                        // message is complete, so it is persisted without the
                        // aborted marker — but the run is still cancelled, and
                        // the turn's tool calls are dropped rather than handed
                        // to the kernel: nothing may be dispatched from salvage.
                        let event = salvaged_message(
                            turn.full_response.clone(),
                            turn.assistant_turn.content.clone(),
                            turn.usage.clone(),
                            false,
                            run_mode,
                        );
                        yield ModelTurnItem::Event(event);
                        yield ModelTurnItem::Cancelled;
                        return;
                    }
                    yield ModelTurnItem::Finished(turn);
                    return;
                }
                rove_core::model_turn::ModelTurnItem::Cancelled(cancelled) => {
                    if salvage_armed && !cancelled.partial.trim().is_empty() {
                        let event = salvaged_message(
                            cancelled.partial.clone(),
                            vec![rove_models::ContentBlock::text(cancelled.partial)],
                            cancelled.usage,
                            true,
                            run_mode,
                        );
                        yield ModelTurnItem::Event(event);
                    }
                    yield ModelTurnItem::Cancelled;
                    return;
                }
                rove_core::model_turn::ModelTurnItem::Failed(error) => {
                    yield ModelTurnItem::Failed(error);
                    return;
                }
            }
        }
    })
}

/// Build the canonical `LlmMessage` for a cancelled turn's salvaged text.
///
/// The turn is normalized to text-only with a `cancelled` stop state, whatever
/// the underlying request did: a salvaged message exists only to keep text the
/// user has already seen, and a canonical turn that still carried tool calls
/// could hand a later resume a tool to run after a cancel.
fn salvaged_message(
    full: String,
    content: Vec<rove_models::ContentBlock>,
    usage: rove_models::Usage,
    aborted: bool,
    run_mode: RunMode,
) -> StreamEvent {
    let assistant_turn = rove_models::AssistantTurn {
        content,
        tool_calls: Vec::new(),
        usage: usage.clone(),
        stop_reason: rove_models::StopReason::Cancelled,
        ..rove_models::AssistantTurn::default()
    };
    llm_message_event(
        full,
        usage,
        Vec::new(),
        Some(Box::new(assistant_turn)),
        run_mode,
        aborted,
    )
}

fn durable_model_event(event: AgentEvent, run_mode: RunMode) -> Option<StreamEvent> {
    match event {
        AgentEvent::ModelStatus { status, message } => {
            Some(StreamEvent::ModelStatus { status, message })
        }
        AgentEvent::TextDelta { delta } if run_mode == RunMode::Normal => {
            Some(StreamEvent::LlmChunk { delta })
        }
        AgentEvent::TextDelta { .. } => None,
        AgentEvent::ModelMessage {
            full,
            usage,
            tool_calls,
            assistant_turn,
        } => Some(llm_message_event(
            full,
            usage,
            tool_calls,
            Some(assistant_turn),
            run_mode,
            false,
        )),
        _ => None,
    }
}

/// The one place a canonical `LlmMessage` event is built.
///
/// Both the completed-turn path and the salvage path go through it, so review
/// redaction is applied identically and cannot be forgotten by one of them.
fn llm_message_event(
    mut full: String,
    usage: rove_models::Usage,
    mut tool_calls: Vec<rove_models::ToolCallRef>,
    mut assistant_turn: Option<Box<rove_models::AssistantTurn>>,
    run_mode: RunMode,
    aborted: bool,
) -> StreamEvent {
    if run_mode == RunMode::Review {
        full = "[review model output omitted]".to_string();
        for call in &mut tool_calls {
            call.args = serde_json::json!({"redacted": true});
        }
        if let Some(turn) = assistant_turn.as_mut() {
            for call in &mut turn.tool_calls {
                call.arguments = serde_json::json!({"redacted": true});
            }
            if !turn.content.is_empty() {
                turn.content = vec![rove_models::ContentBlock::text(
                    "[review model output omitted]",
                )];
            }
        }
    }
    StreamEvent::LlmMessage {
        full,
        usage,
        tool_calls,
        assistant_turn,
        aborted,
    }
}
