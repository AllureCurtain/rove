use futures::stream::BoxStream;
use futures::{StreamExt, stream};

use crate::core::types::{Message, ToolSchema};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, ModelClientId, ModelEvent};

/// OpenAI Responses API model client.
pub struct OpenAiResponsesClient {
    api_base: String,
    api_key: String,
    model: String,
}

impl OpenAiResponsesClient {
    pub fn new(api_base: String, api_key: String, model: String) -> Self {
        Self {
            api_base,
            api_key,
            model,
        }
    }
}

impl ModelClient for OpenAiResponsesClient {
    fn stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        let _api_key_is_configured = !self.api_key.trim().is_empty();
        stream::once(async {
            Err(ModelError::RequestFailed(
                "OpenAI Responses provider adapter is not implemented yet".to_string(),
            ))
        })
        .boxed()
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("openai-responses", &self.api_base, &self.model)
    }
}
