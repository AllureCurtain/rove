//! Shared rich tool result contract.
//!
//! One envelope describes every tool outcome, MCP or local, so no consumer
//! invents a private result shape. The envelope carries bounded content
//! blocks, validated structured content, references to durable artifacts, and
//! only display-safe protocol metadata.
//!
//! Three rules hold everywhere in this module:
//!
//! - Remote data is untrusted. A MIME type, filename, URI, or annotation is
//!   recorded as a claim and validated before use, never treated as authority.
//! - Large data does not travel inline. Anything past a bound becomes an
//!   artifact reference or a truncated preview with the truncation recorded.
//! - Projections narrow. Each consumer gets what it needs, and the model,
//!   planner, and UI projections cannot see secrets, raw headers, session
//!   values, or base64 payloads.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{ToolExecutionStatus, ToolMutation};

/// Longest inline text kept on a block before it is truncated or promoted to
/// an artifact.
pub const MAX_INLINE_TEXT_BYTES: usize = 16 * 1024;

/// Longest inline preview kept for a block whose payload lives in an artifact.
pub const MAX_BLOCK_PREVIEW_BYTES: usize = 2 * 1024;

/// Most content blocks one tool result may carry.
pub const MAX_CONTENT_BLOCKS: usize = 64;

/// Most diagnostics one tool result may carry.
pub const MAX_DIAGNOSTICS: usize = 32;

/// Largest retained body for a block whose type this build does not model.
pub const MAX_UNKNOWN_BLOCK_BYTES: usize = 4 * 1024;

/// Deepest structured-content nesting accepted from a tool.
pub const MAX_STRUCTURED_JSON_DEPTH: usize = 32;

/// Most structured-content nodes accepted from a tool.
pub const MAX_STRUCTURED_JSON_NODES: usize = 4_096;

/// Longest recorded MIME type claim.
pub const MAX_MIME_TYPE_BYTES: usize = 255;

/// Longest recorded URI claim.
pub const MAX_URI_BYTES: usize = 2_048;

/// Longest single diagnostic message.
pub const MAX_DIAGNOSTIC_MESSAGE_BYTES: usize = 500;

/// Why a tool result ended the way it did, at the granularity a caller needs
/// to decide whether retrying is safe.
///
/// This is distinct from [`ToolExecutionStatus`], which stays as the coarse
/// persisted status. `Indeterminate` and `TimedOutKnownNotSent` exist because
/// a tool call can have external effects: only the latter is safe to retry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultOutcome {
    #[default]
    Success,
    /// Some requested work completed and some did not, with both recorded.
    Partial,
    Error,
    /// Refused before dispatch by local policy or approval.
    Rejected,
    Cancelled,
    /// Timed out with proof the request never reached the far side.
    TimedOutKnownNotSent,
    /// Committed to the wire, then failed. The external effect is unknown, so
    /// a caller must not retry it as if nothing happened.
    Indeterminate,
}

impl ToolResultOutcome {
    /// True when retrying cannot duplicate an external effect.
    ///
    /// `Indeterminate` is deliberately absent: the point of that variant is
    /// that the effect is unknown.
    pub fn is_safely_retryable(self) -> bool {
        matches!(self, Self::TimedOutKnownNotSent | Self::Rejected)
    }

    /// True when the caller must treat the external effect as unknown.
    pub fn is_indeterminate(self) -> bool {
        matches!(self, Self::Indeterminate)
    }

    /// Coarse status persisted in existing records and reports.
    ///
    /// Cancelled, timed-out, and indeterminate outcomes all project to
    /// `Error`, because to every existing consumer the step did not succeed.
    /// The precise outcome stays available on the envelope.
    pub fn to_execution_status(self) -> ToolExecutionStatus {
        match self {
            Self::Success => ToolExecutionStatus::Ok,
            Self::Partial => ToolExecutionStatus::PartialSuccess,
            Self::Rejected => ToolExecutionStatus::Rejected,
            Self::Error | Self::Cancelled | Self::TimedOutKnownNotSent | Self::Indeterminate => {
                ToolExecutionStatus::Error
            }
        }
    }
}

/// Where an error came from, so diagnostics stay classifiable without parsing
/// a human-readable message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorDomain {
    Transport,
    Http,
    JsonRpc,
    Protocol,
    /// The remote tool itself reported failure via `isError`.
    RemoteTool,
    InputSchema,
    OutputSchema,
    Artifact,
    Policy,
    Internal,
}

/// Opaque durable artifact identity.
///
/// Opaque on purpose: it is generated locally from validated bytes, so a
/// remote filename or URI can never steer where a payload is written or read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactId(String);

impl ArtifactId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What an artifact holds, as classified locally after validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolArtifactKind {
    Text,
    Image,
    Audio,
    /// A resource fetched or embedded by a tool.
    Resource,
    /// Retained bytes for a block type this build does not model.
    Unknown,
}

/// How sensitive an artifact's bytes are, which drives retention and whether a
/// payload may be projected at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    #[default]
    Normal,
    /// Shorter retention and never previewed inline.
    Sensitive,
}

