use std::{collections::HashSet, time::Duration};

use async_stream::stream;
use futures::{StreamExt, stream::BoxStream};
use reqwest::{
    Url,
    header::{
        AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, HeaderMap, HeaderName, TRANSFER_ENCODING,
    },
};
use thiserror::Error;
use tokio::time::timeout;

use crate::{ModelError, ModelEvent};

use super::{
    AuthStyle, FrameBuffer, FramingLimits, ResolvedAuth, ResolvedHeader, WireProtocol, WireRequest,
    WireRequestInput,
};

const ERROR_TRUNCATION_MARKER: &str = "...[truncated]";
const MAX_ERROR_BODY_BYTES: usize = 1024 * 1024;
const MAX_REDIRECTS: usize = 10;
const MAX_REQUEST_PATH_BYTES: usize = 4096;

/// Redirect behavior is explicit so a provider cannot silently move a secret
/// to an untrusted host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectPolicy {
    None,
    Limited(usize),
}

#[derive(Debug, Clone)]
pub struct TransportConfig {
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub stream_idle_timeout: Duration,
    pub max_error_body_bytes: usize,
    pub framing_limits: FramingLimits,
    pub redirect_policy: RedirectPolicy,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(120),
            // Reasoning-capable compatible providers can pause between the
            // tool name and its argument deltas while still completing within
            // the bounded request deadline.
            stream_idle_timeout: Duration::from_secs(90),
            max_error_body_bytes: 64 * 1024,
            framing_limits: FramingLimits::default(),
            redirect_policy: RedirectPolicy::None,
        }
    }
}

pub struct Transport {
    client: reqwest::Client,
    config: TransportConfig,
}

impl Transport {
    pub fn new(config: TransportConfig) -> Result<Self, TransportError> {
        validate_config(&config)?;
        let redirect = match config.redirect_policy {
            RedirectPolicy::None => reqwest::redirect::Policy::none(),
            RedirectPolicy::Limited(limit) => reqwest::redirect::Policy::limited(limit),
        };
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .redirect(redirect)
            .build()
            .map_err(|_| TransportError::ClientBuild)?;
        Ok(Self { client, config })
    }

    pub fn config(&self) -> &TransportConfig {
        &self.config
    }

    pub fn validate_base_url(base_url: &str) -> Result<(), TransportError> {
        parse_base_url(base_url).map(|_| ())
    }

