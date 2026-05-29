use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::core::events::StreamEvent;
use crate::core::parser::parse_action;
use crate::core::types::{Action, CallId, Message, ToolCallAction, ToolCallRef, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, ModelEvent};

#[derive(Debug)]
pub(crate) struct ModelTurn {
    pub full_response: String,
    pub action: Action,
}

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
    tools: Vec<ToolSchema>,
    cancel_token: CancellationToken,
) -> BoxStream<'a, ModelTurnItem> {
    Box::pin(stream! {
        let mut full_response = String::new();
        let mut usage = Usage::default();
        let mut native_tool_calls = Vec::new();
        let mut model_stream = model.stream(&messages, &tools);
        yield ModelTurnItem::Event(StreamEvent::ModelStatus {
            status: "thinking".to_string(),
            message: "Model is thinking".to_string(),
        });

        loop {
            let chunk_result = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => {
                    yield ModelTurnItem::Cancelled;
                    return;
                }
                chunk = model_stream.next() => chunk,
            };
            let Some(chunk_result) = chunk_result else {
                break;
            };
            match chunk_result {
                Ok(model_event) => match model_event {
                    ModelEvent::TextDelta { text } => {
                        if !text.is_empty() {
                            full_response.push_str(&text);
                            yield ModelTurnItem::Event(StreamEvent::LlmChunk { delta: text });
                        }
                    }
                    ModelEvent::Usage { usage: u } => {
                        usage = u;
                    }
                    ModelEvent::Done => break,
                    ModelEvent::ThinkingDelta { .. } | ModelEvent::ToolUseDelta { .. } => {}
                    ModelEvent::ToolUseStart { name, .. } => {
                        yield ModelTurnItem::Event(StreamEvent::ModelStatus {
                            status: "tool_use_started".to_string(),
                            message: format!("Model selected tool {name}"),
                        });
                    }
                    ModelEvent::ToolUseDone { id, name, args } => {
                        native_tool_calls.push(ToolCallAction {
                            call_id: CallId::new(),
                            tool_use_id: Some(id),
                            name,
                            args,
                        });
                    }
                },
                Err(err) => {
                    yield ModelTurnItem::Failed(err);
                    return;
                }
            }
        }

        yield ModelTurnItem::Event(StreamEvent::LlmMessage {
            full: full_response.clone(),
            usage,
            tool_calls: tool_refs_from_actions(&native_tool_calls),
        });
        let action = build_action_from_model_output(native_tool_calls, &full_response);
        yield ModelTurnItem::Finished(ModelTurn {
            full_response,
            action,
        });
    })
}

fn tool_refs_from_actions(calls: &[ToolCallAction]) -> Vec<ToolCallRef> {
    calls
        .iter()
        .filter_map(|call| {
            call.tool_use_id.as_ref().map(|id| ToolCallRef {
                id: id.clone(),
                name: call.name.clone(),
                args: call.args.clone(),
            })
        })
        .collect()
}

fn build_action_from_model_output(calls: Vec<ToolCallAction>, full_response: &str) -> Action {
    match calls.len() {
        0 => parse_action(full_response),
        1 => {
            let call = calls.into_iter().next().expect("one tool call");
            Action::ToolCall {
                call_id: call.call_id,
                tool_use_id: call.tool_use_id,
                name: call.name,
                args: call.args,
            }
        }
        _ => Action::ToolBatch { calls },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_builder_prefers_native_tool_use_over_text_fallback() {
        let action = build_action_from_model_output(
            vec![ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: Some("toolu-native".to_string()),
                name: "echo".to_string(),
                args: serde_json::json!({ "message": "native" }),
            }],
            r#"{"tool":"fs_read","args":{"path":"Cargo.toml"}}"#,
        );

        match action {
            Action::ToolCall {
                tool_use_id,
                name,
                args,
                ..
            } => {
                assert_eq!(tool_use_id.as_deref(), Some("toolu-native"));
                assert_eq!(name, "echo");
                assert_eq!(args["message"], "native");
            }
            other => panic!("expected native tool call, got {other:?}"),
        }
    }

    #[test]
    fn action_builder_uses_json_text_fallback_when_native_tool_use_is_absent() {
        let action = build_action_from_model_output(
            Vec::new(),
            r#"{"tool":"fs_read","args":{"path":"Cargo.toml"}}"#,
        );

        match action {
            Action::ToolCall {
                tool_use_id,
                name,
                args,
                ..
            } => {
                assert_eq!(tool_use_id, None);
                assert_eq!(name, "fs_read");
                assert_eq!(args["path"], "Cargo.toml");
            }
            other => panic!("expected text-parsed tool call, got {other:?}"),
        }
    }
}
