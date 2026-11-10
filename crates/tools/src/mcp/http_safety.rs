//! URL, redirect, header, and response validation for HTTP MCP transports.
//!
//! A server-supplied URL or header is untrusted input, never a capability
//! grant. These checks are deliberately fail-closed: an endpoint that cannot be
//! proven acceptable is refused rather than attempted.

use super::protocol::{McpProtocolError, bounded_diagnostic};

pub const MAX_MCP_ENDPOINT_BYTES: usize = 2_048;
pub const MAX_MCP_REDIRECTS: usize = 3;

/// Content types a JSON-RPC response body may declare.
pub const MCP_JSON_CONTENT_TYPE: &str = "application/json";
/// Content type of an SSE response body.
pub const MCP_SSE_CONTENT_TYPE: &str = "text/event-stream";

/// What an HTTP endpoint is allowed to be.
/// Defaults are the safe choices: TLS required, redirects refused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HttpEndpointPolicy {
    /// Allow plaintext HTTP. Off by default; intended for loopback and local
    /// deterministic fixtures only.
    pub allow_plaintext_http: bool,
    /// Follow redirects, re-validating the target at each hop.
    pub follow_redirects: bool,
}

impl HttpEndpointPolicy {
    /// Policy for local deterministic fixtures and loopback servers.
    pub fn loopback_permitted() -> Self {
        Self {
            allow_plaintext_http: true,
            follow_redirects: false,
        }
    }
}

/// A validated endpoint. Constructing one is the only way to reach the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEndpoint {
    url: String,
    host: String,
    is_loopback: bool,
}

impl ValidatedEndpoint {
    pub fn as_str(&self) -> &str {
        &self.url
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn is_loopback(&self) -> bool {
        self.is_loopback
    }
}

/// Validate an MCP HTTP endpoint against `policy`.
///
/// Rejects, in order: oversized input, non-HTTP schemes, embedded credentials,
/// missing hosts, and plaintext HTTP to a non-loopback host when the policy does
/// not permit it. Userinfo is refused outright because it would smuggle
/// credentials into a URL that also reaches logs and diagnostics.
pub fn validate_endpoint(
    candidate: &str,
    policy: HttpEndpointPolicy,
) -> Result<ValidatedEndpoint, McpProtocolError> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_MCP_ENDPOINT_BYTES {
        return Err(McpProtocolError::Transport {
            detail: "MCP endpoint is empty or exceeds the supported length".to_string(),
        });
    }

    let (scheme, remainder) = match trimmed.split_once("://") {
        Some((scheme, remainder)) => (scheme.to_ascii_lowercase(), remainder),
        None => {
            return Err(McpProtocolError::Transport {
                detail: "MCP endpoint must be an absolute http(s) URL".to_string(),
            });
        }
    };
    if scheme != "http" && scheme != "https" {
        return Err(McpProtocolError::Transport {
            detail: format!("MCP endpoint scheme {scheme} is not supported"),
        });
    }

    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(McpProtocolError::Transport {
            detail: "MCP endpoint is missing a host".to_string(),
        });
    }
    // Credentials in a URL would be copied into logs and diagnostics.
    if authority.contains('@') {
        return Err(McpProtocolError::Transport {
            detail: "MCP endpoint must not contain userinfo credentials".to_string(),
        });
    }

    let host = host_of(authority);
    if host.is_empty() {
        return Err(McpProtocolError::Transport {
            detail: "MCP endpoint is missing a host".to_string(),
        });
    }
    let is_loopback = is_loopback_host(&host);

    if scheme == "http" && !(policy.allow_plaintext_http && is_loopback) {
        return Err(McpProtocolError::Transport {
            detail:
                "MCP endpoint must use https unless it is an explicitly permitted loopback address"
                    .to_string(),
        });
    }

    Ok(ValidatedEndpoint {
        url: trimmed.to_string(),
        host,
        is_loopback,
    })
}

/// Re-validate a redirect target against the original endpoint.
///
/// A redirect may not change host, and may not downgrade to plaintext. This
/// blocks a redirect from moving a session to an unvetted host.
pub fn validate_redirect(
    origin: &ValidatedEndpoint,
    location: &str,
    policy: HttpEndpointPolicy,
) -> Result<ValidatedEndpoint, McpProtocolError> {
    if !policy.follow_redirects {
        return Err(McpProtocolError::Transport {
            detail: "MCP endpoint redirect was refused by policy".to_string(),
        });
    }
    let target = validate_endpoint(location, policy)?;
    if target.host() != origin.host() {
        return Err(McpProtocolError::Transport {
            detail: "MCP endpoint redirect changed host and was refused".to_string(),
        });
    }
    Ok(target)
}

