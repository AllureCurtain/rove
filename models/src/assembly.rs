use std::collections::{BTreeMap, BTreeSet};

use crate::{
    AssistantTurn, ContentBlock, InternalCallId, MAX_CONTENT_BYTES, MAX_TOOL_ARGUMENT_BYTES,
    MAX_TOOL_CALLS, MAX_TOOL_ID_BYTES, MAX_TOOL_NAME_BYTES, ModelError, ModelEvent,
    ProtocolValidationError, StopReason, ToolCall, Usage,
};

const MAX_TEXT_BYTES: usize = MAX_CONTENT_BYTES;

/// Shared semantic stream assembler used after each provider-specific decoder.
///
/// Wire decoders are responsible only for framing and extracting native
/// fragments.  This state machine owns correlation, bounds, terminal-state
/// validation, and the normalized assistant-turn result consumed by Core.
#[derive(Debug, Default)]
pub struct TurnAssembler {
    text: String,
    usage: Usage,
    calls: BTreeMap<String, PartialCall>,
    completed: BTreeSet<String>,
    terminal: Option<StopReason>,
    event_count: usize,
}

#[derive(Debug)]
struct PartialCall {
    internal_id: InternalCallId,
    name: String,
    arguments: String,
    wire_id: String,
    done: bool,
}

