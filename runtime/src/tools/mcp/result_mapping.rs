//! Maps an MCP `tools/call` result into the shared tool result envelope.
//!
//! This is the producer that makes the rich contract real: without it, an
//! image, audio, or embedded resource returned by an MCP server would collapse
//! to text and its bytes would be lost.
//!
//! The mapping is deliberately conservative:
//!
//! - Every block keeps its original ordinal, so a truncated or promoted block
//!   can still be traced back to what the server sent.
//! - A block type this build does not model becomes `Unknown` with a bounded
//!   body, never a dropped block.
//! - Binary payloads go to the durable artifact store and the envelope keeps
//!   only a reference. When no store is available the payload is refused and
//!   the loss is recorded, rather than being inlined as base64.
//! - `isError` outranks everything: a server that reports failure is failed,
//!   whatever its content blocks say.

use rove_core::{
    ArtifactTrust, ContentBlockMeta, ExternalEffect, MAX_BLOCK_PREVIEW_BYTES,
    MAX_INLINE_TEXT_BYTES, MAX_UNKNOWN_BLOCK_BYTES, Sensitivity, StructuredToolContent,
    ToolArtifactKind, ToolArtifactSource, ToolContentBlock, ToolDiagnostic, ToolErrorDomain,
    ToolOutputEnvelope, ToolProtocolMetadata, ToolResultOutcome, mime_type_is_active_content,
    truncate_utf8, validated_mime_type,
};
use serde_json::Value;

use crate::state::tool_artifacts::{ArtifactClaim, ToolArtifactStore};

/// Everything the mapping needs that does not come from the result itself.
pub struct McpResultContext<'a> {
    pub call_id: String,
    pub remote_tool_name: String,
    pub server_config_id: String,
    pub server_identity_hash: String,
    pub protocol_version: String,
    pub capability_snapshot_id: Option<String>,
    pub session_hash: Option<String>,
    pub attempt_count: u32,
    pub duration_ms: Option<u64>,
    /// Locally validated output schema pinned with the tool catalog.
    pub output_schema: Option<&'a Value>,
    /// Durable artifact authority. `None` in an embedding without a run
    /// directory, in which case binary payloads are refused rather than
    /// inlined.
    pub artifacts: Option<&'a ToolArtifactStore>,
    pub captured_at: String,
}

impl McpResultContext<'_> {
    fn protocol_metadata(&self) -> ToolProtocolMetadata {
        ToolProtocolMetadata {
            protocol: Some("mcp".to_string()),
            server_config_id: Some(self.server_config_id.clone()),
            server_identity_hash: Some(self.server_identity_hash.clone()),
            protocol_version: Some(self.protocol_version.clone()),
            capability_snapshot_id: self.capability_snapshot_id.clone(),
            remote_tool_name: Some(self.remote_tool_name.clone()),
            request_id_hash: None,
            connection_id: None,
            session_hash: self.session_hash.clone(),
            attempt_count: self.attempt_count,
            duration_ms: self.duration_ms,
        }
    }

    fn source(&self, block_ordinal: u32) -> ToolArtifactSource {
        ToolArtifactSource {
            run_id: self
                .artifacts
                .map(ToolArtifactStore::run_id)
                .unwrap_or_default(),
            call_id: self.call_id.clone(),
            server_config_id: Some(self.server_config_id.clone()),
            server_identity_hash: Some(self.server_identity_hash.clone()),
            session_hash: self.session_hash.clone(),
            remote_tool_name: Some(self.remote_tool_name.clone()),
            block_ordinal,
            captured_at: self.captured_at.clone(),
        }
    }
}

