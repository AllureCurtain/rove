use std::borrow::Cow;
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
    /// A locally verified raster image, carried as base64.
    ///
    /// The bytes come from the product attachment store, which has already
    /// sniffed the type and hashed the payload, so `mime_type` is the
    /// server-verified value and never a client claim. The base64 `data` is
    /// bounded by [`MAX_CONTENT_BYTES`] like every other content, which is
    /// what makes an oversized image a labelled degradation at the injection
    /// site instead of a provider-side rejection mid-run.
    Image {
        mime_type: String,
        data: String,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            Self::RichReference { .. } | Self::Image { .. } => None,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Text { text } => bounded("content text", text, MAX_CONTENT_BYTES),
            Self::Image { mime_type, data } => {
                bounded("image MIME type", mime_type, 128)?;
                bounded("image data", data, MAX_CONTENT_BYTES)?;
                if mime_type.trim().is_empty() {
                    return Err(ProtocolValidationError::EmptyField {
                        field: "image MIME type",
                    });
                }
                let media_type = mime_type.split(';').next().unwrap_or_default();
                let mut parts = media_type.splitn(2, '/');
                let main = parts.next().unwrap_or_default();
                let sub = parts.next().unwrap_or_default();
                if main.is_empty()
                    || sub.is_empty()
                    || !main
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
                    || !sub
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '+')
                {
                    return Err(ProtocolValidationError::InvalidField {
                        field: "image MIME type",
                        reason: "must be a concrete type/subtype pair".to_string(),
                    });
                }
                if data.trim().is_empty() {
                    return Err(ProtocolValidationError::EmptyField {
                        field: "image data",
                    });
                }
                Ok(())
            }
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    #[default]
    Ok,
    Error,
    Rejected,
    Partial,
    UnknownEffect,
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

    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    #[default]
    EndTurn,
    ToolUse,
    MaxTokens,
    ContentFilter,
    Cancelled,
    Error,
    Incomplete,
    Other(String),
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

/// Provider-independent message used between Session/Agent context and a
/// concrete provider request.  Unlike the legacy [`Message`], content blocks
/// and typed tool results retain their canonical identity until target
/// projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelMessage {
    #[serde(default = "default_canonical_schema_version")]
    pub schema_version: u16,
    pub role: Role,
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_result: Option<ToolResult>,
}

impl ModelMessage {
    pub fn text(role: Role, text: impl Into<String>) -> Self {
        Self {
            schema_version: CANONICAL_MESSAGE_SCHEMA_VERSION,
            role,
            content: vec![ContentBlock::text(text)],
            tool_calls: Vec::new(),
            tool_result: None,
        }
    }

    pub fn assistant(turn: AssistantTurn) -> Self {
        Self {
            schema_version: turn.schema_version,
            role: Role::Assistant,
            content: turn.content,
            tool_calls: turn.tool_calls,
            tool_result: None,
        }
    }

