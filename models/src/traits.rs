use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::{Message, ModelError, ToolSchema, Usage};

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
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>>;

    /// The model identifier (for logging/tracing).
    fn model_id(&self) -> &str;

    /// Stable provider target identity for routing health state.
    fn client_id(&self) -> ModelClientId {
        ModelClientId::opaque(self.model_id().to_string())
    }
}