/// Builds an envelope from an MCP `tools/call` result.
pub async fn envelope_from_mcp_result(
    result: &Value,
    ctx: &McpResultContext<'_>,
) -> ToolOutputEnvelope {
    let mut blocks: Vec<ToolContentBlock> = Vec::new();
    let mut diagnostics: Vec<ToolDiagnostic> = Vec::new();
    let mut artifacts = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();
    let mut artifact_failed = false;

    match result.get("content").and_then(Value::as_array) {
        Some(content) => {
            for (index, item) in content.iter().enumerate() {
                let ordinal = index as u32;
                let mapped = map_block(item, ordinal, ctx, &mut diagnostics).await;
                if mapped.artifact_failed {
                    artifact_failed = true;
                }
                if let Some(block) = mapped.block {
                    if let ToolContentBlock::Text { text, .. } = &block {
                        text_parts.push(text.clone());
                    }
                    if let Some(artifact) = block.artifact() {
                        artifacts.push(artifact.clone());
                    }
                    blocks.push(block);
                }
            }
        }
        None => {
            // A result without a content array violates the protocol. Say so
            // rather than silently treating it as an empty success.
            diagnostics.push(ToolDiagnostic::new(
                ToolErrorDomain::Protocol,
                "mcp_result_missing_content",
                "the MCP result did not contain a content array",
            ));
        }
    }

    let mut structured_schema_failed = false;
    let structured = match result.get("structuredContent") {
        Some(value) => match StructuredToolContent::bounded(value.clone()) {
            Ok(structured) => {
                if let Some(schema) = ctx.output_schema {
                    match rove_core::validate_tool_args(schema, value) {
                        Ok(()) => Some(structured.with_schema_verdict(true, None)),
                        Err(error) => {
                            structured_schema_failed = true;
                            diagnostics.push(ToolDiagnostic::new(
                                ToolErrorDomain::OutputSchema,
                                "mcp_output_schema_validation_failed",
                                &error.to_string(),
                            ));
                            Some(structured.with_schema_verdict(false, Some(error.to_string())))
                        }
                    }
                } else {
                    Some(structured)
                }
            }
            Err(rejection) => {
                structured_schema_failed = ctx.output_schema.is_some();
                diagnostics.push(ToolDiagnostic::new(
                    ToolErrorDomain::OutputSchema,
                    "mcp_structured_content_rejected",
                    &rejection.to_string(),
                ));
                None
            }
        },
        None => {
            if ctx.output_schema.is_some() {
                structured_schema_failed = true;
                diagnostics.push(ToolDiagnostic::new(
                    ToolErrorDomain::OutputSchema,
                    "mcp_structured_content_missing",
                    "the tool declared outputSchema but returned no structuredContent",
                ));
            }
            None
        }
    };

    let summary = if text_parts.is_empty() {
        String::new()
    } else {
        truncate_utf8(&text_parts.join("\n"), MAX_INLINE_TEXT_BYTES).0
    };

    // Error precedence: a remote `isError` is terminal. A retained-payload
    // failure only downgrades an otherwise successful result to partial,
    // because the tool did run and its text may still be usable.
    let remote_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let outcome = if remote_error || structured_schema_failed {
        ToolResultOutcome::Error
    } else if artifact_failed {
        ToolResultOutcome::Partial
    } else {
        ToolResultOutcome::Success
    };
    if remote_error {
        diagnostics.push(ToolDiagnostic::new(
            ToolErrorDomain::RemoteTool,
            "mcp_tool_reported_error",
            "the remote tool reported isError",
        ));
    }

    ToolOutputEnvelope {
        outcome,
        summary_text: summary,
        content_blocks: blocks,
        structured_content: structured,
        artifacts,
        mutations: Vec::new(),
        // An MCP tool call is an effect outside this workspace. Recording it
        // keeps a Finalizer from treating a remote action as locally verified.
        external_effects: vec![ExternalEffect {
            kind: "mcp_tool_call".to_string(),
            target: format!("{}/{}", ctx.server_config_id, ctx.remote_tool_name),
            indeterminate: false,
        }],
        protocol_metadata: ctx.protocol_metadata(),
        diagnostics,
    }
    .enforce_bounds()
}

/// Preserve a committed remote call whose effect cannot be established.
pub fn indeterminate_envelope(ctx: &McpResultContext<'_>, safe_detail: &str) -> ToolOutputEnvelope {
    let detail = truncate_utf8(safe_detail, 512).0;
    ToolOutputEnvelope {
        outcome: ToolResultOutcome::Indeterminate,
        summary_text: "the MCP tool call ended after dispatch without a verifiable result"
            .to_string(),
        external_effects: vec![ExternalEffect {
            kind: "mcp_tool_call".to_string(),
            target: format!("{}/{}", ctx.server_config_id, ctx.remote_tool_name),
            indeterminate: true,
        }],
        protocol_metadata: ctx.protocol_metadata(),
        diagnostics: vec![ToolDiagnostic::new(
            ToolErrorDomain::Transport,
            "mcp_tool_effect_indeterminate",
            &detail,
        )],
        ..ToolOutputEnvelope::default()
    }
    .enforce_bounds()
}

struct MappedBlock {
    block: Option<ToolContentBlock>,
    artifact_failed: bool,
}

impl MappedBlock {
    fn kept(block: ToolContentBlock) -> Self {
        Self {
            block: Some(block),
            artifact_failed: false,
        }
    }

    fn failed(block: ToolContentBlock) -> Self {
        Self {
            block: Some(block),
            artifact_failed: true,
        }
    }
}

