use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::core::types::{Message, ToolSchema, Usage};
use crate::errors::ModelError;

/// A chunk from the streaming LLM response.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    pub delta: String,
    /// Set on the final chunk when usage info is available.
    pub usage: Option<Usage>,
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
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>>;

    /// The model identifier (for logging/tracing).
    fn model_id(&self) -> &str;
}
