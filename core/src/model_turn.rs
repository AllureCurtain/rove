use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use rove_models::{
    AssistantTurn, InternalCallId, Message, ModelClient, ModelError, ModelEvent, ModelToolSchema,
    StopReason, ToolCall as CanonicalToolCall, ToolCallRef, TurnAssembler, Usage,
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
    /// Cancellation was observed before the turn completed.
    ///
    /// The item carries what the turn had already accumulated, because only the
    /// turn knows it: hosts own persistence and canonical events, the turn owns
    /// its in-memory outcome. Whether anything is done with it — and how long a
    /// host waits for a late completion before giving up — is the host's policy.
    Cancelled(CancelledTurn),
    Failed(ModelError),
}

/// What one in-flight model turn had accumulated when cancellation stopped it.
#[derive(Debug, Clone, Default)]
pub struct CancelledTurn {
    /// Assistant text received from the provider before the stop.
    pub partial: String,
    /// Usage the provider reported before the stop, or the default when it
    /// reported none. Inventing numbers here would make the cancel path lie.
    pub usage: Usage,
}

pub fn run_model_turn<'a>(
    model: &'a dyn ModelClient,
    messages: Vec<Message>,
    tools: Vec<ModelToolSchema>,
    cancel_token: CancellationToken,
) -> BoxStream<'a, ModelTurnItem> {
    Box::pin(stream! {
        let capabilities = model.capabilities();
        if let Err(error) = capabilities.validate_tools(&tools) {
            yield ModelTurnItem::Failed(error);
            return;
        }
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
                    yield ModelTurnItem::Cancelled(CancelledTurn {
                        partial: full_response,
                        usage: assembler.usage().clone(),
                    });
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
                Ok(ModelEvent::StopReason { reason }) => {
                    if let Err(error) = assembler.push(ModelEvent::StopReason { reason }) {
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

        let mut assistant_turn = match assembler.finish() {
            Ok(turn) => turn,
            Err(error) => {
                yield ModelTurnItem::Failed(error);
                return;
            }
        };
        let history_protocol = model.history_protocol();
        for call in &mut assistant_turn.tool_calls {
            if let Some(reference) = call.wire_reference.as_mut() {
                reference.protocol.clone_from(&history_protocol);
            }
        }
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
        let action = build_action_from_model_output(
            native_tool_calls,
            &full_response,
            model.compatibility_text_tool_calls(),
        );
        if assistant_turn.tool_calls.is_empty() {
            assistant_turn.tool_calls = canonical_calls_from_compatibility_action(&action);
            if !assistant_turn.tool_calls.is_empty() {
                assistant_turn.stop_reason = StopReason::ToolUse;
            }
        }
        if let Err(error) = assistant_turn
            .validate()
            .map_err(|error| ModelError::StreamInterrupted(error.to_string()))
            .and_then(|_| capabilities.validate_assistant_turn(&assistant_turn))
        {
            yield ModelTurnItem::Failed(error);
            return;
        }
        yield ModelTurnItem::Event(AgentEvent::ModelMessage {
            full: full_response.clone(),
            usage: assistant_turn.usage.clone(),
            tool_calls: tool_calls.clone(),
            assistant_turn: Box::new(assistant_turn.clone()),
        });
        yield ModelTurnItem::Finished(ModelTurn {
            action,
            full_response,
            usage: assistant_turn.usage.clone(),
            tool_calls,
            stop_reason: assistant_turn.stop_reason.clone(),
            assistant_turn: Box::new(assistant_turn),
        });
    })
}

fn canonical_calls_from_compatibility_action(action: &Action) -> Vec<CanonicalToolCall> {
    let calls = match action {
        Action::ToolCall {
            call_id,
            tool_use_id,
            name,
            args,
        } => vec![ToolCallAction {
            call_id: *call_id,
            tool_use_id: tool_use_id.clone(),
            name: name.clone(),
            args: args.clone(),
        }],
        Action::ToolBatch { calls } => calls.clone(),
        Action::Final { .. } | Action::Malformed { .. } => Vec::new(),
    };
    calls
        .into_iter()
        .map(|call| CanonicalToolCall {
            internal_call_id: InternalCallId::new(call.call_id.to_string())
                .expect("runtime call ids are valid canonical ids"),
            // Compatibility JSON actions are validated again by the runtime
            // tool boundary. Keep a bounded canonical identity even when the
            // legacy payload is malformed so the recoverable ToolCallFailed
            // result can still correlate during persistence.
            name: if call.name.trim().is_empty() {
                "invalid_tool".to_string()
            } else {
                call.name
            },
            arguments: if call.args.is_object() {
                call.args
            } else {
                serde_json::json!({})
            },
            wire_reference: None,
        })
        .collect()
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

fn build_action_from_model_output(
    calls: Vec<ToolCallAction>,
    full_response: &str,
    compatibility_text_tool_calls: bool,
) -> Action {
    match calls.len() {
        0 if compatibility_text_tool_calls => parse_action(full_response),
        0 => Action::Final {
            text: full_response.to_string(),
        },
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use futures::StreamExt;
    use futures::stream::BoxStream;
    use tokio_util::sync::CancellationToken;

    use super::{CancelledTurn, ModelTurnItem, build_action_from_model_output, run_model_turn};
    use crate::{Action, AgentEvent, CallId, ToolCallAction};
    use rove_models::{
        Message, ModelClient, ModelClientId, ModelError, ModelEvent, ModelToolSchema, Usage,
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

    /// Emits one delta and then stays in flight until it is cancelled, so the
    /// cancel path can be observed with text already accumulated. Reported
    /// usage is sent *before* that delta, because a stop can only be honest
    /// about what the turn has already accounted for; `usage` is omitted
    /// entirely when `None`, which is how a provider that reports none behaves.
    struct HangingModel {
        usage: Option<Usage>,
    }

    #[async_trait]
    impl ModelClient for HangingModel {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            let mut events = Vec::new();
            if let Some(usage) = self.usage.clone() {
                events.push(Ok(ModelEvent::Usage { usage }));
            }
            events.push(Ok(ModelEvent::TextDelta {
                text: "half".to_string(),
            }));
            Box::pin(
                futures::stream::iter(events)
                    .chain(futures::stream::pending::<Result<ModelEvent, ModelError>>()),
            )
        }

        fn model_id(&self) -> &str {
            "hanging"
        }

        fn requires_terminal_event(&self) -> bool {
            true
        }
    }

    struct DispatchCountingModel {
        dispatches: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for DispatchCountingModel {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ModelToolSchema],
        ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
            self.dispatches.fetch_add(1, Ordering::SeqCst);
            Box::pin(futures::stream::empty())
        }

        fn model_id(&self) -> &str {
            "dispatch-counting"
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
            true,
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
            true,
        );

        assert!(matches!(
            action,
            Action::ToolCall { tool_use_id, name, args, .. }
                if tool_use_id.is_none()
                    && name == "read_file"
                    && args["path"] == "Cargo.toml"
        ));
    }

    #[test]
    fn native_clients_do_not_promote_text_json_to_tool_calls() {
        let text = r#"{"tool":"read_file","args":{"path":"Cargo.toml"}}"#;
        assert!(matches!(
            build_action_from_model_output(Vec::new(), text, false),
            Action::Final { text: value } if value == text
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
    async fn invalid_tool_schema_fails_before_model_dispatch() {
        let dispatches = Arc::new(AtomicUsize::new(0));
        let model = DispatchCountingModel {
            dispatches: dispatches.clone(),
        };
        let mut stream = run_model_turn(
            &model,
            vec![Message::user("hello")],
            vec![ModelToolSchema {
                name: "invalid".to_string(),
                description: String::new(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "oneOf": []
                }),
            }],
            CancellationToken::new(),
        );

        assert!(matches!(
            stream.next().await,
            Some(ModelTurnItem::Failed(ModelError::InvalidConfiguration(_)))
        ));
        assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_cancelled_turn_hands_back_its_accumulated_text_and_usage() {
        let usage = Usage {
            prompt_tokens: 3,
            completion_tokens: 2,
            total_tokens: 5,
            cached_tokens: 1,
        };
        let cancel = CancellationToken::new();
        let model = HangingModel {
            usage: Some(usage.clone()),
        };
        let mut stream = run_model_turn(
            &model,
            vec![Message::user("hello")],
            Vec::new(),
            cancel.clone(),
        );

        assert!(matches!(
            stream.next().await,
            Some(ModelTurnItem::Event(AgentEvent::ModelStatus { .. }))
        ));
        assert!(matches!(
            stream.next().await,
            Some(ModelTurnItem::Event(AgentEvent::TextDelta { .. }))
        ));

        // The turn is now in flight with text the user has already seen.
        cancel.cancel();
        match stream.next().await {
            Some(ModelTurnItem::Cancelled(CancelledTurn {
                partial,
                usage: seen,
            })) => {
                assert_eq!(partial, "half");
                assert_eq!(
                    seen, usage,
                    "a cancelled turn must report the usage it actually saw"
                );
            }
            other => panic!("expected the accumulated outcome, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_cancelled_turn_that_reported_no_usage_keeps_the_default() {
        let cancel = CancellationToken::new();
        let model = HangingModel { usage: None };
        // A model that only streams text and never reports usage: the cancel
        // path must not invent token counts.
        let mut stream = run_model_turn(
            &model,
            vec![Message::user("hello")],
            Vec::new(),
            cancel.clone(),
        );
        let _ = stream.next().await;
        let _ = stream.next().await;
        cancel.cancel();
        match stream.next().await {
            Some(ModelTurnItem::Cancelled(CancelledTurn { partial, usage })) => {
                assert_eq!(partial, "half");
                assert_eq!(usage, Usage::default());
            }
            other => panic!("expected the accumulated outcome, got {other:?}"),
        }
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