async fn map_block(
    item: &Value,
    ordinal: u32,
    ctx: &McpResultContext<'_>,
    diagnostics: &mut Vec<ToolDiagnostic>,
) -> MappedBlock {
    let declared_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut meta = ContentBlockMeta::new(ordinal);
    meta.mime_type = validated_mime_type(item.get("mimeType").and_then(Value::as_str));

    match declared_type.as_str() {
        "text" => {
            let raw = item.get("text").and_then(Value::as_str).unwrap_or_default();
            let (text, truncated) = truncate_utf8(raw, MAX_INLINE_TEXT_BYTES);
            meta.truncated = truncated;
            MappedBlock::kept(ToolContentBlock::Text { meta, text })
        }
        "image" | "audio" => {
            let kind = if declared_type == "image" {
                ToolArtifactKind::Image
            } else {
                ToolArtifactKind::Audio
            };
            store_binary_block(item, "data", kind, meta, ordinal, ctx, diagnostics, None).await
        }
        "resource_link" => {
            match rove_core::recorded_uri_claim(item.get("uri").and_then(Value::as_str)) {
                Some(uri) => MappedBlock::kept(ToolContentBlock::ResourceLink {
                    meta,
                    uri,
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .map(|name| truncate_utf8(name, MAX_BLOCK_PREVIEW_BYTES).0),
                    description: item
                        .get("description")
                        .and_then(Value::as_str)
                        .map(|value| truncate_utf8(value, MAX_BLOCK_PREVIEW_BYTES).0),
                }),
                None => {
                    diagnostics.push(ToolDiagnostic::new(
                        ToolErrorDomain::Protocol,
                        "mcp_resource_link_uri_rejected",
                        "a resource link did not carry a usable URI",
                    ));
                    MappedBlock::kept(unknown_block(meta, &declared_type, item))
                }
            }
        }
        "resource" => map_embedded_resource(item, meta, ordinal, ctx, diagnostics).await,
        _ => {
            diagnostics.push(ToolDiagnostic::new(
                ToolErrorDomain::Protocol,
                "mcp_unsupported_content_block",
                &format!("retained an unsupported content block of type {declared_type:?}"),
            ));
            MappedBlock::kept(unknown_block(meta, &declared_type, item))
        }
    }
}