impl TurnAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one normalized event.  Errors are returned before a turn can be
    /// handed to tool policy, so malformed or conflicting calls execute zero
    /// tools.
    pub fn push(&mut self, event: ModelEvent) -> Result<(), ModelError> {
        self.event_count = self.event_count.saturating_add(1);
        match event {
            ModelEvent::TextDelta { text } => {
                append_bounded(&mut self.text, &text, MAX_TEXT_BYTES, "text")?;
            }
            ModelEvent::ThinkingDelta { .. } => {
                // Reasoning/signature blocks are intentionally not replayed in
                // canonical history during this wave.
            }
            ModelEvent::ToolUseStart { id, name } => self.start_call(id, name)?,
            ModelEvent::ToolUseDelta { id, args_delta } => {
                let call = self.calls.get_mut(&id).ok_or_else(|| {
                    protocol_error(ProtocolValidationError::UnknownCall { id: id.clone() })
                })?;
                append_bounded(
                    &mut call.arguments,
                    &args_delta,
                    MAX_TOOL_ARGUMENT_BYTES,
                    "tool arguments",
                )?;
            }
            ModelEvent::ToolUseDone { id, name, args } => self.finish_call(id, name, args)?,
            ModelEvent::Usage { usage } => self.usage = usage,
            ModelEvent::Done => self.set_terminal(StopReason::default())?,
        }
        Ok(())
    }

    /// Mark the stream with an explicit normalized stop reason when an adapter
    /// has one.  Existing adapters use `ModelEvent::Done`, which maps to
    /// `tool_use` for a turn containing calls and `end_turn` otherwise.
    pub fn stop(&mut self, reason: StopReason) -> Result<(), ModelError> {
        self.set_terminal(reason)
    }

    pub fn finish(self) -> Result<AssistantTurn, ModelError> {
        let reason = self
            .terminal
            .ok_or_else(|| protocol_error(ProtocolValidationError::IncompleteTurn))?;
        if let Some(call) = self.calls.values().find(|call| !call.done) {
            return Err(protocol_error(ProtocolValidationError::IncompleteCall {
                id: call.internal_id.to_string(),
            }));
        }

        let mut tool_calls = Vec::with_capacity(self.completed.len());
        for call in self.calls.into_values() {
            let arguments =
                serde_json::from_str::<serde_json::Value>(&call.arguments).map_err(|_| {
                    protocol_error(ProtocolValidationError::InvalidArguments {
                        reason: format!(
                            "tool call `{}` arguments are not valid JSON",
                            call.internal_id
                        ),
                    })
                })?;
            tool_calls.push(ToolCall {
                internal_call_id: call.internal_id,
                name: call.name,
                arguments,
                wire_reference: Some(crate::WireCallReference {
                    protocol: "stream".to_string(),
                    value: call.wire_id,
                }),
            });
        }

        let content = if self.text.is_empty() {
            Vec::new()
        } else {
            vec![ContentBlock::text(self.text)]
        };
        let turn = AssistantTurn {
            content,
            tool_calls,
            usage: self.usage,
            stop_reason: if !self.completed.is_empty() && matches!(reason, StopReason::EndTurn) {
                StopReason::ToolUse
            } else {
                reason
            },
            ..AssistantTurn::default()
        };
        turn.validate().map_err(protocol_error)?;
        Ok(turn)
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }

    fn start_call(&mut self, id: String, name: String) -> Result<(), ModelError> {
        bounded_id(&id, "wire call id")?;
        if name.trim().is_empty() {
            return Err(protocol_error(ProtocolValidationError::EmptyField {
                field: "tool name",
            }));
        }
        if name.len() > MAX_TOOL_NAME_BYTES {
            return Err(protocol_error(ProtocolValidationError::TooLarge {
                field: "tool name",
                max: MAX_TOOL_NAME_BYTES,
            }));
        }
        if self.calls.len() >= MAX_TOOL_CALLS {
            return Err(protocol_error(ProtocolValidationError::TooMany {
                field: "tool calls",
                max: MAX_TOOL_CALLS,
            }));
        }
        if self.calls.contains_key(&id) || self.completed.contains(&id) {
            return Err(protocol_error(ProtocolValidationError::DuplicateId { id }));
        }
        let internal_id = InternalCallId::new(id.clone()).map_err(protocol_error)?;
        self.calls.insert(
            id.clone(),
            PartialCall {
                internal_id,
                name,
                arguments: String::new(),
                wire_id: id,
                done: false,
            },
        );
        Ok(())
    }

    fn finish_call(
        &mut self,
        id: String,
        name: String,
        args: serde_json::Value,
    ) -> Result<(), ModelError> {
        let call = self.calls.get_mut(&id).ok_or_else(|| {
            protocol_error(ProtocolValidationError::UnknownCall { id: id.clone() })
        })?;
        if call.done || self.completed.contains(&id) {
            return Err(protocol_error(
                ProtocolValidationError::DuplicateCompletion { id },
            ));
        }
        if call.name != name {
            return Err(protocol_error(ProtocolValidationError::InvalidArguments {
                reason: "tool call name changed while assembling the stream".to_string(),
            }));
        }
        let encoded = serde_json::to_vec(&args).map_err(|_| {
            protocol_error(ProtocolValidationError::InvalidArguments {
                reason: "tool arguments cannot be serialized".to_string(),
            })
        })?;
        if encoded.len() > MAX_TOOL_ARGUMENT_BYTES {
            return Err(protocol_error(ProtocolValidationError::TooLarge {
                field: "tool arguments",
                max: MAX_TOOL_ARGUMENT_BYTES,
            }));
        }
        if !args.is_object() {
            return Err(protocol_error(ProtocolValidationError::InvalidArguments {
                reason: "tool arguments must be a JSON object".to_string(),
            }));
        }
        if !call.arguments.is_empty() {
            let delta_args =
                serde_json::from_str::<serde_json::Value>(&call.arguments).map_err(|_| {
                    protocol_error(ProtocolValidationError::InvalidArguments {
                        reason: "tool argument fragments are not valid JSON".to_string(),
                    })
                })?;
            if delta_args != args {
                return Err(protocol_error(ProtocolValidationError::InvalidArguments {
                    reason: "tool call completion conflicts with assembled arguments".to_string(),
                }));
            }
        }
        call.arguments = args.to_string();
        call.done = true;
        self.completed.insert(id);
        Ok(())
    }

    fn set_terminal(&mut self, reason: StopReason) -> Result<(), ModelError> {
        if self.terminal.is_some() {
            return Err(protocol_error(
                ProtocolValidationError::DuplicateCompletion {
                    id: "turn".to_string(),
                },
            ));
        }
        if let Some(call) = self.calls.values().find(|call| !call.done) {
            return Err(protocol_error(ProtocolValidationError::IncompleteCall {
                id: call.internal_id.to_string(),
            }));
        }
        self.terminal = Some(reason);
        Ok(())
    }
}

