use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::core::types::{Message, ToolSchema, Usage};
use crate::errors::ModelError;

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
}