/// How much the local runtime trusts an artifact's origin. Always
/// `Untrusted` for anything a remote server produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactTrust {
    #[default]
    Untrusted,
    LocalTool,
}

/// Outcome of validating a claim against the retained bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactValidation {
    #[default]
    /// Bytes were hashed and length-checked, and the claimed MIME type is a
    /// well-formed type this build accepts.
    Validated,
    /// Retained, but a claim did not hold. The reason is recorded on the
    /// artifact rather than inferred from the kind.
    ClaimRejected,
    /// A bound stopped retention before the payload was complete.
    QuotaExceeded,
}

/// Where an artifact came from. Every field is either locally generated or a
/// hash, so provenance can be shown and audited without leaking a session
/// value or a raw remote header.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolArtifactSource {
    pub run_id: String,
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_config_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_identity_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tool_name: Option<String>,
    /// Position of the originating content block in the tool's result.
    pub block_ordinal: u32,
    pub captured_at: String,
}

/// Reference to a durable artifact. The envelope carries references, never
/// payload bytes, so a result stays small no matter how large the data was.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolArtifactRef {
    pub artifact_id: ArtifactId,
    pub kind: ToolArtifactKind,
    /// MIME type as validated locally. `None` when the claim was rejected or
    /// absent; it is never guessed from a filename or from the bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub byte_length: u64,
    /// Hash of the exact retained bytes.
    pub sha256: String,
    /// Locally generated storage locator. Never a remote-supplied path.
    pub storage_ref: String,
    pub source: ToolArtifactSource,
    /// URI the remote claimed, retained for provenance only. It is not
    /// resolved, fetched, or used to build a local path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub sensitivity: Sensitivity,
    #[serde(default)]
    pub trust: ArtifactTrust,
    #[serde(default)]
    pub validation: ArtifactValidation,
    /// Why validation did not pass, when it did not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_detail: Option<String>,
}

impl ToolArtifactRef {
    /// True when the payload may be previewed or downloaded inline by a UI.
    ///
    /// Sensitive and unvalidated artifacts are excluded, and so is anything
    /// whose MIME type could execute in a browser.
    pub fn is_inline_previewable(&self) -> bool {
        if self.sensitivity == Sensitivity::Sensitive
            || self.validation != ArtifactValidation::Validated
        {
            return false;
        }
        self.mime_type
            .as_deref()
            .is_some_and(|mime| !mime_type_is_active_content(mime))
    }
}

/// True for MIME types a browser may execute or treat as a document.
///
/// Matched against an explicit list rather than by sniffing the bytes: a
/// remote server must not be able to get active content rendered inline by
/// mislabeling it.
pub fn mime_type_is_active_content(mime_type: &str) -> bool {
    let base = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "text/html"
            | "application/xhtml+xml"
            | "image/svg+xml"
            | "application/javascript"
            | "text/javascript"
            | "application/ecmascript"
            | "text/ecmascript"
            | "application/x-httpd-php"
            | "application/xml"
            | "text/xml"
            | "application/pdf"
    )
}

/// A well-formed MIME type claim, or `None`.
///
/// Only `type/subtype` with optional parameters is accepted, within a byte
/// bound. A rejected claim becomes `None` rather than a guess, so downstream
/// code cannot mistake a malformed value for a real type.
pub fn validated_mime_type(claim: Option<&str>) -> Option<String> {
    let claim = claim?.trim();
    if claim.is_empty() || claim.len() > MAX_MIME_TYPE_BYTES {
        return None;
    }
    if claim.chars().any(|c| c.is_control()) {
        return None;
    }
    let base = claim.split(';').next().unwrap_or_default().trim();
    let (kind, subtype) = base.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
    };
    if !valid(kind) || !valid(subtype) {
        return None;
    }
    Some(claim.to_ascii_lowercase())
}

/// A recorded remote URI claim, or `None` when it is unusable.
///
/// The URI is only ever displayed and audited. It is bounded and stripped of
/// control characters so it cannot corrupt a log line or a UI.
pub fn recorded_uri_claim(claim: Option<&str>) -> Option<String> {
    let claim = claim?.trim();
    if claim.is_empty() || claim.len() > MAX_URI_BYTES {
        return None;
    }
    if claim.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(claim.to_string())
}

/// Shared per-block facts. Every block records its position, what the remote
/// claimed, and whether the retained form is complete.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentBlockMeta {
    /// Position in the tool's original result, preserved even when a block is
    /// truncated or promoted to an artifact.
    pub ordinal: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Validated audience annotation. Advisory only: it never changes who may
    /// read a block, only how a UI may present it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<f32>,
    /// True when the retained inline form is shorter than what the tool sent.
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub validation: ArtifactValidation,
}

impl ContentBlockMeta {
    pub fn new(ordinal: u32) -> Self {
        Self {
            ordinal,
            mime_type: None,
            audience: None,
            priority: None,
            truncated: false,
            validation: ArtifactValidation::Validated,
        }
    }
}

