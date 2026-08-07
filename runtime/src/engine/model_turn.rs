use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use rove_core::AgentEvent;

use crate::events::StreamEvent;
use crate::types::{Message, ModelToolSchema};
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
                    if let Some(event) = durable_model_event(event) {
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

fn durable_model_event(event: AgentEvent) -> Option<StreamEvent> {
    match event {
        AgentEvent::ModelStatus { status, message } => {
            Some(StreamEvent::ModelStatus { status, message })
        }
        AgentEvent::TextDelta { delta } => Some(StreamEvent::LlmChunk { delta }),
        AgentEvent::ModelMessage {
            full,
            usage,
            tool_calls,
            assistant_turn,
        } => Some(StreamEvent::LlmMessage {
            full,
            usage,
            tool_calls,
            assistant_turn: Some(assistant_turn),
        }),
        _ => None,
    }
}
