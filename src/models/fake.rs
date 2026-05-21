use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::core::types::{Message, ToolSchema, Usage};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, StreamChunk};

/// Deterministic local model for smoke tests and demos.
pub struct FakeModelClient {
    response: String,
}

impl FakeModelClient {
    pub fn new(response: String) -> Self {
        Self { response }
    }
}

#[async_trait]
impl ModelClient for FakeModelClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let response = self.response.clone();
        Box::pin(futures::stream::once(async move {
            Ok(StreamChunk {
                delta: response,
                usage: Some(Usage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                }),
            })
        }))
    }

    fn model_id(&self) -> &str {
        "fake"
    }
}
