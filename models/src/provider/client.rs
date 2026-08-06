use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use thiserror::Error;

use crate::{
    Message, ModelClient, ModelClientId, ModelError, ModelEvent, ModelToolSchema,
    ProviderCapabilities, ProviderOptions,
};

use super::{
    ResolvedAuth, ResolvedHeader, Transport, TransportError, WireProtocol, WireRequestInput,
};

const MAX_CLIENT_NAMESPACE_BYTES: usize = 128;
const MAX_MODEL_ID_BYTES: usize = 1024;

/// Resolved data needed to assemble one provider target.
#[derive(Clone)]
pub struct ProviderClientConfig {
    pub client_namespace: String,
    pub base_url: String,
    pub model: String,
    pub auth: ResolvedAuth,
    pub headers: Vec<ResolvedHeader>,
    pub options: ProviderOptions,
    pub protocol_options: serde_json::Value,
}

pub struct ProviderClient {
    config: ProviderClientConfig,
    protocol: Arc<dyn WireProtocol>,
    transport: Arc<Transport>,
}

impl ProviderClient {
    pub fn new(
        mut config: ProviderClientConfig,
        protocol: Arc<dyn WireProtocol>,
        transport: Arc<Transport>,
    ) -> Result<Self, ProviderClientError> {
        config.client_namespace = config.client_namespace.trim().to_string();
        config.base_url = config.base_url.trim().trim_end_matches('/').to_string();
        config.model = config.model.trim().to_string();
        validate_config(&config)?;
        Transport::validate_base_url(&config.base_url)?;
        Ok(Self {
            config,
            protocol,
            transport,
        })
    }

    pub fn protocol(&self) -> &dyn WireProtocol {
        self.protocol.as_ref()
    }

    pub fn config(&self) -> &ProviderClientConfig {
        &self.config
    }
}

#[async_trait]
impl ModelClient for ProviderClient {
    fn stream(
        &self,
        messages: &[Message],
        tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        if let Err(error) = self.protocol.capabilities().validate_tools(tools) {
            return Box::pin(futures::stream::once(async move { Err(error) }));
        }
        self.transport.stream(
            &self.config.base_url,
            &self.config.auth,
            &self.config.headers,
            self.protocol.as_ref(),
            WireRequestInput {
                model: &self.config.model,
                messages,
                tools,
                options: &self.config.options,
                protocol_options: &self.config.protocol_options,
            },
        )
    }

    fn model_id(&self) -> &str {
        &self.config.model
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new(
            &self.config.client_namespace,
            &self.config.base_url,
            &self.config.model,
        )
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.protocol.capabilities()
    }

