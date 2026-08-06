use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::{Message, ModelError, ModelToolSchema, Usage};

/// Capabilities negotiated before a provider request is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_calls: bool,
    pub parallel_tool_calls: bool,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
        }
    }
}

impl ProviderCapabilities {
    pub fn validate_tools(&self, tools: &[ModelToolSchema]) -> Result<(), ModelError> {
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
}