    /// Drive one protocol strategy over the shared HTTP/framing path.
    pub fn stream<'a>(
        &'a self,
        base_url: &str,
        auth: &ResolvedAuth,
        extra_headers: &[ResolvedHeader],
        protocol: &'a dyn WireProtocol,
        input: WireRequestInput<'_>,
    ) -> BoxStream<'a, Result<ModelEvent, ModelError>> {
        let setup = match protocol.build_request(&input) {
            Ok(wire_request) => self
                .prepare_request(base_url, auth, extra_headers, &wire_request)
                .map(|(url, headers)| (wire_request, url, headers))
                .map_err(|error| ModelError::InvalidConfiguration(error.to_string())),
            Err(error) => Err(error),
        };
        let auth = auth.clone();
        let extra_headers = extra_headers.to_vec();

        Box::pin(stream! {
            let (wire_request, url, headers) = match setup {
                Ok(prepared) => prepared,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };

            let response = match self
                .client
                .request(wire_request.method, url)
                .headers(headers)
                .json(&wire_request.body)
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    yield Err(map_request_error(&error));
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let response_headers = response.headers().clone();
                let body = read_bounded_body(
                    response,
                    self.config.max_error_body_bytes,
                    self.config.stream_idle_timeout,
                )
                .await;
                let body = redact_and_bound_body(&body, self.config.max_error_body_bytes, &auth, &extra_headers);
                yield Err(protocol.classify_error(status, &response_headers, &body));
                return;
            }

            let mut byte_stream = response.bytes_stream();
            let mut framer = FrameBuffer::with_limits(protocol.framing(), self.config.framing_limits);
            let mut decoder = protocol.decoder();

            let mut timed_out = false;
            loop {
                let next = match timeout(self.config.stream_idle_timeout, byte_stream.next()).await {
                    Ok(next) => next,
                    Err(_) => {
                        timed_out = true;
                        break;
                    }
                };
                let Some(chunk_result) = next else {
                    break;
                };
                let chunk = match chunk_result {
                    Ok(chunk) => chunk,
                    Err(_) => {
                        yield Err(ModelError::StreamInterrupted("provider stream read failed".to_string()));
                        return;
                    }
                };
                let frames = match framer.push(&chunk) {
                    Ok(frames) => frames,
                    Err(error) => {
                        yield Err(ModelError::StreamInterrupted(format!("provider stream framing failed: {error}")));
                        return;
                    }
                };
                for frame in frames {
                    let events = match decoder.push(&frame) {
                        Ok(events) => events,
                        Err(error) => {
                            yield Err(error);
                            return;
                        }
                    };
                    for event in events {
                        let done = matches!(event, ModelEvent::Done);
                        yield Ok(event);
                        if done {
                            return;
                        }
                    }
                }
            }

            if timed_out {
                yield Err(ModelError::StreamInterrupted("provider stream idle timeout".to_string()));
                return;
            }

            let frames = match framer.finish() {
                Ok(frames) => frames,
                Err(error) => {
                    yield Err(ModelError::StreamInterrupted(format!("provider stream framing failed: {error}")));
                    return;
                }
            };
            for frame in frames {
                let events = match decoder.push(&frame) {
                    Ok(events) => events,
                    Err(error) => {
                        yield Err(error);
                        return;
                    }
                };
                for event in events {
                    let done = matches!(event, ModelEvent::Done);
                    yield Ok(event);
                    if done {
                        return;
                    }
                }
            }
            let events = match decoder.finish() {
                Ok(events) => events,
                Err(error) => {
                    yield Err(error);
                    return;
                }
            };
            for event in events {
                yield Ok(event);
            }
        })
    }

    fn prepare_request(
        &self,
        base_url: &str,
        auth: &ResolvedAuth,
        extra_headers: &[ResolvedHeader],
        wire_request: &WireRequest,
    ) -> Result<(Url, HeaderMap), TransportError> {
        let url = resolve_endpoint(base_url, &wire_request.path)?;
        let mut headers = HeaderMap::new();
        let mut seen = HashSet::new();

        for (name, value) in &wire_request.headers {
            validate_transport_header(name)?;
            if !seen.insert(name.clone()) {
                return Err(TransportError::DuplicateHeader {
                    name: name.as_str().to_string(),
                });
            }
            headers.insert(name.clone(), value.clone());
        }

        for header in extra_headers {
            validate_transport_header(header.name())?;
            if !seen.insert(header.name().clone()) {
                return Err(TransportError::DuplicateHeader {
                    name: header.name().as_str().to_string(),
                });
            }
            let value = reqwest::header::HeaderValue::from_str(header.value()).map_err(|_| {
                TransportError::InvalidHeaderValue {
                    name: header.name().as_str().to_string(),
                }
            })?;
            headers.insert(header.name().clone(), value);
        }

        match auth.style() {
            AuthStyle::None => {}
            AuthStyle::Bearer => {
                ensure_auth_header_available(&seen, &AUTHORIZATION)?;
                let secret = auth.secret().ok_or(TransportError::MissingSecret)?;
                let value = reqwest::header::HeaderValue::from_str(&format!("Bearer {secret}"))
                    .map_err(|_| TransportError::InvalidAuthHeader)?;
                headers.insert(AUTHORIZATION, value);
            }
            AuthStyle::Header(name) => {
                validate_transport_auth_header(name)?;
                ensure_auth_header_available(&seen, name)?;
                let secret = auth.secret().ok_or(TransportError::MissingSecret)?;
                let value = reqwest::header::HeaderValue::from_str(secret)
                    .map_err(|_| TransportError::InvalidAuthHeader)?;
                headers.insert(name.clone(), value);
            }
        }

        Ok((url, headers))
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("transport configuration value '{field}' must be greater than zero")]
    InvalidConfig { field: &'static str },
    #[error("failed to build the provider HTTP client")]
    ClientBuild,
    #[error("provider endpoint URL is invalid")]
    InvalidEndpoint,
    #[error("provider endpoint URL must use http or https")]
    UnsupportedEndpointScheme,
    #[error("provider endpoint URL must not contain credentials")]
    EndpointCredentials,
    #[error("provider endpoint URL must not contain a query or fragment")]
    EndpointQueryOrFragment,
    #[error("provider endpoint URL must contain a host")]
    EndpointHostMissing,
    #[error(
        "provider request path must be relative and must not contain a query, fragment, or parent segment"
    )]
    InvalidRequestPath,
    #[error("provider request header '{name}' is managed by transport")]
    ManagedHeader { name: String },
    #[error("provider request header '{name}' is specified more than once")]
    DuplicateHeader { name: String },
    #[error("provider request header '{name}' contains an invalid value")]
    InvalidHeaderValue { name: String },
    #[error("provider authentication secret is missing")]
    MissingSecret,
    #[error("provider authentication header value is invalid")]
    InvalidAuthHeader,
}

