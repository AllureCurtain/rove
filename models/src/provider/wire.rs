use reqwest::{
    Method, StatusCode,
    header::{HeaderMap, HeaderName},
};

use crate::{Message, ModelError, ModelEvent, ModelToolSchema, ProviderOptions};

use super::WireProtocolId;

/// The transport framing used by a native streaming response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    ServerSentEvents,
    JsonLines,
}

/// How a resolved secret is attached to an HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStyle {
    None,
    Bearer,
    Header(HeaderName),
}

/// Neutral input needed to build one native provider request.
#[derive(Debug, Clone, Copy)]
pub struct WireRequestInput<'a> {
    pub model: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ModelToolSchema],
    pub options: &'a ProviderOptions,
    pub protocol_options: &'a serde_json::Value,
}

/// Side-effect-free description of one native provider request.
#[derive(Debug, Clone)]
pub struct WireRequest {
    pub method: Method,
    pub path: String,
    pub headers: HeaderMap,
    pub body: serde_json::Value,
}

/// Per-request stream state machine for one wire protocol.
pub trait StreamDecoder: Send {
    fn push(&mut self, frame: &str) -> Result<Vec<ModelEvent>, ModelError>;

    fn finish(&mut self) -> Result<Vec<ModelEvent>, ModelError> {
        Ok(Vec::new())
    }
}

/// Translation strategy between Rove's neutral model contract and one native
/// wire protocol.
pub trait WireProtocol: Send + Sync {
    fn id(&self) -> &WireProtocolId;

    fn build_request(&self, input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError>;

    fn framing(&self) -> Framing;

    fn decoder(&self) -> Box<dyn StreamDecoder>;

    fn classify_error(&self, status: StatusCode, headers: &HeaderMap, body: &str) -> ModelError;

    fn default_auth_style(&self) -> AuthStyle;
}