fn host_of(authority: &str) -> String {
    // Bracketed IPv6 literal.
    if let Some(rest) = authority.strip_prefix('[')
        && let Some(end) = rest.find(']')
    {
        return rest[..end].to_ascii_lowercase();
    }
    authority
        .rsplit_once(':')
        .map(|(host, _port)| host)
        .unwrap_or(authority)
        .to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" || host == "::1" {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_loopback())
}

/// Classify the declared content type of a response body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpResponseKind {
    Json,
    EventStream,
    /// Accepted with no body, e.g. a notification acknowledged with 202.
    Empty,
}

/// Decide how to read a response body.
///
/// The content type is never guessed: an unrecognized type fails so a hostile
/// or misconfigured server cannot have its body reinterpreted.
pub fn classify_response(
    status: u16,
    content_type: Option<&str>,
    has_body: bool,
) -> Result<McpResponseKind, McpProtocolError> {
    // A session the server no longer knows must be re-initialized rather than
    // retried against a dead session ID.
    if status == 404 {
        return Err(McpProtocolError::SessionExpired);
    }
    if status == 202 || !has_body {
        return Ok(McpResponseKind::Empty);
    }
    if !(200..300).contains(&status) {
        return Err(McpProtocolError::Transport {
            detail: format!("MCP endpoint returned HTTP {status}"),
        });
    }

    let declared = content_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match declared.as_str() {
        MCP_JSON_CONTENT_TYPE => Ok(McpResponseKind::Json),
        MCP_SSE_CONTENT_TYPE => Ok(McpResponseKind::EventStream),
        "" => Err(McpProtocolError::Transport {
            detail: "MCP response did not declare a content type".to_string(),
        }),
        other => Err(McpProtocolError::Transport {
            detail: bounded_diagnostic(&format!(
                "MCP response declared unsupported content type {other}"
            )),
        }),
    }
}

/// One parsed SSE frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseFrame {
    pub event: Option<String>,
    pub data: String,
    /// Present when the server supplied an ID usable for `Last-Event-ID`.
    pub id: Option<String>,
}

/// Incremental SSE frame parser with a bounded buffer.
#[derive(Debug, Default)]
pub struct SseFrameParser {
    buffer: String,
    max_frame_bytes: usize,
}

impl SseFrameParser {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            buffer: String::new(),
            max_frame_bytes,
        }
    }

    /// Feed a chunk and return every complete frame it produced.
    pub fn push(&mut self, chunk: &str) -> Result<Vec<SseFrame>, McpProtocolError> {
        if self.buffer.len().saturating_add(chunk.len()) > self.max_frame_bytes {
            return Err(McpProtocolError::MessageTooLarge);
        }
        self.buffer.push_str(chunk);

        let mut frames = Vec::new();
        // A blank line terminates a frame. Both LF and CRLF forms are accepted.
        loop {
            let boundary = self
                .buffer
                .find("\n\n")
                .map(|index| (index, 2))
                .or_else(|| self.buffer.find("\r\n\r\n").map(|index| (index, 4)));
            let Some((index, width)) = boundary else {
                break;
            };
            let block: String = self.buffer.drain(..index + width).collect();
            if let Some(frame) = parse_sse_block(&block) {
                frames.push(frame);
            }
        }
        Ok(frames)
    }
}

