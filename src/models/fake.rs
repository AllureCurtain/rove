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
        messages: &[Message],
        _tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let response = if messages
            .first()
            .map(|message| message.content.contains("You are the planner for rove."))
            .unwrap_or(false)
        {
            serde_json::json!({
                "goal": messages
                    .get(1)
                    .map(|message| message.content.trim_start_matches("Goal: ").to_string())
                    .unwrap_or_else(|| "fake goal".to_string()),
                "steps": [
                    { "id": "1", "title": "answer the request" }
                ]
            })
            .to_string()
        } else {
            self.response.clone()
        };
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
