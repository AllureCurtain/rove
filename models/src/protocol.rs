use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Version of the provider-neutral message contracts.
///
/// The legacy [`Message`] JSON shape remains readable and writable.  New
/// typed values carry this version so persisted session projections can be
/// upgraded without treating a provider request body as canonical state.
pub const CANONICAL_MESSAGE_SCHEMA_VERSION: u16 = 1;
pub const MAX_CONTENT_BLOCKS: usize = 128;
pub const MAX_CONTENT_BYTES: usize = 1024 * 1024;
pub const MAX_TOOL_CALLS: usize = 128;
pub const MAX_TOOL_ID_BYTES: usize = 256;
pub const MAX_TOOL_NAME_BYTES: usize = 256;
pub const MAX_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;

/// Provider-neutral content in an assistant or tool result.
///
/// Rich content is deliberately a bounded reference.  The model layer does
/// not fetch a URI or persist a remote payload as part of request projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    RichReference {
        kind: String,
        reference: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::RichReference { .. } => None,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Text { text } => bounded("content text", text, MAX_CONTENT_BYTES),
            Self::RichReference {
                kind,
                reference,
                mime_type,
                title,
            } => {
                bounded("rich content kind", kind, 128)?;
                bounded("rich content reference", reference, 4096)?;
                if let Some(mime_type) = mime_type {
                    bounded("rich content MIME type", mime_type, 128)?;
                }
                if let Some(title) = title {
                    bounded("rich content title", title, 256)?;
                }
                if reference.trim().is_empty() {
                    return Err(ProtocolValidationError::EmptyField {
                        field: "rich content reference",
                    });
                }
                Ok(())
            }
        }
    }
}

/// Stable Rove identity for a tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, Ord, PartialOrd)]
#[serde(transparent)]
pub struct InternalCallId(String);

impl InternalCallId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolValidationError> {
        let value = value.into();
        bounded("internal call id", &value, MAX_TOOL_ID_BYTES)?;
        if value.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField {
                field: "internal call id",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for InternalCallId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Provider/protocol-bound call identity.  It is never used as a Runtime
/// approval or artifact identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WireCallReference {
    pub protocol: String,
    pub value: String,
}

impl WireCallReference {
    pub fn new(
        protocol: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, ProtocolValidationError> {
        let protocol = protocol.into();
        let value = value.into();
        bounded("wire protocol", &protocol, 128)?;
        bounded("wire call id", &value, MAX_TOOL_ID_BYTES)?;
        if protocol.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField {
                field: "wire protocol",
            });
        }
        if value.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField {
                field: "wire call id",
            });
        }
        Ok(Self { protocol, value })
    }
}

/// Normalized tool call emitted by a provider stream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub internal_call_id: InternalCallId,
    pub name: String,
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_reference: Option<WireCallReference>,
}

impl ToolCall {
    pub fn new(
        internal_call_id: InternalCallId,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            internal_call_id,
            name: name.into(),
            arguments,
            wire_reference: None,
        }
    }

    fn validate(&self) -> Result<(), ProtocolValidationError> {
        bounded("tool name", &self.name, MAX_TOOL_NAME_BYTES)?;
        if self.name.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField { field: "tool name" });
        }
        let encoded = serde_json::to_vec(&self.arguments).map_err(|_| {
            ProtocolValidationError::InvalidArguments {
                reason: "arguments cannot be serialized".to_string(),
            }
        })?;
        if encoded.len() > MAX_TOOL_ARGUMENT_BYTES {
            return Err(ProtocolValidationError::TooLarge {
                field: "tool arguments",
                max: MAX_TOOL_ARGUMENT_BYTES,
            });
        }
        if !self.arguments.is_object() {
            return Err(ProtocolValidationError::InvalidArguments {
                reason: "tool arguments must be a JSON object".to_string(),
            });
        }
        if let Some(wire_reference) = &self.wire_reference {
            WireCallReference::new(
                wire_reference.protocol.clone(),
                wire_reference.value.clone(),
            )?;
        }
        Ok(())
    }
}

/// Normalized outcome of one tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Ok,
    Error,
    Rejected,
    Partial,
    UnknownEffect,
}

impl Default for ToolResultStatus {
    fn default() -> Self {
        Self::Ok
    }
}