fn parse_sse_block(block: &str) -> Option<SseFrame> {
    let mut frame = SseFrame::default();
    let mut data_lines = Vec::new();
    for line in block.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => frame.event = Some(value.to_string()),
            "data" => data_lines.push(value.to_string()),
            "id" => frame.id = Some(value.to_string()),
            // Unknown fields (including `retry`) are ignored per the SSE spec.
            _ => {}
        }
    }
    if data_lines.is_empty() && frame.event.is_none() && frame.id.is_none() {
        return None;
    }
    frame.data = data_lines.join("\n");
    Some(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_endpoints_are_accepted_and_plaintext_is_refused_by_default() {
        let policy = HttpEndpointPolicy::default();
        let endpoint = validate_endpoint("https://mcp.example.com/rpc", policy).unwrap();
        assert_eq!(endpoint.host(), "mcp.example.com");
        assert!(!endpoint.is_loopback());

        let error = validate_endpoint("http://mcp.example.com/rpc", policy).unwrap_err();
        assert!(matches!(error, McpProtocolError::Transport { .. }));
    }

    #[test]
    fn plaintext_is_permitted_only_for_loopback_and_only_when_enabled() {
        let policy = HttpEndpointPolicy::loopback_permitted();
        for allowed in [
            "http://127.0.0.1:8931/mcp",
            "http://localhost:3000/rpc",
            "http://[::1]:8080/rpc",
        ] {
            let endpoint = validate_endpoint(allowed, policy).unwrap();
            assert!(endpoint.is_loopback(), "{allowed} should be loopback");
        }
        // Enabling plaintext does not open non-loopback hosts.
        assert!(validate_endpoint("http://example.com/rpc", policy).is_err());
    }

    #[test]
    fn hostile_endpoints_are_refused() {
        let policy = HttpEndpointPolicy::loopback_permitted();
        for hostile in [
            "",
            "ftp://example.com/rpc",
            "file:///etc/passwd",
            "not-a-url",
            "https://user:secret@example.com/rpc",
            "https:///no-host",
        ] {
            assert!(
                validate_endpoint(hostile, policy).is_err(),
                "must refuse {hostile:?}"
            );
        }
        assert!(
            validate_endpoint(
                &format!("https://a.com/{}", "x".repeat(MAX_MCP_ENDPOINT_BYTES)),
                policy
            )
            .is_err()
        );
    }

    #[test]
    fn a_redirect_may_not_change_host_or_be_followed_by_default() {
        let origin =
            validate_endpoint("https://mcp.example.com/rpc", HttpEndpointPolicy::default())
                .unwrap();

        // Refused entirely when redirects are disabled.
        assert!(
            validate_redirect(
                &origin,
                "https://mcp.example.com/moved",
                HttpEndpointPolicy::default()
            )
            .is_err()
        );

        let permissive = HttpEndpointPolicy {
            allow_plaintext_http: false,
            follow_redirects: true,
        };
        assert!(validate_redirect(&origin, "https://mcp.example.com/moved", permissive).is_ok());
        // A different host is refused even when redirects are allowed.
        assert!(validate_redirect(&origin, "https://evil.example.net/rpc", permissive).is_err());
        // A downgrade to plaintext is refused.
        assert!(validate_redirect(&origin, "http://mcp.example.com/rpc", permissive).is_err());
    }

    #[test]
    fn response_classification_never_guesses_a_content_type() {
        assert_eq!(
            classify_response(200, Some("application/json"), true).unwrap(),
            McpResponseKind::Json
        );
        assert_eq!(
            classify_response(200, Some("text/event-stream; charset=utf-8"), true).unwrap(),
            McpResponseKind::EventStream
        );
        assert_eq!(
            classify_response(202, None, false).unwrap(),
            McpResponseKind::Empty
        );
        // A dead session must be re-initialized, not retried.
        assert_eq!(
            classify_response(404, None, false),
            Err(McpProtocolError::SessionExpired)
        );
        assert!(classify_response(200, Some("text/html"), true).is_err());
        assert!(classify_response(200, None, true).is_err());
        assert!(classify_response(500, Some("application/json"), true).is_err());
    }

    #[test]
    fn sse_frames_are_parsed_incrementally_with_ids() {
        let mut parser = SseFrameParser::new(4096);
        assert!(parser.push("data: {\"jsonrpc\"").unwrap().is_empty());

        let frames = parser
            .push(":\"2.0\",\"id\":1}\n\nid: 7\nevent: message\ndata: second\n\n")
            .unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, "{\"jsonrpc\":\"2.0\",\"id\":1}");
        assert_eq!(frames[1].id.as_deref(), Some("7"));
        assert_eq!(frames[1].event.as_deref(), Some("message"));
        assert_eq!(frames[1].data, "second");
    }

    #[test]
    fn multi_line_data_and_comments_are_handled() {
        let mut parser = SseFrameParser::new(4096);
        let frames = parser
            .push(": this is a comment\ndata: line one\ndata: line two\n\n")
            .unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "line one\nline two");
    }

    #[test]
    fn crlf_frames_are_supported() {
        let mut parser = SseFrameParser::new(4096);
        let frames = parser.push("data: hello\r\n\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, "hello");
    }

    #[test]
    fn an_oversized_sse_stream_is_refused() {
        let mut parser = SseFrameParser::new(32);
        assert_eq!(
            parser.push(&"d".repeat(64)),
            Err(McpProtocolError::MessageTooLarge)
        );
    }
}