/// An embedded resource can be text or binary. Text stays inline within its
/// bound; binary goes to the artifact store.
async fn map_embedded_resource(
    item: &Value,
    mut meta: ContentBlockMeta,
    ordinal: u32,
    ctx: &McpResultContext<'_>,
    diagnostics: &mut Vec<ToolDiagnostic>,
) -> MappedBlock {
    let resource = item.get("resource").unwrap_or(&Value::Null);
    let uri = rove_core::recorded_uri_claim(resource.get("uri").and_then(Value::as_str));
    if meta.mime_type.is_none() {
        meta.mime_type = validated_mime_type(resource.get("mimeType").and_then(Value::as_str));
    }

    if let Some(text) = resource.get("text").and_then(Value::as_str) {
        // A text resource is still stored, so it can be downloaded whole, but
        // it also carries a bounded preview for the model and the UI.
        let (preview, truncated) = truncate_utf8(text, MAX_BLOCK_PREVIEW_BYTES);
        meta.truncated = truncated;
        let mime_is_active = meta
            .mime_type
            .as_deref()
            .is_some_and(mime_type_is_active_content);
        return match store_bytes(
            text.as_bytes(),
            ToolArtifactKind::Resource,
            &meta,
            ordinal,
            ctx,
            diagnostics,
        )
        .await
        {
            Some(artifact) => MappedBlock::kept(ToolContentBlock::EmbeddedResource {
                meta,
                uri,
                artifact,
                // Never preview active content, even as plain text: a UI that
                // renders it would be doing exactly what the MIME check
                // exists to prevent.
                preview: (!mime_is_active).then_some(preview),
            }),
            None => MappedBlock::failed(unknown_block_text(meta, "resource", &preview)),
        };
    }

    store_binary_block(
        resource,
        "blob",
        ToolArtifactKind::Resource,
        meta,
        ordinal,
        ctx,
        diagnostics,
        uri,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn store_binary_block(
    holder: &Value,
    field: &str,
    kind: ToolArtifactKind,
    meta: ContentBlockMeta,
    ordinal: u32,
    ctx: &McpResultContext<'_>,
    diagnostics: &mut Vec<ToolDiagnostic>,
    uri: Option<String>,
) -> MappedBlock {
    let encoded = holder
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(bytes) = decode_base64_strict(encoded) else {
        diagnostics.push(ToolDiagnostic::new(
            ToolErrorDomain::Protocol,
            "mcp_binary_payload_not_base64",
            "a binary content block was not valid base64 and was refused",
        ));
        return MappedBlock::failed(unknown_block_text(
            meta,
            "binary",
            "the payload was not valid base64",
        ));
    };

    match store_bytes(&bytes, kind, &meta, ordinal, ctx, diagnostics).await {
        Some(artifact) => MappedBlock::kept(match kind {
            ToolArtifactKind::Image => ToolContentBlock::Image { meta, artifact },
            ToolArtifactKind::Audio => ToolContentBlock::Audio { meta, artifact },
            _ => ToolContentBlock::EmbeddedResource {
                meta,
                uri,
                artifact,
                // A binary resource has no text preview to offer.
                preview: None,
            },
        }),
        None => MappedBlock::failed(unknown_block_text(
            meta,
            "binary",
            "the payload could not be retained",
        )),
    }
}

/// Stores bytes, reporting a refusal as a diagnostic.
///
/// Returns `None` when the payload was not retained, so the caller can decide
/// how to degrade instead of assuming the artifact exists.
async fn store_bytes(
    bytes: &[u8],
    kind: ToolArtifactKind,
    meta: &ContentBlockMeta,
    ordinal: u32,
    ctx: &McpResultContext<'_>,
    diagnostics: &mut Vec<ToolDiagnostic>,
) -> Option<rove_core::ToolArtifactRef> {
    let Some(store) = ctx.artifacts else {
        // Without a durable store the only alternatives are inlining base64
        // into the model prompt or losing the payload silently. Both are
        // worse than refusing it and saying so.
        diagnostics.push(ToolDiagnostic::new(
            ToolErrorDomain::Artifact,
            "artifact_store_unavailable",
            &format!(
                "refused a {} byte payload because this embedding has no artifact store",
                bytes.len()
            ),
        ));
        return None;
    };

    match store
        .put(
            kind,
            bytes,
            ctx.source(ordinal),
            ArtifactClaim {
                mime_type: meta.mime_type.clone(),
                ..ArtifactClaim::default()
            },
            Sensitivity::Normal,
            // Anything a remote server produced stays untrusted, whatever it
            // claims about itself.
            ArtifactTrust::Untrusted,
        )
        .await
    {
        Ok(artifact) => Some(artifact),
        Err(error) => {
            diagnostics.push(ToolDiagnostic::new(
                ToolErrorDomain::Artifact,
                "artifact_rejected",
                &error.to_string(),
            ));
            None
        }
    }
}

fn unknown_block(meta: ContentBlockMeta, declared_type: &str, item: &Value) -> ToolContentBlock {
    let rendered = serde_json::to_string(item).unwrap_or_default();
    let (retained, _) = truncate_utf8(&rendered, MAX_UNKNOWN_BLOCK_BYTES);
    ToolContentBlock::Unknown {
        meta,
        declared_type: truncate_utf8(declared_type, 64).0,
        retained: (!retained.is_empty()).then_some(retained),
    }
}

fn unknown_block_text(
    meta: ContentBlockMeta,
    declared_type: &str,
    detail: &str,
) -> ToolContentBlock {
    ToolContentBlock::Unknown {
        meta,
        declared_type: declared_type.to_string(),
        retained: Some(truncate_utf8(detail, MAX_UNKNOWN_BLOCK_BYTES).0),
    }
}

/// Decodes standard base64, rejecting anything malformed.
///
/// Strict on purpose. A lenient decoder that skipped stray characters would
/// produce bytes the server never sent, and those bytes would then be hashed
/// and stored as if they were authentic. Whitespace is not accepted, padding
/// must be correct, and padding may only appear at the very end.
pub(crate) fn decode_base64_strict(encoded: &str) -> Option<Vec<u8>> {
    if encoded.is_empty() || !encoded.len().is_multiple_of(4) {
        return None;
    }
    let bytes = encoded.as_bytes();
    let padding = bytes.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return None;
    }
    // Padding is only legal as the final one or two characters.
    if bytes[..bytes.len() - padding].contains(&b'=') {
        return None;
    }

    let value_of = |b: u8| -> Option<u32> {
        match b {
            b'A'..=b'Z' => Some((b - b'A') as u32),
            b'a'..=b'z' => Some((b - b'a' + 26) as u32),
            b'0'..=b'9' => Some((b - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };

    let mut out = Vec::with_capacity(encoded.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let is_last = std::ptr::eq(chunk.as_ptr(), bytes[bytes.len() - 4..].as_ptr());
        let mut accumulator = 0u32;
        let mut significant = 0usize;
        for (index, &byte) in chunk.iter().enumerate() {
            if byte == b'=' {
                // Only the final chunk may be padded.
                if !is_last || index < 2 {
                    return None;
                }
                accumulator <<= 6;
                continue;
            }
            accumulator = (accumulator << 6) | value_of(byte)?;
            significant += 1;
        }
        let produced = match significant {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None,
        };
        let full = accumulator.to_be_bytes();
        out.extend_from_slice(&full[1..1 + produced]);
    }
    Some(out)
}