/// Provider-neutral tool result.  Status is retained even when the target
/// wire protocol only has a text result field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolResult {
    pub internal_call_id: InternalCallId,
    pub tool_name: String,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub status: ToolResultStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

impl ToolResult {
    pub fn text(
        internal_call_id: InternalCallId,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            internal_call_id,
            tool_name: tool_name.into(),
            content: vec![ContentBlock::text(content)],
            status: ToolResultStatus::Ok,
            error_code: None,
        }
    }

    fn validate(&self) -> Result<(), ProtocolValidationError> {
        bounded("tool result name", &self.tool_name, MAX_TOOL_NAME_BYTES)?;
        if self.tool_name.trim().is_empty() {
            return Err(ProtocolValidationError::EmptyField {
                field: "tool result name",
            });
        }
        validate_content(&self.content)?;
        if let Some(error_code) = &self.error_code {
            bounded("tool result error code", error_code, 128)?;
        }
        Ok(())
    }
}

/// Normalized provider stop state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    ContentFilter,
    Cancelled,
    Error,
    Incomplete,
    Other(String),
}

impl Default for StopReason {
    fn default() -> Self {
        Self::EndTurn
    }
}

/// Safe provider provenance.  Wire payloads, signatures, and headers remain
/// private to the provider adapter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnProvenance {
    pub model: String,
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
}

/// Normalized assistant result consumed by Core and Runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistantTurn {
    #[serde(default = "default_canonical_schema_version")]
    pub schema_version: u16,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub usage: Usage,
    #[serde(default)]
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<TurnProvenance>,
}

fn default_canonical_schema_version() -> u16 {
    CANONICAL_MESSAGE_SCHEMA_VERSION
}

impl Default for AssistantTurn {
    fn default() -> Self {
        Self {
            schema_version: CANONICAL_MESSAGE_SCHEMA_VERSION,
            content: Vec::new(),
            tool_calls: Vec::new(),
            usage: Usage::default(),
            stop_reason: StopReason::EndTurn,
            provenance: None,
        }
    }
}

impl AssistantTurn {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::text(text)],
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if self.schema_version == 0 || self.schema_version > CANONICAL_MESSAGE_SCHEMA_VERSION {
            return Err(ProtocolValidationError::UnsupportedVersion {
                version: self.schema_version,
            });
        }
        validate_content(&self.content)?;
        if self.tool_calls.len() > MAX_TOOL_CALLS {
            return Err(ProtocolValidationError::TooMany {
                field: "tool calls",
                max: MAX_TOOL_CALLS,
            });
        }
        let mut ids = BTreeSet::new();
        for call in &self.tool_calls {
            call.validate()?;
            if !ids.insert(call.internal_call_id.clone()) {
                return Err(ProtocolValidationError::DuplicateId {
                    id: call.internal_call_id.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// Errors raised before a model turn can reach tool policy/execution.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ProtocolValidationError {
    #[error("{field} must not be empty")]
    EmptyField { field: &'static str },
    #[error("{field} exceeds {max} bytes")]
    TooLarge { field: &'static str, max: usize },
    #[error("too many {field}; maximum is {max}")]
    TooMany { field: &'static str, max: usize },
    #[error("duplicate internal call id `{id}`")]
    DuplicateId { id: String },
    #[error("invalid tool arguments: {reason}")]
    InvalidArguments { reason: String },
    #[error("unsupported canonical message schema version {version}")]
    UnsupportedVersion { version: u16 },
    #[error("stream ended before a complete terminal turn")]
    IncompleteTurn,
    #[error("tool call `{id}` was not started")]
    UnknownCall { id: String },
    #[error("tool call `{id}` was completed more than once")]
    DuplicateCompletion { id: String },
    #[error("tool call `{id}` has no completed arguments")]
    IncompleteCall { id: String },
}

fn bounded(field: &'static str, value: &str, max: usize) -> Result<(), ProtocolValidationError> {
    if value.len() > max {
        return Err(ProtocolValidationError::TooLarge { field, max });
    }
    Ok(())
}

fn validate_content(content: &[ContentBlock]) -> Result<(), ProtocolValidationError> {
    if content.len() > MAX_CONTENT_BLOCKS {
        return Err(ProtocolValidationError::TooMany {
            field: "content blocks",
            max: MAX_CONTENT_BLOCKS,
        });
    }
    let mut bytes = 0usize;
    for block in content {
        block.validate()?;
        bytes = bytes.saturating_add(serde_json::to_vec(block).unwrap_or_default().len());
        if bytes > MAX_CONTENT_BYTES {
            return Err(ProtocolValidationError::TooLarge {
                field: "content",
                max: MAX_CONTENT_BYTES,
            });
        }
    }
    Ok(())
}

/// A provider-neutral message in model conversation history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Additive canonical identity fields.  They are omitted for legacy
    /// messages, preserving existing artifact and wire JSON exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internal_call_id: Option<InternalCallId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result_status: Option<ToolResultStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<ContentBlock>,
}

/// A tool call reference recorded on an assistant message for provider replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallRef {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            internal_call_id: None,
            tool_name: None,
            tool_result_status: None,
            content_blocks: Vec::new(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            internal_call_id: None,
            tool_name: None,
            tool_result_status: None,
            content_blocks: Vec::new(),
        }
    }

    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCallRef>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            internal_call_id: None,
            tool_name: None,
            tool_result_status: None,
            content_blocks: Vec::new(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            internal_call_id: None,
            tool_name: None,
            tool_result_status: None,
            content_blocks: Vec::new(),
        }
    }

    pub fn tool(content: impl Into<String>, tool_call_id: Option<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id,
            internal_call_id: None,
            tool_name: None,
            tool_result_status: None,
            content_blocks: Vec::new(),
        }
    }

    pub fn tool_with_status(
        content: impl Into<String>,
        tool_call_id: Option<String>,
        internal_call_id: Option<InternalCallId>,
        tool_name: Option<String>,
        status: ToolResultStatus,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id,
            internal_call_id,
            tool_name,
            tool_result_status: Some(status),
            content_blocks: Vec::new(),
        }
    }
}

