use async_stream::stream;
use futures::{StreamExt, stream::BoxStream};

use crate::core::types::{Message, ToolSchema};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, StreamChunk};

/// Model client that tries a primary provider, then fallback providers if the
/// active provider fails before streaming any response chunks.
pub struct RoutingModelClient {
    clients: Vec<Box<dyn ModelClient>>,
    model_id: String,
}

impl RoutingModelClient {
    pub fn new(primary: Box<dyn ModelClient>, fallbacks: Vec<Box<dyn ModelClient>>) -> Self {
        let mut clients = Vec::with_capacity(1 + fallbacks.len());
        clients.push(primary);
        clients.extend(fallbacks);
        let model_id = format!(
            "routing({})",
            clients
                .iter()
                .map(|client| client.model_id())
                .collect::<Vec<_>>()
                .join(",")
        );
        Self { clients, model_id }
    }
}

impl ModelClient for RoutingModelClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
        let messages = messages.to_vec();
        let tools = tools.to_vec();
        Box::pin(stream! {
            let mut last_error = None;
            for client in &self.clients {
                let mut chunks = client.stream(&messages, &tools);
                let mut emitted_chunk = false;
                loop {
                    match chunks.next().await {
                        Some(Ok(chunk)) => {
                            emitted_chunk = true;
                            yield Ok(chunk);
                        }
                        Some(Err(err)) => {
                            if emitted_chunk {
                                yield Err(err);
                                return;
                            }
                            last_error = Some(err);
                            break;
                        }
                        None => return,
                    }
                }
            }
            if let Some(err) = last_error {
                yield Err(err);
            }
        })
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use futures::{StreamExt, stream, stream::BoxStream};

    use crate::core::types::{Message, ToolSchema, Usage};
    use crate::errors::ModelError;
    use crate::models::routing::RoutingModelClient;
    use crate::models::traits::{ModelClient, StreamChunk};

    struct FailingClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for FailingClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::once(async {
                Err(ModelError::RequestFailed("primary unavailable".to_string()))
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    struct StaticClient {
        id: &'static str,
        response: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for StaticClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let response = self.response.to_string();
            Box::pin(stream::once(async move {
                Ok(StreamChunk {
                    delta: response,
                    usage: Some(Usage::default()),
                })
            }))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    struct PartialThenErrorClient {
        id: &'static str,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl ModelClient for PartialThenErrorClient {
        fn stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> BoxStream<'_, Result<StreamChunk, ModelError>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(stream::iter([
                Ok(StreamChunk {
                    delta: "partial".to_string(),
                    usage: None,
                }),
                Err(ModelError::StreamInterrupted(
                    "primary interrupted".to_string(),
                )),
            ]))
        }

        fn model_id(&self) -> &str {
            self.id
        }
    }

    #[tokio::test]
    async fn falls_back_when_primary_errors_before_streaming() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(FailingClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        );
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "fallback answer");
    }

    #[tokio::test]
    async fn does_not_fallback_after_primary_has_streamed_chunks() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let model = RoutingModelClient::new(
            Box::new(PartialThenErrorClient {
                id: "primary",
                calls: primary_calls.clone(),
            }),
            vec![Box::new(StaticClient {
                id: "fallback",
                response: "fallback answer",
                calls: fallback_calls.clone(),
            })],
        );
        let messages: Vec<Message> = Vec::new();
        let tools: Vec<ToolSchema> = Vec::new();

        let chunks = model.stream(&messages, &tools).collect::<Vec<_>>().await;

        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].as_ref().unwrap().delta, "partial");
        assert!(matches!(
            chunks[1],
            Err(ModelError::StreamInterrupted(ref message))
                if message == "primary interrupted"
        ));
    }
}