/// One piece of a tool result.
///
/// Binary kinds never carry bytes here: they carry an artifact reference plus
/// an optional bounded, non-executable preview. `Unknown` exists so a block
/// type this build does not model is preserved rather than silently dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContentBlock {
    Text {
        meta: ContentBlockMeta,
        text: String,
    },
    Image {
        meta: ContentBlockMeta,
        artifact: ToolArtifactRef,
    },
    Audio {
        meta: ContentBlockMeta,
        artifact: ToolArtifactRef,
    },
    /// A link the remote offered. Recorded, never followed here.
    ResourceLink {
        meta: ContentBlockMeta,
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    EmbeddedResource {
        meta: ContentBlockMeta,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
        artifact: ToolArtifactRef,
        /// Bounded, text-only preview. Absent for sensitive or active content.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
    },
    /// A block whose declared type this build does not model. The declared
    /// type name and a bounded body are kept so nothing is lost.
    Unknown {
        meta: ContentBlockMeta,
        declared_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retained: Option<String>,
    },
}

impl ToolContentBlock {
    pub fn meta(&self) -> &ContentBlockMeta {
        match self {
            Self::Text { meta, .. }
            | Self::Image { meta, .. }
            | Self::Audio { meta, .. }
            | Self::ResourceLink { meta, .. }
            | Self::EmbeddedResource { meta, .. }
            | Self::Unknown { meta, .. } => meta,
        }
    }

    pub fn artifact(&self) -> Option<&ToolArtifactRef> {
        match self {
            Self::Image { artifact, .. }
            | Self::Audio { artifact, .. }
            | Self::EmbeddedResource { artifact, .. } => Some(artifact),
            _ => None,
        }
    }

    /// Short label naming the block kind, for diagnostics and UI.
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Image { .. } => "image",
            Self::Audio { .. } => "audio",
            Self::ResourceLink { .. } => "resource_link",
            Self::EmbeddedResource { .. } => "embedded_resource",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// Text a model may see for this block.
    ///
    /// Binary payloads produce a description rather than their bytes, so a
    /// model prompt can never receive base64.
    pub fn model_text(&self) -> String {
        match self {
            Self::Text { text, .. } => text.clone(),
            Self::Image { artifact, .. } | Self::Audio { artifact, .. } => {
                describe_artifact(self.kind_label(), artifact)
            }
            Self::ResourceLink { uri, name, .. } => match name {
                Some(name) => format!("[resource link] {name} <{uri}>"),
                None => format!("[resource link] <{uri}>"),
            },
            Self::EmbeddedResource {
                artifact, preview, ..
            } => match preview {
                Some(preview) => format!(
                    "{}\n{preview}",
                    describe_artifact("embedded resource", artifact)
                ),
                None => describe_artifact("embedded resource", artifact),
            },
            Self::Unknown {
                declared_type,
                retained,
                ..
            } => match retained {
                Some(retained) => {
                    format!("[unsupported block \"{declared_type}\"] {retained}")
                }
                None => format!("[unsupported block \"{declared_type}\"]"),
            },
        }
    }
}

fn describe_artifact(label: &str, artifact: &ToolArtifactRef) -> String {
    let mime = artifact.mime_type.as_deref().unwrap_or("unknown type");
    format!(
        "[{label}] {} ({mime}, {} bytes, artifact {})",
        artifact.kind_label(),
        artifact.byte_length,
        artifact.artifact_id
    )
}

impl ToolArtifactRef {
    fn kind_label(&self) -> &'static str {
        match self.kind {
            ToolArtifactKind::Text => "text",
            ToolArtifactKind::Image => "image",
            ToolArtifactKind::Audio => "audio",
            ToolArtifactKind::Resource => "resource",
            ToolArtifactKind::Unknown => "unknown",
        }
    }
}

/// Why structured content was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredContentRejection {
    TooDeep,
    TooManyNodes,
}

impl std::fmt::Display for StructuredContentRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooDeep => write!(f, "structured content exceeds the maximum nesting depth"),
            Self::TooManyNodes => write!(f, "structured content exceeds the maximum node count"),
        }
    }
}

/// Structured content a tool returned, with its schema verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredToolContent {
    pub value: Value,
    /// Whether a declared output schema was satisfied. `None` when the tool
    /// declared no output schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_valid: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_error: Option<String>,
}

impl StructuredToolContent {
    /// Accepts structured content only if it fits the depth and node bounds.
    ///
    /// Bounded before anything else touches it: a deeply nested or enormous
    /// object from a remote server must not be walked by a projection, a
    /// serializer, or a schema validator first.
    pub fn bounded(value: Value) -> Result<Self, StructuredContentRejection> {
        let mut nodes = 0usize;
        check_structured_bounds(&value, 1, &mut nodes)?;
        Ok(Self {
            value,
            schema_valid: None,
            schema_error: None,
        })
    }

    pub fn with_schema_verdict(mut self, valid: bool, error: Option<String>) -> Self {
        self.schema_valid = Some(valid);
        self.schema_error =
            error.map(|error| truncate_utf8(&error, MAX_DIAGNOSTIC_MESSAGE_BYTES).0);
        self
    }
}

