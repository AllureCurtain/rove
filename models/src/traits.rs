use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::{
    AssistantTurn, Message, ModelError, ModelToolSchema, StopReason, Usage, validate_model_tools,
};

/// Capabilities negotiated before a provider request is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_calls: bool,
    pub parallel_tool_calls: bool,
    /// Whether the protocol can carry an image content block natively.
    ///
    /// Default `false`, so any client that has not been taught to project
    /// images declares itself incapable and the request fails closed before
    /// dispatch, rather than emitting a payload the provider would reject.
    pub images: bool,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            images: false,
        }
    }
}

impl ProviderCapabilities {
    pub fn validate_tools(&self, tools: &[ModelToolSchema]) -> Result<(), ModelError> {
        validate_model_tools(tools)
            .map_err(|error| ModelError::InvalidConfiguration(error.to_string()))?;
        if !tools.is_empty() && !self.tool_calls {
            return Err(ModelError::InvalidConfiguration(
                "selected provider does not support tool calls".to_string(),
            ));
        }
        if !self.streaming {
            return Err(ModelError::InvalidConfiguration(
                "selected provider does not support streaming".to_string(),
            ));
        }
        Ok(())
    }

    /// Fail closed on image content before dispatch, following the
    /// [`ProviderCapabilities::validate_tools`] precedent: a capability the
    /// client does not declare is a typed configuration error, never a
    /// payload the provider discovers mid-request.
    pub fn validate_messages(&self, messages: &[Message]) -> Result<(), ModelError> {
        if self.images {
            return Ok(());
        }
        let carries_image = messages.iter().any(|message| {
            message
                .content_blocks
                .iter()
                .any(|block| matches!(block, crate::ContentBlock::Image { .. }))
        });
        if carries_image {
            return Err(ModelError::InvalidConfiguration(
                "selected provider does not support image input".to_string(),
            ));
        }
        Ok(())
    }

    pub fn validate_assistant_turn(&self, turn: &AssistantTurn) -> Result<(), ModelError> {
        if !turn.tool_calls.is_empty() && !self.tool_calls {
            return Err(ModelError::InvalidConfiguration(
                "provider returned tool calls but does not declare tool-call support".to_string(),
            ));
        }
        if turn.tool_calls.len() > 1 && !self.parallel_tool_calls {
            return Err(ModelError::InvalidConfiguration(
                "provider returned parallel tool calls but does not declare parallel support"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelClientId(String);

impl ModelClientId {
    pub fn new(provider: &str, endpoint: impl AsRef<str>, model: impl AsRef<str>) -> Self {
        let endpoint = endpoint.as_ref().trim_end_matches('/');
        let model = model.as_ref();
        Self(format!("{provider}:{endpoint}:{model}"))
    }

    pub fn opaque(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ModelClientId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A normalized event from a streaming LLM response.
///
/// Provider adapters translate OpenAI/Anthropic/Ollama-specific stream frames
/// into this event model before the engine consumes them.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelEvent {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        text: String,
    },
    ToolUseStart {
        id: String,
        name: String,
    },
    ToolUseDelta {
        id: String,
        args_delta: String,
    },
    ToolUseDone {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    Usage {
        usage: Usage,
    },
    StopReason {
        reason: StopReason,
    },
    Done,
}

/// Trait for LLM model clients.
///
/// Implementations wrap specific providers (OpenAI, Anthropic, Ollama).
/// The engine only interacts with models through this trait.
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// Stream a completion from the model.
    ///
    /// Returns a stream of chunks. The final chunk may contain usage info.
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>>;

    /// The model identifier (for logging/tracing).
    fn model_id(&self) -> &str;

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    /// Protocol namespace used when a canonical session is projected into a
    /// request history. The default preserves source compatibility for custom
    /// clients and intentionally avoids reusing a provider wire id.
    fn history_protocol(&self) -> String {
        "legacy".to_string()
    }

    /// Explicit opt-in for legacy clients which encode tool calls as JSON
    /// assistant text. Native wire protocols must leave this disabled.
    fn compatibility_text_tool_calls(&self) -> bool {
        false
    }

    /// Whether this client participates in the explicit terminal-event
    /// contract introduced by the typed turn boundary.
    ///
    /// The default keeps existing embedded clients compatible: their stream
    /// EOF is treated as the legacy terminal marker. First-party provider
    /// clients and the shared Fake client opt in so a truncated stream is
    /// rejected before a tool action can be created.
    fn requires_terminal_event(&self) -> bool {
        false
    }

    /// Stable provider target identity for routing health state.
    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque(self.model_id().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderCapabilities;
    use crate::ModelToolSchema;

    #[test]
    fn capability_failure_is_deterministic_before_request_projection() {
        let capabilities = ProviderCapabilities {
            streaming: true,
            tool_calls: false,
            parallel_tool_calls: false,
            images: false,
        };
        let error = capabilities
            .validate_tools(&[ModelToolSchema {
                name: "echo".to_string(),
                description: String::new(),
                parameters: serde_json::json!({"type":"object"}),
            }])
            .unwrap_err();
        assert!(matches!(error, crate::ModelError::InvalidConfiguration(_)));
    }

    #[test]
    fn image_content_fails_closed_on_a_client_that_cannot_project_it() {
        use crate::{ContentBlock, Message};
        let image = ContentBlock::Image {
            mime_type: "image/png".to_string(),
            data: "aW1hZ2U=".to_string(),
        };
        let mut message = Message::user("look at this".to_string());
        message.content_blocks = vec![image];

        let incapable = ProviderCapabilities {
            images: false,
            ..ProviderCapabilities::default()
        };
        let error = incapable.validate_messages(&[message.clone()]).unwrap_err();
        assert!(matches!(error, crate::ModelError::InvalidConfiguration(_)));

        // The same request is accepted the moment the client declares support,
        // and a text-only history never trips the check.
        let capable = ProviderCapabilities {
            images: true,
            ..ProviderCapabilities::default()
        };
        capable.validate_messages(&[message]).unwrap();
        capable
            .validate_messages(&[Message::user("text only".to_string())])
            .unwrap();
    }
}
