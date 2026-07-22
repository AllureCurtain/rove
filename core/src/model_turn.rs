use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use rove_models::{Message, ModelClient, ModelError, ModelEvent, ToolCallRef, ToolSchema, Usage};

use crate::{Action, AgentEvent, CallId, ToolCallAction, parse_action};

#[derive(Debug)]
pub struct ModelTurn {
    pub full_response: String,
    pub action: Action,
    pub usage: Usage,
    pub tool_calls: Vec<ToolCallRef>,
}

#[derive(Debug)]
pub enum ModelTurnItem {
    Event(AgentEvent),
    Finished(ModelTurn),
    Cancelled,
    Failed(ModelError),
}

pub fn run_model_turn<'a>(
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
        yield ModelTurnItem::Event(AgentEvent::ModelStatus {
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
                Ok(ModelEvent::TextDelta { text }) => {
                    if !text.is_empty() {
                        full_response.push_str(&text);
                        yield ModelTurnItem::Event(AgentEvent::TextDelta { delta: text });
                    }
                }
                Ok(ModelEvent::Usage { usage: current }) => usage = current,
                Ok(ModelEvent::Done) => break,
                Ok(ModelEvent::ThinkingDelta { .. } | ModelEvent::ToolUseDelta { .. }) => {}
                Ok(ModelEvent::ToolUseStart { name, .. }) => {
                    yield ModelTurnItem::Event(AgentEvent::ModelStatus {
                        status: "tool_use_started".to_string(),
                        message: format!("Model selected tool {name}"),
                    });
                }
                Ok(ModelEvent::ToolUseDone { id, name, args }) => {
                    native_tool_calls.push(ToolCallAction {
                        call_id: CallId::new(),
                        tool_use_id: Some(id),
                        name,
                        args,
                    });
                }
                Err(error) => {
                    yield ModelTurnItem::Failed(error);
                    return;
                }
            }
        }

        let tool_calls = tool_refs_from_actions(&native_tool_calls);
        yield ModelTurnItem::Event(AgentEvent::ModelMessage {
            full: full_response.clone(),
            usage: usage.clone(),
            tool_calls: tool_calls.clone(),
        });
        yield ModelTurnItem::Finished(ModelTurn {
            action: build_action_from_model_output(native_tool_calls, &full_response),
            full_response,
            usage,
            tool_calls,
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
    use super::build_action_from_model_output;
    use crate::{Action, CallId, ToolCallAction};

    #[test]
    fn native_tool_use_wins_over_text_fallback() {
        let action = build_action_from_model_output(
            vec![ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: Some("toolu-native".to_string()),
                name: "echo".to_string(),
                args: serde_json::json!({"message":"native"}),
            }],
            r#"{"tool":"fs_read","args":{"path":"Cargo.toml"}}"#,
        );

        assert!(matches!(
            action,
            Action::ToolCall { tool_use_id, name, args, .. }
                if tool_use_id.as_deref() == Some("toolu-native")
                    && name == "echo"
                    && args["message"] == "native"
        ));
    }

    #[test]
    fn text_fallback_is_used_without_native_tool_use() {
        let action = build_action_from_model_output(
            Vec::new(),
            r#"{"tool":"fs_read","args":{"path":"Cargo.toml"}}"#,
        );

        assert!(matches!(
            action,
            Action::ToolCall { tool_use_id, name, args, .. }
                if tool_use_id.is_none()
                    && name == "fs_read"
                    && args["path"] == "Cargo.toml"
        ));
    }
}