/// Provider-neutral message role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// Token usage from a single model call.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
    #[serde(default)]
    pub cached_tokens: u32,
}

/// Provider-neutral tool schema sent to a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_turn_has_stable_defaults_and_schema_version() {
        let turn = AssistantTurn::default();
        assert_eq!(turn.schema_version, CANONICAL_MESSAGE_SCHEMA_VERSION);
        assert_eq!(turn.stop_reason, StopReason::EndTurn);
        assert_eq!(
            serde_json::from_value::<AssistantTurn>(serde_json::json!({})).unwrap(),
            turn
        );
    }

    #[test]
    fn typed_call_and_result_keep_distinct_internal_and_wire_identity() {
        let internal = InternalCallId::new("run-call-1").unwrap();
        let wire = WireCallReference::new("openai-completions", "call_1").unwrap();
        let call = ToolCall {
            internal_call_id: internal.clone(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"message":"ok"}),
            wire_reference: Some(wire),
        };
        let result = ToolResult::text(internal.clone(), "echo", "ok");
        assert_ne!(call.internal_call_id.to_string(), "call_1");
        assert_eq!(result.internal_call_id, internal);
        assert!(call.validate().is_ok());
        assert!(result.validate().is_ok());
    }

    #[test]
    fn validation_rejects_empty_duplicate_and_non_object_calls() {
        assert!(matches!(
            InternalCallId::new(" "),
            Err(ProtocolValidationError::EmptyField { .. })
        ));
        let id = InternalCallId::new("same").unwrap();
        let turn = AssistantTurn {
            tool_calls: vec![
                ToolCall::new(id.clone(), "echo", serde_json::json!({})),
                ToolCall::new(id, "echo", serde_json::json!({})),
            ],
            ..AssistantTurn::default()
        };
        assert!(matches!(
            turn.validate(),
            Err(ProtocolValidationError::DuplicateId { .. })
        ));
        let turn = AssistantTurn {
            tool_calls: vec![ToolCall::new(
                InternalCallId::new("bad-args").unwrap(),
                "echo",
                serde_json::json!("not an object"),
            )],
            ..AssistantTurn::default()
        };
        assert!(matches!(
            turn.validate(),
            Err(ProtocolValidationError::InvalidArguments { .. })
        ));
    }
}