pub fn assemble_turn<I>(events: I) -> Result<AssistantTurn, ModelError>
where
    I: IntoIterator<Item = Result<ModelEvent, ModelError>>,
{
    let mut assembler = TurnAssembler::new();
    for event in events {
        assembler.push(event?)?;
    }
    assembler.finish()
}

fn bounded_id(value: &str, field: &'static str) -> Result<(), ModelError> {
    if value.trim().is_empty() {
        return Err(protocol_error(ProtocolValidationError::EmptyField {
            field,
        }));
    }
    if value.len() > MAX_TOOL_ID_BYTES {
        return Err(protocol_error(ProtocolValidationError::TooLarge {
            field,
            max: MAX_TOOL_ID_BYTES,
        }));
    }
    Ok(())
}

fn append_bounded(
    target: &mut String,
    delta: &str,
    max: usize,
    field: &'static str,
) -> Result<(), ModelError> {
    let size = target.len().saturating_add(delta.len());
    if size > max {
        return Err(protocol_error(ProtocolValidationError::TooLarge {
            field,
            max,
        }));
    }
    target.push_str(delta);
    Ok(())
}

fn protocol_error(error: ProtocolValidationError) -> ModelError {
    ModelError::StreamInterrupted(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_text_and_native_tool_call_only_after_terminal_done() {
        let events = vec![
            Ok(ModelEvent::TextDelta {
                text: "hello".to_string(),
            }),
            Ok(ModelEvent::ToolUseStart {
                id: "call-a".to_string(),
                name: "echo".to_string(),
            }),
            Ok(ModelEvent::ToolUseDelta {
                id: "call-a".to_string(),
                args_delta: "{\"message\":\"ok\"}".to_string(),
            }),
            Ok(ModelEvent::ToolUseDone {
                id: "call-a".to_string(),
                name: "echo".to_string(),
                args: serde_json::json!({"message":"ok"}),
            }),
            Ok(ModelEvent::Done),
        ];
        let turn = assemble_turn(events).unwrap();
        assert_eq!(turn.stop_reason, StopReason::ToolUse);
        assert_eq!(turn.tool_calls[0].arguments["message"], "ok");
    }

    #[test]
    fn incomplete_or_duplicate_calls_are_rejected_before_execution() {
        let mut assembler = TurnAssembler::new();
        assembler
            .push(ModelEvent::ToolUseStart {
                id: "call-a".to_string(),
                name: "echo".to_string(),
            })
            .unwrap();
        assert!(assembler.push(ModelEvent::Done).is_err());

        let mut duplicate = TurnAssembler::new();
        duplicate
            .push(ModelEvent::ToolUseStart {
                id: "call-a".to_string(),
                name: "echo".to_string(),
            })
            .unwrap();
        assert!(
            duplicate
                .push(ModelEvent::ToolUseStart {
                    id: "call-a".to_string(),
                    name: "echo".to_string(),
                })
                .is_err()
        );
    }

    #[test]
    fn old_message_fixture_shape_is_unchanged_and_new_fields_default() {
        let message: crate::Message = serde_json::from_value(serde_json::json!({
            "role": "tool",
            "content": "legacy",
            "tool_call_id": "call-a"
        }))
        .unwrap();
        assert!(message.internal_call_id.is_none());
        assert!(message.content_blocks.is_empty());
        assert_eq!(
            serde_json::to_value(message).unwrap(),
            serde_json::json!({
                "role": "tool",
                "content": "legacy",
                "tool_call_id": "call-a"
            })
        );
    }
}