fn check_structured_bounds(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), StructuredContentRejection> {
    if depth > MAX_STRUCTURED_JSON_DEPTH {
        return Err(StructuredContentRejection::TooDeep);
    }
    *nodes += 1;
    if *nodes > MAX_STRUCTURED_JSON_NODES {
        return Err(StructuredContentRejection::TooManyNodes);
    }
    match value {
        Value::Array(items) => {
            for item in items {
                check_structured_bounds(item, depth + 1, nodes)?;
            }
        }
        Value::Object(entries) => {
            for entry in entries.values() {
                check_structured_bounds(entry, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Display-safe protocol facts about how a result was produced.
///
/// Every remote-derived value here is a hash or a locally validated name.
/// There is deliberately no field for an Authorization header, a raw header
/// map, or a readable session ID.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolProtocolMetadata {
    /// Protocol family, for example `mcp`. Absent for a purely local tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_config_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_identity_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_hash: Option<String>,
    /// How many dispatch attempts produced this result. Zero means the tool
    /// was local and never dispatched over a protocol.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// One classified, bounded diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDiagnostic {
    pub domain: ToolErrorDomain,
    /// Stable machine-readable code. Not a message.
    pub code: String,
    /// Redacted, bounded human-readable detail.
    pub message: String,
}

impl ToolDiagnostic {
    pub fn new(domain: ToolErrorDomain, code: impl Into<String>, message: &str) -> Self {
        Self {
            domain,
            code: code.into(),
            message: truncate_utf8(message, MAX_DIAGNOSTIC_MESSAGE_BYTES).0,
        }
    }
}

/// An effect outside the workspace that a tool reported causing.
///
/// Recorded so a Finalizer never claims an unverified external action
/// succeeded, and so an indeterminate outcome stays visible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEffect {
    /// What kind of effect, for example `mcp_tool_call` or `network_write`.
    pub kind: String,
    /// Display-safe target description. Never a credential-bearing URL.
    pub target: String,
    /// True when it is unknown whether the effect happened.
    #[serde(default)]
    pub indeterminate: bool,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// Truncates on a UTF-8 boundary, reporting whether anything was removed.
pub fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

/// The one rich tool result shape.
///
/// `summary_text` is the legacy text projection and is always populated, so
/// every existing consumer keeps working unchanged while richer consumers read
/// the blocks, structured content, and artifacts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolOutputEnvelope {
    #[serde(default)]
    pub outcome: ToolResultOutcome,
    pub summary_text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<ToolContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<StructuredToolContent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ToolArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutations: Vec<ToolMutation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_effects: Vec<ExternalEffect>,
    #[serde(default)]
    pub protocol_metadata: ToolProtocolMetadata,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ToolDiagnostic>,
}

impl Default for ToolOutputEnvelope {
    fn default() -> Self {
        Self {
            outcome: ToolResultOutcome::Success,
            summary_text: String::new(),
            content_blocks: Vec::new(),
            structured_content: None,
            artifacts: Vec::new(),
            mutations: Vec::new(),
            external_effects: Vec::new(),
            protocol_metadata: ToolProtocolMetadata::default(),
            diagnostics: Vec::new(),
        }
    }
}

impl ToolOutputEnvelope {
    /// A successful text-only result, the shape most local tools produce.
    pub fn text(summary: impl Into<String>) -> Self {
        Self {
            summary_text: summary.into(),
            ..Self::default()
        }
    }

    /// Applies the envelope-wide bounds.
    ///
    /// Called once when an envelope is built from untrusted input, so no later
    /// consumer has to re-check counts. Dropped items are reported as
    /// diagnostics rather than disappearing.
    pub fn enforce_bounds(mut self) -> Self {
        let (summary, truncated) = truncate_utf8(&self.summary_text, MAX_INLINE_TEXT_BYTES);
        self.summary_text = summary;
        if truncated {
            self.diagnostics.push(ToolDiagnostic::new(
                ToolErrorDomain::Protocol,
                "summary_truncated",
                "the result summary exceeded the inline text bound and was truncated",
            ));
        }
        if self.content_blocks.len() > MAX_CONTENT_BLOCKS {
            let dropped = self.content_blocks.len() - MAX_CONTENT_BLOCKS;
            self.content_blocks.truncate(MAX_CONTENT_BLOCKS);
            self.diagnostics.push(ToolDiagnostic::new(
                ToolErrorDomain::Protocol,
                "content_blocks_truncated",
                &format!("dropped {dropped} content blocks beyond the bound"),
            ));
            // Losing blocks means the result is no longer complete.
            if self.outcome == ToolResultOutcome::Success {
                self.outcome = ToolResultOutcome::Partial;
            }
        }
        self.diagnostics.truncate(MAX_DIAGNOSTICS);
        self
    }

    /// True when the caller must not assume the external effect did or did not
    /// happen.
    pub fn is_indeterminate(&self) -> bool {
        self.outcome.is_indeterminate()
            || self
                .external_effects
                .iter()
                .any(|effect| effect.indeterminate)
    }

    /// Text projected to the model.
    ///
    /// Concatenates the summary with each block's model text, then bounds the
    /// whole thing. No base64, headers, or session values can reach a prompt
    /// through this path because no block exposes them.
    pub fn model_projection(&self) -> String {
        let mut sections: Vec<String> = Vec::new();
        if !self.summary_text.is_empty() {
            sections.push(self.summary_text.clone());
        }
        for block in &self.content_blocks {
            let text = block.model_text();
            if !text.is_empty() {
                sections.push(text);
            }
        }
        if let Some(structured) = &self.structured_content {
            if let Ok(rendered) = serde_json::to_string(&structured.value) {
                let (rendered, _) = truncate_utf8(&rendered, MAX_BLOCK_PREVIEW_BYTES);
                sections.push(format!("[structured content] {rendered}"));
            }
            if structured.schema_valid == Some(false) {
                sections.push(format!(
                    "[structured content failed its declared output schema{}]",
                    structured
                        .schema_error
                        .as_deref()
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                ));
            }
        }
        match self.outcome {
            ToolResultOutcome::Partial => {
                sections.push("[the tool reported a partial result]".to_string());
            }
            ToolResultOutcome::Indeterminate => {
                sections.push(
                    "[the tool call may or may not have taken effect; do not assume it was applied]"
                        .to_string(),
                );
            }
            ToolResultOutcome::TimedOutKnownNotSent => {
                sections.push("[the tool call timed out before it was sent]".to_string());
            }
            ToolResultOutcome::Cancelled => {
                sections.push("[the tool call was cancelled]".to_string());
            }
            _ => {}
        }
        truncate_utf8(&sections.join("\n"), MAX_INLINE_TEXT_BYTES).0
    }

    /// What a UI may render: block cards, validation badges, and safe artifact
    /// handles. Sensitive and active-content artifacts are marked
    /// non-previewable rather than omitted, so the UI shows they exist.
    pub fn ui_projection(&self) -> UiResultProjection {
        UiResultProjection {
            outcome: self.outcome,
            summary_text: self.summary_text.clone(),
            blocks: self
                .content_blocks
                .iter()
                .map(|block| UiBlockProjection {
                    ordinal: block.meta().ordinal,
                    kind: block.kind_label().to_string(),
                    mime_type: block.meta().mime_type.clone(),
                    truncated: block.meta().truncated,
                    validation: block.meta().validation,
                    artifact_id: block.artifact().map(|a| a.artifact_id.clone()),
                    inline_previewable: block
                        .artifact()
                        .is_some_and(ToolArtifactRef::is_inline_previewable),
                    text: match block {
                        ToolContentBlock::Text { text, .. } => Some(text.clone()),
                        ToolContentBlock::EmbeddedResource { preview, .. } => preview.clone(),
                        _ => None,
                    },
                })
                .collect(),
            structured_schema_valid: self
                .structured_content
                .as_ref()
                .and_then(|structured| structured.schema_valid),
            artifact_count: self.artifacts.len(),
            attempt_count: self.protocol_metadata.attempt_count,
            server_identity_hash: self.protocol_metadata.server_identity_hash.clone(),
            remote_tool_name: self.protocol_metadata.remote_tool_name.clone(),
            external_effects: self.external_effects.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    /// What a Finalizer may treat as evidence.
    ///
    /// Deliberately not the raw content: a Finalizer gets the terminal status,
    /// evidence references, and anything unverified, so it cannot assemble a
    /// confident claim out of unverified remote text.
    pub fn finalizer_projection(&self) -> FinalizerResultProjection {
        FinalizerResultProjection {
            outcome: self.outcome,
            summary_text: truncate_utf8(&self.summary_text, MAX_BLOCK_PREVIEW_BYTES).0,
            artifact_ids: self
                .artifacts
                .iter()
                .map(|artifact| artifact.artifact_id.clone())
                .collect(),
            mutation_paths: self
                .mutations
                .iter()
                .map(|mutation| mutation.path.clone())
                .collect(),
            unverified_effects: self
                .external_effects
                .iter()
                .filter(|effect| effect.indeterminate)
                .cloned()
                .collect(),
            structured_schema_valid: self
                .structured_content
                .as_ref()
                .and_then(|structured| structured.schema_valid),
        }
    }

    /// What an audit record keeps: identity, hashes, lineage, and decisions.
    /// It may be broader than a user-facing report but still holds no secret.
    pub fn audit_projection(&self) -> AuditResultProjection {
        AuditResultProjection {
            outcome: self.outcome,
            protocol_metadata: self.protocol_metadata.clone(),
            artifact_lineage: self
                .artifacts
                .iter()
                .map(|artifact| AuditArtifactLineage {
                    artifact_id: artifact.artifact_id.clone(),
                    sha256: artifact.sha256.clone(),
                    byte_length: artifact.byte_length,
                    kind: artifact.kind,
                    sensitivity: artifact.sensitivity,
                    trust: artifact.trust,
                    validation: artifact.validation,
                    block_ordinal: artifact.source.block_ordinal,
                })
                .collect(),
            diagnostics: self.diagnostics.clone(),
            external_effects: self.external_effects.clone(),
        }
    }
}

/// UI-safe view of one block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiBlockProjection {
    pub ordinal: u32,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub truncated: bool,
    pub validation: ArtifactValidation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<ArtifactId>,
    pub inline_previewable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// UI-safe view of a whole result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiResultProjection {
    pub outcome: ToolResultOutcome,
    pub summary_text: String,
    pub blocks: Vec<UiBlockProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_schema_valid: Option<bool>,
    pub artifact_count: usize,
    pub attempt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_identity_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_tool_name: Option<String>,
    pub external_effects: Vec<ExternalEffect>,
    pub diagnostics: Vec<ToolDiagnostic>,
}

/// Evidence-only view for finalization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinalizerResultProjection {
    pub outcome: ToolResultOutcome,
    pub summary_text: String,
    pub artifact_ids: Vec<ArtifactId>,
    pub mutation_paths: Vec<String>,
    pub unverified_effects: Vec<ExternalEffect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_schema_valid: Option<bool>,
}

/// Artifact lineage kept for audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditArtifactLineage {
    pub artifact_id: ArtifactId,
    pub sha256: String,
    pub byte_length: u64,
    pub kind: ToolArtifactKind,
    pub sensitivity: Sensitivity,
    pub trust: ArtifactTrust,
    pub validation: ArtifactValidation,
    pub block_ordinal: u32,
}

/// Audit view of a whole result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditResultProjection {
    pub outcome: ToolResultOutcome,
    pub protocol_metadata: ToolProtocolMetadata,
    pub artifact_lineage: Vec<AuditArtifactLineage>,
    pub diagnostics: Vec<ToolDiagnostic>,
    pub external_effects: Vec<ExternalEffect>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(kind: ToolArtifactKind, mime: Option<&str>) -> ToolArtifactRef {
        ToolArtifactRef {
            artifact_id: ArtifactId::new("art_0123456789abcdef"),
            kind,
            mime_type: mime.map(str::to_string),
            byte_length: 2_048,
            sha256: "a".repeat(64),
            storage_ref: "artifacts/art_0123456789abcdef/payload".to_string(),
            source: ToolArtifactSource {
                run_id: "run_1".to_string(),
                call_id: "call_1".to_string(),
                server_config_id: Some("srv".to_string()),
                server_identity_hash: Some("hash".to_string()),
                session_hash: Some("session-hash".to_string()),
                remote_tool_name: Some("render".to_string()),
                block_ordinal: 0,
                captured_at: "2026-08-09T00:00:00Z".to_string(),
            },
            original_uri: Some("https://remote.example.com/x.png".to_string()),
            audience: None,
            priority: None,
            last_modified: None,
            sensitivity: Sensitivity::Normal,
            trust: ArtifactTrust::Untrusted,
            validation: ArtifactValidation::Validated,
            validation_detail: None,
        }
    }

    #[test]
    fn outcome_retry_safety_never_includes_indeterminate() {
        assert!(ToolResultOutcome::TimedOutKnownNotSent.is_safely_retryable());
        assert!(ToolResultOutcome::Rejected.is_safely_retryable());
        for outcome in [
            ToolResultOutcome::Indeterminate,
            ToolResultOutcome::Success,
            ToolResultOutcome::Partial,
            ToolResultOutcome::Error,
            ToolResultOutcome::Cancelled,
        ] {
            assert!(
                !outcome.is_safely_retryable(),
                "{outcome:?} must not be reported as safely retryable"
            );
        }
        assert!(ToolResultOutcome::Indeterminate.is_indeterminate());
    }

    #[test]
    fn legacy_status_projection_never_reports_success_for_a_failed_outcome() {
        assert_eq!(
            ToolResultOutcome::Success.to_execution_status(),
            ToolExecutionStatus::Ok
        );
        assert_eq!(
            ToolResultOutcome::Partial.to_execution_status(),
            ToolExecutionStatus::PartialSuccess
        );
        assert_eq!(
            ToolResultOutcome::Rejected.to_execution_status(),
            ToolExecutionStatus::Rejected
        );
        for outcome in [
            ToolResultOutcome::Error,
            ToolResultOutcome::Cancelled,
            ToolResultOutcome::TimedOutKnownNotSent,
            ToolResultOutcome::Indeterminate,
        ] {
            assert_eq!(
                outcome.to_execution_status(),
                ToolExecutionStatus::Error,
                "{outcome:?} must not project to a successful legacy status"
            );
        }
    }

    #[test]
    fn a_mime_claim_is_validated_not_guessed() {
        assert_eq!(
            validated_mime_type(Some("Image/PNG")),
            Some("image/png".to_string())
        );
        assert_eq!(
            validated_mime_type(Some("text/plain; charset=utf-8")),
            Some("text/plain; charset=utf-8".to_string())
        );
        for hostile in [
            "",
            "notamime",
            "image/",
            "/png",
            "image/png\r\nX-Injected: 1",
            "image/png\u{0}",
        ] {
            assert_eq!(
                validated_mime_type(Some(hostile)),
                None,
                "{hostile:?} must be rejected rather than accepted or guessed"
            );
        }
        assert_eq!(validated_mime_type(Some(&"a/".repeat(400))), None);
        assert_eq!(validated_mime_type(None), None);
    }

    #[test]
    fn active_content_is_never_inline_previewable() {
        for mime in [
            "text/html",
            "image/svg+xml",
            "application/pdf",
            "text/javascript",
            "APPLICATION/XHTML+XML",
            "text/html; charset=utf-8",
        ] {
            let block = artifact(ToolArtifactKind::Resource, Some(mime));
            assert!(
                !block.is_inline_previewable(),
                "{mime} must never be offered for inline preview"
            );
        }
        assert!(artifact(ToolArtifactKind::Image, Some("image/png")).is_inline_previewable());
        // An absent or rejected MIME claim is not a licence to preview.
        assert!(!artifact(ToolArtifactKind::Image, None).is_inline_previewable());
        let mut sensitive = artifact(ToolArtifactKind::Image, Some("image/png"));
        sensitive.sensitivity = Sensitivity::Sensitive;
        assert!(!sensitive.is_inline_previewable());
        let mut unvalidated = artifact(ToolArtifactKind::Image, Some("image/png"));
        unvalidated.validation = ArtifactValidation::ClaimRejected;
        assert!(!unvalidated.is_inline_previewable());
    }

    #[test]
    fn structured_content_bounds_reject_hostile_shapes() {
        let mut deep = serde_json::json!(1);
        for _ in 0..MAX_STRUCTURED_JSON_DEPTH + 4 {
            deep = serde_json::json!([deep]);
        }
        assert_eq!(
            StructuredToolContent::bounded(deep),
            Err(StructuredContentRejection::TooDeep)
        );

        let wide = serde_json::Value::Array(
            (0..MAX_STRUCTURED_JSON_NODES + 8)
                .map(|i| serde_json::json!(i))
                .collect(),
        );
        assert_eq!(
            StructuredToolContent::bounded(wide),
            Err(StructuredContentRejection::TooManyNodes)
        );

        let ok = StructuredToolContent::bounded(serde_json::json!({"a": [1, 2, 3]})).unwrap();
        assert_eq!(ok.schema_valid, None);
        let judged = ok.with_schema_verdict(false, Some("missing field".to_string()));
        assert_eq!(judged.schema_valid, Some(false));
        assert_eq!(judged.schema_error.as_deref(), Some("missing field"));
    }

    #[test]
    fn the_model_projection_carries_no_bytes_and_flags_unsafe_outcomes() {
        let envelope = ToolOutputEnvelope {
            outcome: ToolResultOutcome::Indeterminate,
            summary_text: "called the remote tool".to_string(),
            content_blocks: vec![
                ToolContentBlock::Text {
                    meta: ContentBlockMeta::new(0),
                    text: "plain detail".to_string(),
                },
                ToolContentBlock::Image {
                    meta: ContentBlockMeta::new(1),
                    artifact: artifact(ToolArtifactKind::Image, Some("image/png")),
                },
            ],
            structured_content: None,
            artifacts: vec![artifact(ToolArtifactKind::Image, Some("image/png"))],
            mutations: Vec::new(),
            external_effects: Vec::new(),
            protocol_metadata: ToolProtocolMetadata {
                session_hash: Some("session-hash".to_string()),
                ..ToolProtocolMetadata::default()
            },
            diagnostics: Vec::new(),
        };

        let projection = envelope.model_projection();
        assert!(projection.contains("plain detail"));
        assert!(projection.contains("artifact art_0123456789abcdef"));
        assert!(
            projection.contains("may or may not have taken effect"),
            "an indeterminate outcome must be stated to the model: {projection}"
        );
        assert!(
            !projection.contains("session-hash"),
            "protocol metadata must not reach the model: {projection}"
        );
    }

    #[test]
    fn bounds_drop_excess_blocks_and_downgrade_success_to_partial() {
        let blocks: Vec<ToolContentBlock> = (0..MAX_CONTENT_BLOCKS as u32 + 5)
            .map(|ordinal| ToolContentBlock::Text {
                meta: ContentBlockMeta::new(ordinal),
                text: format!("block {ordinal}"),
            })
            .collect();
        let envelope = ToolOutputEnvelope {
            summary_text: "x".repeat(MAX_INLINE_TEXT_BYTES + 100),
            content_blocks: blocks,
            ..ToolOutputEnvelope::default()
        }
        .enforce_bounds();

        assert_eq!(envelope.content_blocks.len(), MAX_CONTENT_BLOCKS);
        assert_eq!(envelope.summary_text.len(), MAX_INLINE_TEXT_BYTES);
        assert_eq!(
            envelope.outcome,
            ToolResultOutcome::Partial,
            "dropping blocks means the result is no longer complete"
        );
        assert!(
            envelope
                .diagnostics
                .iter()
                .any(|d| d.code == "content_blocks_truncated")
        );
        assert!(
            envelope
                .diagnostics
                .iter()
                .any(|d| d.code == "summary_truncated")
        );
    }

    #[test]
    fn truncation_respects_utf8_boundaries() {
        let (kept, truncated) = truncate_utf8("日本語テキスト", 4);
        assert!(truncated);
        assert_eq!(kept, "日");
        let (kept, truncated) = truncate_utf8("abc", 8);
        assert!(!truncated);
        assert_eq!(kept, "abc");
    }

    #[test]
    fn the_finalizer_projection_exposes_unverified_effects() {
        let envelope = ToolOutputEnvelope {
            outcome: ToolResultOutcome::Indeterminate,
            summary_text: "posted".to_string(),
            external_effects: vec![
                ExternalEffect {
                    kind: "mcp_tool_call".to_string(),
                    target: "deploy".to_string(),
                    indeterminate: true,
                },
                ExternalEffect {
                    kind: "mcp_tool_call".to_string(),
                    target: "read".to_string(),
                    indeterminate: false,
                },
            ],
            artifacts: vec![artifact(ToolArtifactKind::Text, Some("text/plain"))],
            ..ToolOutputEnvelope::default()
        };

        let projection = envelope.finalizer_projection();
        assert_eq!(projection.unverified_effects.len(), 1);
        assert_eq!(projection.unverified_effects[0].target, "deploy");
        assert_eq!(projection.artifact_ids.len(), 1);
        assert!(envelope.is_indeterminate());
    }

    #[test]
    fn the_ui_projection_marks_unsafe_artifacts_without_hiding_them() {
        let envelope = ToolOutputEnvelope {
            content_blocks: vec![ToolContentBlock::EmbeddedResource {
                meta: ContentBlockMeta::new(3),
                uri: Some("https://remote.example.com/page.html".to_string()),
                artifact: artifact(ToolArtifactKind::Resource, Some("text/html")),
                preview: None,
            }],
            ..ToolOutputEnvelope::default()
        };

        let projection = envelope.ui_projection();
        assert_eq!(projection.blocks.len(), 1);
        assert_eq!(projection.blocks[0].ordinal, 3);
        assert_eq!(projection.blocks[0].kind, "embedded_resource");
        assert!(
            !projection.blocks[0].inline_previewable,
            "remote HTML must not be presented as previewable UI"
        );
        assert!(projection.blocks[0].artifact_id.is_some());
    }

    #[test]
    fn the_audit_projection_keeps_lineage_and_hashes() {
        let envelope = ToolOutputEnvelope {
            artifacts: vec![artifact(ToolArtifactKind::Image, Some("image/png"))],
            protocol_metadata: ToolProtocolMetadata {
                protocol: Some("mcp".to_string()),
                attempt_count: 2,
                ..ToolProtocolMetadata::default()
            },
            ..ToolOutputEnvelope::default()
        };

        let projection = envelope.audit_projection();
        assert_eq!(projection.artifact_lineage.len(), 1);
        assert_eq!(projection.artifact_lineage[0].sha256.len(), 64);
        assert_eq!(projection.protocol_metadata.attempt_count, 2);
    }

    #[test]
    fn an_unknown_block_is_preserved_rather_than_dropped() {
        let block = ToolContentBlock::Unknown {
            meta: ContentBlockMeta::new(7),
            declared_type: "video".to_string(),
            retained: Some("{\"frames\":2}".to_string()),
        };
        let text = block.model_text();
        assert!(text.contains("unsupported block \"video\""));
        assert!(text.contains("frames"));
        assert_eq!(block.kind_label(), "unknown");
    }

    #[test]
    fn a_uri_claim_is_recorded_but_bounded() {
        assert_eq!(
            recorded_uri_claim(Some(" https://example.com/x ")),
            Some("https://example.com/x".to_string())
        );
        assert_eq!(recorded_uri_claim(Some("bad\nuri")), None);
        assert_eq!(
            recorded_uri_claim(Some(&"x".repeat(MAX_URI_BYTES + 1))),
            None
        );
        assert_eq!(recorded_uri_claim(Some("   ")), None);
    }

    #[test]
    fn the_envelope_round_trips_through_serde_with_defaults() {
        let envelope = ToolOutputEnvelope::text("done");
        let encoded = serde_json::to_string(&envelope).unwrap();
        // Empty collections stay off the wire so an old reader sees a minimal
        // object rather than a pile of nulls.
        assert_eq!(
            encoded,
            "{\"outcome\":\"success\",\"summary_text\":\"done\",\"protocol_metadata\":{}}"
        );
        let decoded: ToolOutputEnvelope = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, envelope);
        // An older record with only the text field still loads.
        let minimal: ToolOutputEnvelope =
            serde_json::from_str("{\"summary_text\":\"legacy\"}").unwrap();
        assert_eq!(minimal.outcome, ToolResultOutcome::Success);
        assert!(minimal.content_blocks.is_empty());
    }
}