    pub fn tool(result: ToolResult) -> Self {
        Self {
            schema_version: CANONICAL_MESSAGE_SCHEMA_VERSION,
            role: Role::Tool,
            content: result.content.clone(),
            tool_calls: Vec::new(),
            tool_result: Some(result),
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
        if let Some(result) = &self.tool_result {
            result.validate()?;
        }
        Ok(())
    }
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
    #[error("{field} is invalid: {reason}")]
    InvalidField { field: &'static str, reason: String },
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

/// Project a runtime conversation into the message list a provider request may
/// carry.
///
/// Payload validity belongs to the provider boundary: every adapter under
/// `provider/protocols/` serializes whatever list it is handed, so an invalid
/// list becomes an invalid request body. The requirement the four adapters
/// share is that a projected message must carry content. Anthropic is the
/// strict one — `format_anthropic_message` serializes `Message::assistant("")`
/// as `{"role":"assistant","content":""}`, which the Messages API rejects with
/// a non-retryable 400 unless the empty turn is the optional final assistant
/// prefill.
///
/// An empty assistant turn is reachable without any provider misbehaving: the
/// kernel records the final answer even when it is empty
/// (`core/src/kernel.rs`), and silent-turn recovery then appends the nudge
/// after it (`runtime/src/engine/run_loop.rs`), while the context manager
/// appends the current user message last
/// (`runtime/src/context/manager.rs::build_with_checkpoint`). The same empty
/// turn also reaches every later run of the session, because the run's history
/// is resumable state rather than a recovery-only artifact.
///
/// Role alternation is *not* a requirement here, which is why this projection
/// does not try to restore it. `format_anthropic_message` already maps a
/// mid-conversation system message and every tool result to `user`, so the
/// runtime has always sent consecutive `user` turns: the context manager places
/// the `Message::system` runtime guidance immediately before the current user
/// message on every ordinary run, and the leading system prompt is the only
/// message `extract_system` removes. OpenAI-compatible and Ollama adapters take
/// any role order.
///
/// So the projection drops an empty assistant turn that is not the final
/// message. Dropping is preferred to repairing: the turn carries neither text
/// nor a tool call, so the provider loses nothing, whereas a synthesized marker
/// would put text into the conversation that the model never produced and that
/// no adapter could distinguish from a real answer. The dropped turn stays in
/// the durable history and in the canonical event stream; only the provider
/// request omits it. A final empty assistant turn is preserved, because
/// Anthropic accepts it as an assistant prefill.
pub fn project_provider_messages(messages: &[Message]) -> Cow<'_, [Message]> {
    let last = messages.len().saturating_sub(1);
    let dropped =
        |index: usize, message: &Message| index != last && is_empty_assistant_turn(message);
    if !messages
        .iter()
        .enumerate()
        .any(|(index, message)| dropped(index, message))
    {
        return Cow::Borrowed(messages);
    }
    Cow::Owned(
        messages
            .iter()
            .enumerate()
            .filter(|(index, message)| !dropped(*index, message))
            .map(|(_, message)| message.clone())
            .collect(),
    )
}

/// Whether a message is an assistant turn with nothing a provider can accept.
///
/// Tool-call turns are never empty: the adapters project them into a non-empty
/// content-block list, so a turn that asked for a tool must survive even when
/// its text is empty. The same holds for any content block that carries text,
/// even when the flat [`Message::content`] mirror is empty.
fn is_empty_assistant_turn(message: &Message) -> bool {
    message.role == Role::Assistant
        && message.tool_calls.is_empty()
        && message.content.trim().is_empty()
        && message
            .content_blocks
            .iter()
            .all(|block| block.text_value().is_none_or(|text| text.trim().is_empty()))
}

/// The conversation a silent-turn recovery request actually carries.
///
/// Shared by the four adapter tests so each one serializes the same shape and
/// asserts its own body rules: prior work with a tool call, the silent turn the
/// kernel recorded, the recovery nudge, and the current user message.
#[cfg(test)]
pub(crate) fn recovery_shaped_conversation() -> Vec<Message> {
    vec![
        Message::system("You are a test agent.".to_string()),
        Message::user("inspect the repository".to_string()),
        Message::assistant_with_tool_calls(
            String::new(),
            vec![ToolCallRef {
                id: "toolu_1".to_string(),
                name: "read_file".to_string(),
                args: serde_json::json!({"path": "Cargo.toml"}),
            }],
        ),
        Message::tool("contents".to_string(), Some("toolu_1".to_string())),
        // The silent turn: the kernel recorded an empty final answer.
        Message::assistant(String::new()),
        // The recovery nudge the run loop appends after it.
        Message::system("Your previous turn produced no visible response. Continue.".to_string()),
        // The context manager appends the current user message last.
        Message::user("inspect the repository".to_string()),
    ]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_content_is_bounded_and_typed() {
        let valid = ContentBlock::Image {
            mime_type: "image/png".to_string(),
            data: "aW1hZ2U=".to_string(),
        };
        valid.validate().unwrap();

        // The MIME type must be a concrete type/subtype pair.
        for mime_type in [
            "",
            "image",
            "image/",
            "/png",
            "image/;charset=x",
            "not a mime!",
        ] {
            let invalid = ContentBlock::Image {
                mime_type: mime_type.to_string(),
                data: "aW1hZ2U=".to_string(),
            };
            assert!(
                invalid.validate().is_err(),
                "{mime_type} must not validate as an image MIME type"
            );
        }
        // The payload must be present and inside the per-request content bound.
        let empty = ContentBlock::Image {
            mime_type: "image/png".to_string(),
            data: "  ".to_string(),
        };
        assert!(empty.validate().is_err());
        let oversized = ContentBlock::Image {
            mime_type: "image/png".to_string(),
            data: "x".repeat(MAX_CONTENT_BYTES + 1),
        };
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn image_blocks_serialise_additively_and_legacy_payloads_still_parse() {
        // A pre-image canonical payload with no content blocks parses as before.
        let legacy: Message = serde_json::from_str(r#"{"role":"user","content":"hello"}"#).unwrap();
        assert!(legacy.content_blocks.is_empty());
        // An image block round-trips through the tagged representation.
        let image = ContentBlock::Image {
            mime_type: "image/png".to_string(),
            data: "aW1hZ2U=".to_string(),
        };
        let json = serde_json::to_value(&image).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(
            <ContentBlock as serde::Deserialize>::deserialize(json).unwrap(),
            image
        );
    }

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

    #[test]
    fn an_empty_non_final_assistant_turn_is_dropped_from_the_projection() {
        let messages = recovery_shaped_conversation();
        let projected = project_provider_messages(&messages);

        assert_eq!(
            projected.len(),
            messages.len() - 1,
            "exactly the empty silent turn is dropped"
        );
        assert!(
            !projected
                .iter()
                .any(|message| message.role == Role::Assistant
                    && message.tool_calls.is_empty()
                    && message.content.is_empty()),
            "no empty assistant turn may survive the projection: {projected:?}"
        );
        assert!(
            projected.iter().any(|message| message
                .tool_calls
                .iter()
                .any(|call| call.name == "read_file")),
            "a tool-call turn survives even though its text is empty"
        );
        assert!(
            projected.iter().any(|message| message.content
                == "Your previous turn produced no visible response. Continue."),
            "the recovery nudge must still reach the provider"
        );
    }

    #[test]
    fn a_conversation_without_an_empty_turn_is_projected_unchanged() {
        let messages = vec![
            Message::system("You are a test agent.".to_string()),
            Message::user("inspect".to_string()),
            Message::assistant("an answer".to_string()),
        ];
        assert!(
            matches!(project_provider_messages(&messages), Cow::Borrowed(_)),
            "a valid conversation is borrowed, so existing bodies stay byte-identical"
        );
    }

    #[test]
    fn a_final_empty_assistant_turn_is_kept_as_an_optional_prefill() {
        let messages = vec![
            Message::system("You are a test agent.".to_string()),
            Message::user("inspect".to_string()),
            Message::assistant(String::new()),
        ];
        let projected = project_provider_messages(&messages);
        assert_eq!(projected.len(), 3);
        assert!(projected[2].content.is_empty());
    }

    #[test]
    fn whitespace_only_and_rich_reference_only_turns_count_as_empty() {
        let messages = vec![
            Message::user("inspect".to_string()),
            Message::assistant("  \n ".to_string()),
            Message::user("again".to_string()),
        ];
        assert_eq!(project_provider_messages(&messages).len(), 2);

        let mut reference_only = Message::assistant(String::new());
        reference_only.content_blocks = vec![ContentBlock::RichReference {
            kind: "tool_artifact".to_string(),
            reference: "artifact-1".to_string(),
            mime_type: None,
            title: None,
        }];
        let messages = vec![
            Message::user("inspect".to_string()),
            reference_only,
            Message::user("again".to_string()),
        ];
        assert_eq!(
            project_provider_messages(&messages).len(),
            2,
            "an assistant turn whose only block is a reference is still empty on the wire"
        );
    }
}
