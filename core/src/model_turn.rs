use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use rove_models::{
    AssistantTurn, Message, ModelClient, ModelError, ModelEvent, ModelToolSchema, StopReason,
    ToolCallRef, TurnAssembler, Usage,
};

use crate::{Action, AgentEvent, CallId, ToolCallAction, parse_action};

#[derive(Debug)]
pub struct ModelTurn {
    pub full_response: String,
    pub action: Action,
    pub usage: Usage,
    pub tool_calls: Vec<ToolCallRef>,
    pub assistant_turn: Box<AssistantTurn>,
    pub stop_reason: StopReason,
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
    tools: Vec<ModelToolSchema>,
    cancel_token: CancellationToken,
) -> BoxStream<'a, ModelTurnItem> {
    Box::pin(stream! {
        let mut full_response = String::new();
        let mut assembler = TurnAssembler::new();
        let requires_terminal_event = model.requires_terminal_event();
        let mut saw_terminal_event = false;
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
                        yield ModelTurnItem::Event(AgentEvent::TextDelta { delta: text.clone() });
                    }
                    if let Err(error) = assembler.push(ModelEvent::TextDelta { text }) {
                        yield ModelTurnItem::Failed(error);
                        return;
                    }
                }
                Ok(ModelEvent::Usage { usage: current }) => {
                    if let Err(error) = assembler.push(ModelEvent::Usage { usage: current }) {
                        yield ModelTurnItem::Failed(error);
                        return;
                    }
                }
                Ok(ModelEvent::Done) => {
                    saw_terminal_event = true;
                    if let Err(error) = assembler.push(ModelEvent::Done) {
                        yield ModelTurnItem::Failed(error);
                        return;
                    }
                    break;
                }
                Ok(ModelEvent::ThinkingDelta { text }) => {
                    if let Err(error) = assembler.push(ModelEvent::ThinkingDelta { text }) {
                        yield ModelTurnItem::Failed(error);
                        return;
                    }
                }
                Ok(ModelEvent::ToolUseDelta { id, args_delta }) => {
                    if let Err(error) = assembler.push(ModelEvent::ToolUseDelta { id, args_delta }) {
                        yield ModelTurnItem::Failed(error);
                        return;
                    }
                }
                Ok(ModelEvent::ToolUseStart { id, name }) => {
                    if let Err(error) = assembler.push(ModelEvent::ToolUseStart {
                        id,
                        name: name.clone(),
                    }) {
                        yield ModelTurnItem::Failed(error);
                        return;
                    }
                    yield ModelTurnItem::Event(AgentEvent::ModelStatus {
                        status: "tool_use_started".to_string(),
                        message: format!("Model selected tool {name}"),
                    });
                }
                Ok(ModelEvent::ToolUseDone { id, name, args }) => {
                    if let Err(error) = assembler.push(ModelEvent::ToolUseDone { id, name, args }) {
                        yield ModelTurnItem::Failed(error);
                        return;
                    }
                }
                Err(error) => {
                    yield ModelTurnItem::Failed(error);
                    return;
                }
            }
        }

        if !saw_terminal_event {
            if requires_terminal_event {
                yield ModelTurnItem::Failed(ModelError::StreamInterrupted(
                    "stream ended before a complete terminal turn".to_string(),
                ));
                return;
            }
            // Compatibility for pre-wave embedded ModelClient implementations
            // whose stream contract used EOF as the terminal marker.
            if let Err(error) = assembler.push(ModelEvent::Done) {
                yield ModelTurnItem::Failed(error);
                return;
            }
        }

        let assistant_turn = match assembler.finish() {
            Ok(turn) => turn,
            Err(error) => {
                yield ModelTurnItem::Failed(error);
                return;
            }
        };
        full_response = assistant_turn
            .content
            .iter()
            .filter_map(|block| block.text_value())
            .collect::<String>();
        let native_tool_calls = assistant_turn
            .tool_calls
            .iter()
            .map(|call| ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: call
                    .wire_reference
                    .as_ref()
                    .map(|reference| reference.value.clone()),
                name: call.name.clone(),
                args: call.arguments.clone(),
            })
            .collect::<Vec<_>>();
        let tool_calls = tool_refs_from_actions(&native_tool_calls);
        yield ModelTurnItem::Event(AgentEvent::ModelMessage {
            full: full_response.clone(),
            usage: assistant_turn.usage.clone(),
            tool_calls: tool_calls.clone(),
        });
        yield ModelTurnItem::Finished(ModelTurn {
            action: build_action_from_model_output(native_tool_calls, &full_response),
            full_response,
            usage: assistant_turn.usage.clone(),
            tool_calls,
            stop_reason: assistant_turn.stop_reason.clone(),
            assistant_turn: Box::new(assistant_turn),
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
    use async_trait::async_trait;
    use futures::StreamExt;
    use futures::stream::BoxStream;
    use tokio_util::sync::CancellationToken;

    use super::{ModelTurnItem, build_action_from_model_output, run_model_turn};
    use crate::{Action, CallId, ToolCallAction};
    use rove_models::{
        Message, ModelClient, ModelClientId, ModelError, ModelEvent, ModelToolSchema,
    };

    struct IncompleteModel;

    #[async_trait]
    impl ModelClient for IncompleteModel {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            Box::pin(futures::stream::iter([Ok(ModelEvent::TextDelta {
                text: "partial".to_string(),
            })]))
        }

        fn model_id(&self) -> &str {
            "incomplete"
        }

        fn client_id(&self) -> ModelClientId {
            ModelClientId::opaque("incomplete")
        }

        fn requires_terminal_event(&self) -> bool {
            true
        }
    }

    struct MalformedToolModel;

    #[async_trait]
    impl ModelClient for MalformedToolModel {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            Box::pin(futures::stream::iter([
                Ok(ModelEvent::ToolUseStart {
                    id: "call-a".to_string(),
                    name: "echo".to_string(),
                }),
                Ok(ModelEvent::ToolUseDone {
                    id: "call-a".to_string(),
                    name: "echo".to_string(),
                    args: serde_json::Value::String("not-json-object".to_string()),
                }),
                Ok(ModelEvent::Done),
            ]))
        }

        fn model_id(&self) -> &str {
            "malformed"
        }
    }

    struct LegacyEofModel;

    #[async_trait]
    impl ModelClient for LegacyEofModel {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            Box::pin(futures::stream::iter([Ok(ModelEvent::TextDelta {
                text: "legacy final".to_string(),
            })]))
        }

        fn model_id(&self) -> &str {
            "legacy-eof"
        }
    }

    #[test]
    fn native_tool_use_wins_over_text_fallback() {
        let action = build_action_from_model_output(
            vec![ToolCallAction {
                call_id: CallId::new(),
                tool_use_id: Some("toolu-native".to_string()),
                name: "echo".to_string(),
                args: serde_json::json!({"message":"native"}),
            }],
            r#"{"tool":"read_file","args":{"path":"Cargo.toml"}}"#,
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
            r#"{"tool":"read_file","args":{"path":"Cargo.toml"}}"#,
        );

        assert!(matches!(
            action,
            Action::ToolCall { tool_use_id, name, args, .. }
                if tool_use_id.is_none()
                    && name == "read_file"
                    && args["path"] == "Cargo.toml"
        ));
    }

    #[tokio::test]
    async fn truncated_stream_never_finishes_a_turn() {
        let mut stream = run_model_turn(
            &IncompleteModel,
            vec![Message::user("hello")],
            Vec::new(),
            CancellationToken::new(),
        );
        let mut finished = false;
        let mut failed = false;
        while let Some(item) = stream.next().await {
            match item {
                ModelTurnItem::Finished(_) => finished = true,
                ModelTurnItem::Failed(_) => failed = true,
                _ => {}
            }
        }
        assert!(!finished);
        assert!(failed);
    }

    #[tokio::test]
    async fn malformed_tool_arguments_fail_before_a_finished_turn() {
        let mut stream = run_model_turn(
            &MalformedToolModel,
            vec![Message::user("use echo")],
            Vec::new(),
            CancellationToken::new(),
        );
        let mut finished = false;
        let mut failed = false;
        while let Some(item) = stream.next().await {
            match item {
                ModelTurnItem::Finished(_) => finished = true,
                ModelTurnItem::Failed(_) => failed = true,
                _ => {}
            }
        }
        assert!(!finished);
        assert!(failed);
    }

    #[tokio::test]
    async fn legacy_eof_clients_remain_compatible_until_they_opt_in() {
        let mut stream = run_model_turn(
            &LegacyEofModel,
            vec![Message::user("hello")],
            Vec::new(),
            CancellationToken::new(),
        );
        let mut finished = None;
        while let Some(item) = stream.next().await {
            if let ModelTurnItem::Finished(turn) = item {
                finished = Some(turn);
            }
        }
        let turn = finished.expect("legacy EOF should synthesize the compatibility terminal");
        assert_eq!(turn.full_response, "legacy final");
    }
}