fn validate_config(config: &TransportConfig) -> Result<(), TransportError> {
    for (field, value) in [
        ("connect_timeout", config.connect_timeout),
        ("request_timeout", config.request_timeout),
        ("stream_idle_timeout", config.stream_idle_timeout),
    ] {
        if value.is_zero() {
            return Err(TransportError::InvalidConfig { field });
        }
    }
    if config.max_error_body_bytes == 0 || config.max_error_body_bytes > MAX_ERROR_BODY_BYTES {
        return Err(TransportError::InvalidConfig {
            field: "max_error_body_bytes",
        });
    }
    if let RedirectPolicy::Limited(limit) = config.redirect_policy
        && (limit == 0 || limit > MAX_REDIRECTS)
    {
        return Err(TransportError::InvalidConfig {
            field: "redirect_limit",
        });
    }
    Ok(())
}

fn resolve_endpoint(base_url: &str, request_path: &str) -> Result<Url, TransportError> {
    let mut base = parse_base_url(base_url)?;
    if request_path.trim().is_empty()
        || request_path.len() > MAX_REQUEST_PATH_BYTES
        || request_path.starts_with("//")
        || request_path.contains("://")
        || request_path.contains('?')
        || request_path.contains('#')
        || request_path.split('/').any(|segment| segment == "..")
    {
        return Err(TransportError::InvalidRequestPath);
    }

    let base_path = base.path().trim_end_matches('/');
    let request_path = request_path.trim_start_matches('/');
    let combined_path = if base_path.is_empty() {
        format!("/{request_path}")
    } else {
        format!("{base_path}/{request_path}")
    };
    base.set_path(&combined_path);
    Ok(base)
}

fn parse_base_url(base_url: &str) -> Result<Url, TransportError> {
    let base = Url::parse(base_url).map_err(|_| TransportError::InvalidEndpoint)?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err(TransportError::UnsupportedEndpointScheme);
    }
    if base.host_str().is_none() {
        return Err(TransportError::EndpointHostMissing);
    }
    if !base.username().is_empty() || base.password().is_some() {
        return Err(TransportError::EndpointCredentials);
    }
    if base.query().is_some() || base.fragment().is_some() {
        return Err(TransportError::EndpointQueryOrFragment);
    }
    Ok(base)
}

fn validate_transport_header(name: &HeaderName) -> Result<(), TransportError> {
    if *name == AUTHORIZATION
        || *name == CONTENT_LENGTH
        || *name == TRANSFER_ENCODING
        || matches!(name.as_str(), "host" | "connection")
    {
        return Err(TransportError::ManagedHeader {
            name: name.as_str().to_string(),
        });
    }
    if matches!(name.as_str(), "x-api-key" | "api-key") {
        return Err(TransportError::ManagedHeader {
            name: name.as_str().to_string(),
        });
    }
    Ok(())
}

fn validate_transport_auth_header(name: &HeaderName) -> Result<(), TransportError> {
    if *name == CONTENT_LENGTH
        || *name == TRANSFER_ENCODING
        || *name == CONTENT_TYPE
        || matches!(name.as_str(), "host" | "connection")
    {
        return Err(TransportError::ManagedHeader {
            name: name.as_str().to_string(),
        });
    }
    Ok(())
}

fn ensure_auth_header_available(
    seen: &HashSet<HeaderName>,
    name: &HeaderName,
) -> Result<(), TransportError> {
    if seen.contains(name) {
        return Err(TransportError::DuplicateHeader {
            name: name.as_str().to_string(),
        });
    }
    Ok(())
}

fn map_request_error(error: &reqwest::Error) -> ModelError {
    let message = if error.is_timeout() {
        "provider request timed out"
    } else if error.is_connect() {
        "provider connection failed"
    } else if error.is_builder() {
        "provider request could not be built"
    } else {
        "provider request failed"
    };
    ModelError::RequestFailed(message.to_string())
}