    fn requires_terminal_event(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ProviderClientError {
    #[error("provider client namespace must not be empty")]
    EmptyClientNamespace,
    #[error("provider client namespace exceeds {max} bytes")]
    ClientNamespaceTooLong { max: usize },
    #[error("provider model id must not be empty")]
    EmptyModel,
    #[error("provider model id exceeds {max} bytes")]
    ModelIdTooLong { max: usize },
    #[error(transparent)]
    Transport(#[from] TransportError),
}

fn validate_config(config: &ProviderClientConfig) -> Result<(), ProviderClientError> {
    if config.client_namespace.is_empty() {
        return Err(ProviderClientError::EmptyClientNamespace);
    }
    if config.client_namespace.len() > MAX_CLIENT_NAMESPACE_BYTES {
        return Err(ProviderClientError::ClientNamespaceTooLong {
            max: MAX_CLIENT_NAMESPACE_BYTES,
        });
    }
    if config.model.is_empty() {
        return Err(ProviderClientError::EmptyModel);
    }
    if config.model.len() > MAX_MODEL_ID_BYTES {
        return Err(ProviderClientError::ModelIdTooLong {
            max: MAX_MODEL_ID_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use reqwest::{Method, StatusCode, header::HeaderMap};

    use super::*;
    use crate::provider::{AuthStyle, Framing, StreamDecoder, WireProtocolId, WireRequest};

    struct NoopDecoder;

    impl StreamDecoder for NoopDecoder {
        fn push(&mut self, _frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
            Ok(Vec::new())
        }
    }

    struct NoopProtocol {
        id: WireProtocolId,
    }

    impl NoopProtocol {
        fn new() -> Self {
            Self {
                id: WireProtocolId::new("test/noop").unwrap(),
            }
        }
    }

    impl WireProtocol for NoopProtocol {
        fn id(&self) -> &WireProtocolId {
            &self.id
        }

        fn build_request(&self, _input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
            Ok(WireRequest {
                method: Method::POST,
                path: "stream".to_string(),
                headers: HeaderMap::new(),
                body: serde_json::json!({}),
            })
        }

        fn framing(&self) -> Framing {
            Framing::JsonLines
        }

        fn decoder(&self) -> Box<dyn StreamDecoder> {
            Box::new(NoopDecoder)
        }

        fn classify_error(
            &self,
            status: StatusCode,
            _headers: &HeaderMap,
            _body: &str,
        ) -> ModelError {
            ModelError::RequestFailed(format!("HTTP {status}"))
        }

        fn default_auth_style(&self) -> AuthStyle {
            AuthStyle::None
        }

        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                streaming: true,
                tool_calls: false,
                parallel_tool_calls: false,
            }
        }
    }

    fn config() -> ProviderClientConfig {
        ProviderClientConfig {
            client_namespace: "openai".to_string(),
            base_url: "https://example.test/v1/".to_string(),
            model: "model-a".to_string(),
            auth: ResolvedAuth::none(),
            headers: Vec::new(),
            options: ProviderOptions::default(),
            protocol_options: serde_json::json!({}),
        }
    }

    #[test]
    fn client_preserves_legacy_target_identity_while_protocol_id_is_independent() {
        let client = ProviderClient::new(
            config(),
            Arc::new(NoopProtocol::new()),
            Arc::new(Transport::new(Default::default()).unwrap()),
        )
        .unwrap();

        assert_eq!(client.model_id(), "model-a");
        assert_eq!(
            client.client_id().as_str(),
            "openai:https://example.test/v1:model-a"
        );
        assert_eq!(client.protocol().id().as_str(), "test/noop");
        assert_eq!(client.config().base_url, "https://example.test/v1");
    }

    #[tokio::test]
    async fn capability_failure_precedes_transport_dispatch() {
        let client = ProviderClient::new(
            config(),
            Arc::new(NoopProtocol::new()),
            Arc::new(Transport::new(Default::default()).unwrap()),
        )
        .unwrap();
        let mut stream = client.stream(
            &[Message::user("hello")],
            &[ModelToolSchema {
                name: "echo".to_string(),
                description: String::new(),
                parameters: serde_json::json!({"type":"object"}),
            }],
        );

        assert!(matches!(
            stream.next().await,
            Some(Err(ModelError::InvalidConfiguration(message)))
                if message.contains("does not support tool calls")
        ));
    }

    #[test]
    fn client_rejects_invalid_identity_model_and_endpoint() {
        let protocol = || Arc::new(NoopProtocol::new()) as Arc<dyn WireProtocol>;
        let transport = || Arc::new(Transport::new(Default::default()).unwrap());

        let mut invalid = config();
        invalid.client_namespace = " ".to_string();
        assert!(matches!(
            ProviderClient::new(invalid, protocol(), transport()),
            Err(ProviderClientError::EmptyClientNamespace)
        ));

        let mut invalid = config();
        invalid.model = String::new();
        assert!(matches!(
            ProviderClient::new(invalid, protocol(), transport()),
            Err(ProviderClientError::EmptyModel)
        ));

        let mut invalid = config();
        invalid.base_url = "file:///tmp/provider".to_string();
        assert!(matches!(
            ProviderClient::new(invalid, protocol(), transport()),
            Err(ProviderClientError::Transport(
                TransportError::UnsupportedEndpointScheme
            ))
        ));
    }
}
