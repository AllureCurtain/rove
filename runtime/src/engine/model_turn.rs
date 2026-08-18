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

#[derive(Debug)]
pub(crate) enum ModelTurnItem {
    Event(StreamEvent),
    Finished(ModelTurn),
    Cancelled,
    Failed(ModelError),
}

pub(crate) fn run_model_turn<'a>(
    model: &'a dyn ModelClient,
    messages: Vec<Message>,
    tools: Vec<ModelToolSchema>,
    cancel_token: CancellationToken,
    run_mode: RunMode,
) -> BoxStream<'a, ModelTurnItem> {
    Box::pin(stream! {
        let mut core_stream = rove_core::model_turn::run_model_turn(
            model,
            messages,
            tools,
            cancel_token,
        );
        while let Some(item) = core_stream.next().await {
            match item {
                rove_core::model_turn::ModelTurnItem::Event(event) => {
                    if let Some(event) = durable_model_event(event, run_mode) {
                        yield ModelTurnItem::Event(event);
                    }
                }
                rove_core::model_turn::ModelTurnItem::Finished(turn) => {
                    yield ModelTurnItem::Finished(turn);
                    return;
                }
                rove_core::model_turn::ModelTurnItem::Cancelled => {
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
            mut full,
            usage,
            mut tool_calls,
            mut assistant_turn,
        } => {
            if run_mode == RunMode::Review {
                full = "[review model output omitted]".to_string();
                for call in &mut tool_calls {
                    call.args = serde_json::json!({"redacted": true});
                }
                for call in &mut assistant_turn.tool_calls {
                    call.arguments = serde_json::json!({"redacted": true});
                }
                if !assistant_turn.content.is_empty() {
                    assistant_turn.content = vec![rove_models::ContentBlock::text(
                        "[review model output omitted]",
                    )];
                }
            }
            Some(StreamEvent::LlmMessage {
                full,
                usage,
                tool_calls,
                assistant_turn: Some(assistant_turn),
            })
        }
        _ => None,
    }
}