async fn read_bounded_body(
    response: reqwest::Response,
    max_bytes: usize,
    idle_timeout: Duration,
) -> String {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    while body.len() < max_bytes {
        let next = timeout(idle_timeout, stream.next()).await;
        let Ok(Some(Ok(chunk))) = next else {
            break;
        };
        let remaining = max_bytes.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    String::from_utf8_lossy(&body).into_owned()
}

fn redact_and_bound_body(
    body: &str,
    max_bytes: usize,
    auth: &ResolvedAuth,
    extra_headers: &[ResolvedHeader],
) -> String {
    let mut redacted = body.to_string();
    if let Some(secret) = auth.secret().filter(|secret| !secret.is_empty()) {
        redacted = redacted.replace(secret, "[REDACTED]");
    }
    for header in extra_headers {
        let secret = header.value();
        if !secret.is_empty() {
            redacted = redacted.replace(secret, "[REDACTED]");
        }
    }
    if redacted.len() <= max_bytes {
        return redacted;
    }
    if max_bytes <= ERROR_TRUNCATION_MARKER.len() {
        return ERROR_TRUNCATION_MARKER[..max_bytes].to_string();
    }
    let content_limit = max_bytes - ERROR_TRUNCATION_MARKER.len();
    let mut output = redacted;
    let mut boundary = content_limit.min(output.len());
    while !output.is_char_boundary(boundary) {
        boundary -= 1;
    }
    output.truncate(boundary);
    output.push_str(ERROR_TRUNCATION_MARKER);
    output
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use reqwest::{
        Method, StatusCode,
        header::{HeaderMap, HeaderName},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::sleep,
    };

    use super::super::{Framing, StreamDecoder};
    use super::*;
    use crate::{Message, ModelToolSchema, ProviderOptions};

    struct TestDecoder;

    impl StreamDecoder for TestDecoder {
        fn push(&mut self, frame: &str) -> Result<Vec<ModelEvent>, ModelError> {
            if frame == "[DONE]" {
                Ok(vec![ModelEvent::Done])
            } else {
                Ok(vec![ModelEvent::TextDelta {
                    text: frame.to_string(),
                }])
            }
        }
    }

    struct TestProtocol {
        classify_auth: bool,
        id: super::super::WireProtocolId,
    }

    impl TestProtocol {
        fn new(classify_auth: bool) -> Self {
            Self {
                classify_auth,
                id: super::super::WireProtocolId::new("test/stream").unwrap(),
            }
        }
    }

    impl WireProtocol for TestProtocol {
        fn id(&self) -> &super::super::WireProtocolId {
            &self.id
        }

        fn build_request(&self, input: &WireRequestInput<'_>) -> Result<WireRequest, ModelError> {
            Ok(WireRequest {
                method: Method::POST,
                path: "/chat".to_string(),
                headers: HeaderMap::from_iter([(
                    HeaderName::from_static("x-wire-protocol"),
                    reqwest::header::HeaderValue::from_static("test"),
                )]),
                body: serde_json::json!({
                    "model": input.model,
                    "messages": input.messages.len(),
                    "tools": input.tools.len(),
                    "options": input.options.max_tokens,
                }),
            })
        }

        fn framing(&self) -> Framing {
            Framing::ServerSentEvents
        }

        fn decoder(&self) -> Box<dyn StreamDecoder> {
            Box::new(TestDecoder)
        }

        fn classify_error(
            &self,
            status: StatusCode,
            _headers: &HeaderMap,
            body: &str,
        ) -> ModelError {
            if self.classify_auth && status == StatusCode::UNAUTHORIZED {
                ModelError::AuthFailed
            } else {
                ModelError::RequestFailed(body.to_string())
            }
        }

        fn default_auth_style(&self) -> AuthStyle {
            AuthStyle::Bearer
        }
    }

    async fn test_server(
        response: String,
        delay_after_headers: Option<Duration>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let header_end = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
                .unwrap();
            let content_length = String::from_utf8_lossy(&request[..header_end])
                .lines()
                .find_map(|line| {
                    line.strip_prefix("Content-Length:")
                        .or_else(|| line.strip_prefix("content-length:"))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            let _ = request;

            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            if let Some(delay) = delay_after_headers {
                sleep(delay).await;
            }
            let response_bytes = response.as_bytes();
            let midpoint = response_bytes.len() / 2;
            socket.write_all(&response_bytes[..midpoint]).await.unwrap();
            if delay_after_headers.is_some() {
                sleep(Duration::from_millis(150)).await;
            }
            socket.write_all(&response_bytes[midpoint..]).await.unwrap();
        });
        (format!("http://{address}"), handle)
    }

    async fn request_capture_server(
        response: &str,
    ) -> (
        String,
        tokio::sync::oneshot::Receiver<Vec<u8>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let response = response.to_string();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = sender.send(request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            );
            socket.write_all(header.as_bytes()).await.unwrap();
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (format!("http://{address}"), receiver, handle)
    }

    fn input() -> WireRequestInput<'static> {
        let messages = Box::leak(Box::new([Message::user("hello")]));
        let tools = Box::leak(Box::new([ModelToolSchema {
            name: "echo".to_string(),
            description: "echo".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        }]));
        let options = Box::leak(Box::new(ProviderOptions {
            max_tokens: Some(128),
            ..Default::default()
        }));
        let protocol_options = Box::leak(Box::new(serde_json::json!({})));
        WireRequestInput {
            model: "test-model",
            messages,
            tools,
            options,
            protocol_options,
        }
    }

    #[tokio::test]
    async fn transport_injects_auth_and_drives_fragmented_sse() {
        let (base_url, receiver, handle) =
            request_capture_server("data: hi\n\ndata: [DONE]\n\n").await;
        let transport = Transport::new(TransportConfig::default()).unwrap();
        let auth = ResolvedAuth::bearer("test-secret").unwrap();
        let headers = [ResolvedHeader::try_new("x-tenant", "tenant-value").unwrap()];
        let protocol = TestProtocol::new(false);
        let events = transport
            .stream(&base_url, &auth, &headers, &protocol, input())
            .collect::<Vec<_>>()
            .await;
        handle.await.unwrap();

        let request = String::from_utf8(receiver.await.unwrap())
            .unwrap()
            .to_ascii_lowercase();
        assert!(request.contains("authorization: bearer test-secret"));
        assert!(request.contains("x-tenant: tenant-value"));
        assert!(request.contains("post /chat http/1.1"));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Ok(ModelEvent::TextDelta { text }) if text == "hi"))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Ok(ModelEvent::Done)))
        );
    }

    #[tokio::test]
    async fn transport_redacts_and_bounds_error_body_before_protocol_classification() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await;
            let body = "secret=test-secret; diagnostic=".to_string() + &"x".repeat(200);
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let config = TransportConfig {
            max_error_body_bytes: 48,
            ..Default::default()
        };
        let transport = Transport::new(config).unwrap();
        let auth = ResolvedAuth::bearer("test-secret").unwrap();
        let protocol = TestProtocol::new(false);
        let events = transport
            .stream(&format!("http://{address}"), &auth, &[], &protocol, input())
            .collect::<Vec<_>>()
            .await;
        handle.await.unwrap();

        let error = events.into_iter().find_map(Result::err).unwrap();
        let ModelError::RequestFailed(message) = error else {
            panic!("unexpected error: {error:?}");
        };
        assert!(!message.contains("test-secret"));
        assert!(message.contains("[REDACTED]"));
        assert!(message.len() <= 48);
    }

    #[tokio::test]
    async fn transport_reports_idle_stream_timeout_without_leaking_url_or_secret() {
        let (base_url, handle) = test_server(
            "data: first\n\n".to_string(),
            Some(Duration::from_millis(1)),
        )
        .await;
        let config = TransportConfig {
            stream_idle_timeout: Duration::from_millis(40),
            ..Default::default()
        };
        let transport = Transport::new(config).unwrap();
        let auth = ResolvedAuth::bearer("idle-secret").unwrap();
        let protocol = TestProtocol::new(false);
        let events = transport
            .stream(&base_url, &auth, &[], &protocol, input())
            .collect::<Vec<_>>()
            .await;
        handle.await.unwrap();

        assert!(events.iter().any(|event| matches!(event, Err(ModelError::StreamInterrupted(message)) if message.contains("idle timeout"))));
        assert!(!format!("{events:?}").contains("idle-secret"));
    }

    #[test]
    fn default_idle_window_allows_slow_tool_deltas_within_request_deadline() {
        let config = TransportConfig::default();

        assert_eq!(config.stream_idle_timeout, Duration::from_secs(90));
        assert!(config.stream_idle_timeout < config.request_timeout);
    }

    #[test]
    fn endpoint_and_header_validation_fail_closed() {
        let transport = Transport::new(TransportConfig::default()).unwrap();
        let auth = ResolvedAuth::none();
        let protocol = TestProtocol::new(false);
        let request = protocol.build_request(&input()).unwrap();

        assert_eq!(
            resolve_endpoint("ftp://example.test", "/chat").unwrap_err(),
            TransportError::UnsupportedEndpointScheme
        );
        assert_eq!(
            resolve_endpoint("https://user:pass@example.test", "/chat").unwrap_err(),
            TransportError::EndpointCredentials
        );
        assert_eq!(
            resolve_endpoint("https://example.test/v1?key=secret", "/chat").unwrap_err(),
            TransportError::EndpointQueryOrFragment
        );
        assert_eq!(
            resolve_endpoint("https://example.test/v1", "https://evil.test/chat").unwrap_err(),
            TransportError::InvalidRequestPath
        );
        let mut request = request;
        request.headers.insert(
            AUTHORIZATION,
            reqwest::header::HeaderValue::from_static("bypass"),
        );
        assert_eq!(
            transport
                .prepare_request("https://example.test", &auth, &[], &request)
                .unwrap_err(),
            TransportError::ManagedHeader {
                name: "authorization".to_string()
            }
        );
    }

    #[test]
    fn auth_header_allows_credentials_but_rejects_transport_managed_names() {
        let transport = Transport::new(TransportConfig::default()).unwrap();
        let protocol = TestProtocol::new(false);
        let request = protocol.build_request(&input()).unwrap();

        for name in ["authorization", "x-api-key", "api-key"] {
            let auth = ResolvedAuth::header(HeaderName::from_static(name), "secret").unwrap();
            let (_, headers) = transport
                .prepare_request("https://example.test", &auth, &[], &request)
                .unwrap();
            assert_eq!(headers.get(name).unwrap(), "secret");
        }

        for name in [
            "host",
            "connection",
            "content-length",
            "transfer-encoding",
            "content-type",
        ] {
            let auth = ResolvedAuth::header(HeaderName::from_static(name), "secret").unwrap();
            assert_eq!(
                transport
                    .prepare_request("https://example.test", &auth, &[], &request)
                    .unwrap_err(),
                TransportError::ManagedHeader {
                    name: name.to_string()
                }
            );
        }
    }

    #[test]
    fn transport_config_rejects_zero_bounds_and_redirect_limit() {
        assert_eq!(
            Transport::new(TransportConfig {
                stream_idle_timeout: Duration::ZERO,
                ..Default::default()
            })
            .err()
            .unwrap(),
            TransportError::InvalidConfig {
                field: "stream_idle_timeout"
            }
        );
        assert_eq!(
            Transport::new(TransportConfig {
                redirect_policy: RedirectPolicy::Limited(0),
                ..Default::default()
            })
            .err()
            .unwrap(),
            TransportError::InvalidConfig {
                field: "redirect_limit"
            }
        );
        assert_eq!(
            Transport::new(TransportConfig {
                redirect_policy: RedirectPolicy::Limited(MAX_REDIRECTS + 1),
                ..Default::default()
            })
            .err()
            .unwrap(),
            TransportError::InvalidConfig {
                field: "redirect_limit"
            }
        );
        assert_eq!(
            Transport::new(TransportConfig {
                max_error_body_bytes: MAX_ERROR_BODY_BYTES + 1,
                ..Default::default()
            })
            .err()
            .unwrap(),
            TransportError::InvalidConfig {
                field: "max_error_body_bytes"
            }
        );
    }

    #[test]
    fn default_redirect_policy_is_fail_closed() {
        assert_eq!(
            TransportConfig::default().redirect_policy,
            RedirectPolicy::None
        );
    }

    #[test]
    fn unicode_error_body_is_truncated_on_a_character_boundary() {
        let auth = ResolvedAuth::none();
        let body = "错误信息".repeat(20);
        let output = redact_and_bound_body(&body, 24, &auth, &[]);

        assert!(output.len() <= 24);
        assert!(output.is_char_boundary(output.len()));
        assert!(output.ends_with(ERROR_TRUNCATION_MARKER));
    }
}
