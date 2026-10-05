//! Tests for the MCP result to envelope mapping.

use rove_core::{ArtifactTrust, ToolArtifactKind, ToolContentBlock, ToolResultOutcome};
use serde_json::json;

use super::result_mapping::{McpResultContext, decode_base64_strict, envelope_from_mcp_result};
use crate::state::tool_artifacts::{MAX_SINGLE_ARTIFACT_BYTES, ToolArtifactStore};

fn context<'a>(store: Option<&'a ToolArtifactStore>) -> McpResultContext<'a> {
    McpResultContext {
        call_id: "call_1".to_string(),
        remote_tool_name: "render".to_string(),
        server_config_id: "srv".to_string(),
        server_identity_hash: "identity-hash".to_string(),
        protocol_version: "2025-06-18".to_string(),
        capability_snapshot_id: Some("catalog-hash".to_string()),
        session_hash: Some("session-hash".to_string()),
        attempt_count: 1,
        duration_ms: Some(12),
        output_schema: None,
        artifacts: store,
        captured_at: "2026-08-09T00:00:00Z".to_string(),
    }
}

fn store() -> (tempfile::TempDir, ToolArtifactStore) {
    let dir = tempfile::TempDir::new().unwrap();
    let store = ToolArtifactStore::new(dir.path().join("runs/run_map"));
    (dir, store)
}

/// Minimal standard base64 encoder for building fixtures.
fn encode_base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18) as usize & 63] as char);
        out.push(ALPHABET[(triple >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(triple >> 6) as usize & 63] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[triple as usize & 63] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[test]
fn the_base64_decoder_refuses_anything_malformed() {
    assert_eq!(decode_base64_strict(&encode_base64(b"hi")).unwrap(), b"hi");
    assert_eq!(
        decode_base64_strict(&encode_base64(b"abc")).unwrap(),
        b"abc"
    );
    assert_eq!(
        decode_base64_strict(&encode_base64(&[0u8, 255, 128])).unwrap(),
        vec![0u8, 255, 128]
    );

    for hostile in [
        "",         // empty
        "aGk",      // wrong length
        "aGk*",     // invalid character
        "aG k=",    // embedded whitespace
        "a=k=",     // padding not at the end
        "====",     // all padding
        "aGkxx===", // too much padding
        "=GkA",     // leading padding
    ] {
        assert!(
            decode_base64_strict(hostile).is_none(),
            "{hostile:?} must be refused rather than silently decoded"
        );
    }
}

#[tokio::test]
async fn text_blocks_project_to_the_summary_in_order() {
    let (_dir, store) = store();
    let result = json!({
        "content": [
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"}
        ]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert_eq!(envelope.outcome, ToolResultOutcome::Success);
    assert_eq!(envelope.summary_text, "first\nsecond");
    assert_eq!(envelope.content_blocks.len(), 2);
    assert_eq!(envelope.content_blocks[0].meta().ordinal, 0);
    assert_eq!(envelope.content_blocks[1].meta().ordinal, 1);
    assert!(envelope.artifacts.is_empty());
}

#[tokio::test]
async fn an_image_block_becomes_an_artifact_and_never_inlines_base64() {
    let (_dir, store) = store();
    let payload = b"\x89PNG\r\n\x1a\nbinary";
    let result = json!({
        "content": [{
            "type": "image",
            "mimeType": "image/png",
            "data": encode_base64(payload)
        }]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert_eq!(envelope.outcome, ToolResultOutcome::Success);
    assert_eq!(envelope.artifacts.len(), 1);
    let artifact = &envelope.artifacts[0];
    assert_eq!(artifact.kind, ToolArtifactKind::Image);
    assert_eq!(artifact.mime_type.as_deref(), Some("image/png"));
    assert_eq!(artifact.byte_length, payload.len() as u64);
    assert_eq!(artifact.trust, ArtifactTrust::Untrusted);
    // The stored bytes are exactly what the server sent.
    assert_eq!(
        store.get(&artifact.artifact_id).await.unwrap(),
        payload.to_vec()
    );
    // No base64 anywhere in what a model would see.
    let projection = envelope.model_projection();
    assert!(!projection.contains(&encode_base64(payload)));
    assert!(projection.contains(artifact.artifact_id.as_str()));
}

#[tokio::test]
async fn a_remote_is_error_outranks_its_content() {
    let (_dir, store) = store();
    let result = json!({
        "isError": true,
        "content": [{"type": "text", "text": "looks fine"}]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert_eq!(envelope.outcome, ToolResultOutcome::Error);
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|d| d.code == "mcp_tool_reported_error"),
        "the remote failure must be classified, not just implied"
    );
    // The text is still preserved for context.
    assert_eq!(envelope.summary_text, "looks fine");
}

#[tokio::test]
async fn an_unknown_block_type_is_retained_not_dropped() {
    let (_dir, store) = store();
    let result = json!({
        "content": [
            {"type": "text", "text": "before"},
            {"type": "hologram", "frames": 3},
            {"type": "text", "text": "after"}
        ]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert_eq!(envelope.content_blocks.len(), 3);
    let ToolContentBlock::Unknown {
        declared_type,
        retained,
        meta,
    } = &envelope.content_blocks[1]
    else {
        panic!("expected the middle block to be retained as unknown");
    };
    assert_eq!(declared_type, "hologram");
    assert_eq!(meta.ordinal, 1, "the original position must survive");
    assert!(retained.as_deref().unwrap().contains("frames"));
    // Ordinals of the surrounding blocks are unchanged.
    assert_eq!(envelope.content_blocks[2].meta().ordinal, 2);
}

#[tokio::test]
async fn a_non_base64_binary_payload_is_refused_and_degrades_to_partial() {
    let (_dir, store) = store();
    let result = json!({
        "content": [{
            "type": "image",
            "mimeType": "image/png",
            "data": "not base64 at all!!"
        }]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert_eq!(envelope.outcome, ToolResultOutcome::Partial);
    assert!(envelope.artifacts.is_empty());
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|d| d.code == "mcp_binary_payload_not_base64")
    );
}

#[tokio::test]
async fn a_payload_over_the_quota_is_refused_and_degrades_to_partial() {
    let (_dir, store) = store();
    let oversized = vec![9u8; MAX_SINGLE_ARTIFACT_BYTES as usize + 16];
    let result = json!({
        "content": [{
            "type": "image",
            "mimeType": "image/png",
            "data": encode_base64(&oversized)
        }]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert_eq!(envelope.outcome, ToolResultOutcome::Partial);
    assert!(envelope.artifacts.is_empty());
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|d| d.code == "artifact_rejected")
    );
    // The refusal is durable in the ledger, not only in this envelope.
    assert_eq!(store.ledger().await.unwrap().len(), 1);
}

#[tokio::test]
async fn without_an_artifact_store_a_binary_payload_is_refused_not_inlined() {
    let payload = b"binary bytes";
    let encoded = encode_base64(payload);
    let result = json!({
        "content": [{
            "type": "image",
            "mimeType": "image/png",
            "data": encoded.clone()
        }]
    });

    let envelope = envelope_from_mcp_result(&result, &context(None)).await;

    assert_eq!(envelope.outcome, ToolResultOutcome::Partial);
    assert!(envelope.artifacts.is_empty());
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|d| d.code == "artifact_store_unavailable")
    );
    assert!(
        !envelope.model_projection().contains(&encoded),
        "a refused payload must never fall back to inline base64"
    );
}

#[tokio::test]
async fn an_active_content_text_resource_is_stored_without_a_preview() {
    let (_dir, store) = store();
    let html = "<script>alert(1)</script>";
    let result = json!({
        "content": [{
            "type": "resource",
            "resource": {
                "uri": "https://remote.example.com/page.html",
                "mimeType": "text/html",
                "text": html
            }
        }]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    let ToolContentBlock::EmbeddedResource {
        preview, artifact, ..
    } = &envelope.content_blocks[0]
    else {
        panic!("expected an embedded resource block");
    };
    assert!(
        preview.is_none(),
        "remote HTML must not be offered as a preview"
    );
    assert!(!artifact.is_inline_previewable());
    // The bytes are still retained so a user can download them deliberately.
    assert_eq!(
        store.get(&artifact.artifact_id).await.unwrap(),
        html.as_bytes().to_vec()
    );
}

#[tokio::test]
async fn a_plain_text_resource_keeps_a_bounded_preview() {
    let (_dir, store) = store();
    let result = json!({
        "content": [{
            "type": "resource",
            "resource": {
                "uri": "file:///notes.txt",
                "mimeType": "text/plain",
                "text": "plain notes"
            }
        }]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    let ToolContentBlock::EmbeddedResource { preview, uri, .. } = &envelope.content_blocks[0]
    else {
        panic!("expected an embedded resource block");
    };
    assert_eq!(preview.as_deref(), Some("plain notes"));
    assert_eq!(uri.as_deref(), Some("file:///notes.txt"));
}

#[tokio::test]
async fn structured_content_is_bounded_and_its_rejection_is_recorded() {
    let (_dir, store) = store();
    let mut deep = json!(1);
    for _ in 0..40 {
        deep = json!([deep]);
    }
    let result = json!({
        "content": [{"type": "text", "text": "ok"}],
        "structuredContent": deep
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert!(envelope.structured_content.is_none());
    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|d| d.code == "mcp_structured_content_rejected")
    );
    // Rejecting structured content does not invalidate the text result.
    assert_eq!(envelope.summary_text, "ok");
}

#[tokio::test]
async fn well_formed_structured_content_is_kept() {
    let (_dir, store) = store();
    let result = json!({
        "content": [{"type": "text", "text": "ok"}],
        "structuredContent": {"rows": [1, 2, 3]}
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    let structured = envelope.structured_content.as_ref().unwrap();
    assert_eq!(structured.value["rows"][2], 3);
}

#[tokio::test]
async fn declared_output_schema_is_enforced_before_success_is_reported() {
    let (_dir, store) = store();
    let schema = json!({
        "type": "object",
        "properties": { "ok": { "type": "boolean" } },
        "required": ["ok"],
        "additionalProperties": false
    });
    let mut valid_context = context(Some(&store));
    valid_context.output_schema = Some(&schema);
    let valid = envelope_from_mcp_result(
        &json!({
            "content": [{"type": "text", "text": "ok"}],
            "structuredContent": {"ok": true}
        }),
        &valid_context,
    )
    .await;
    assert_eq!(valid.outcome, ToolResultOutcome::Success);
    assert_eq!(
        valid
            .structured_content
            .as_ref()
            .and_then(|content| content.schema_valid),
        Some(true)
    );

    let invalid = envelope_from_mcp_result(
        &json!({
            "content": [{"type": "text", "text": "claimed success"}],
            "structuredContent": {"ok": "not a boolean"}
        }),
        &valid_context,
    )
    .await;
    assert_eq!(invalid.outcome, ToolResultOutcome::Error);
    assert_eq!(
        invalid
            .structured_content
            .as_ref()
            .and_then(|content| content.schema_valid),
        Some(false)
    );
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "mcp_output_schema_validation_failed" })
    );

    let missing = envelope_from_mcp_result(
        &json!({"content": [{"type": "text", "text": "no structured result"}]}),
        &valid_context,
    )
    .await;
    assert_eq!(missing.outcome, ToolResultOutcome::Error);
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == "mcp_structured_content_missing" })
    );
}

#[tokio::test]
async fn a_result_without_content_is_reported_as_a_protocol_violation() {
    let (_dir, store) = store();
    let result = json!({"unexpected": true});

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert!(
        envelope
            .diagnostics
            .iter()
            .any(|d| d.code == "mcp_result_missing_content"),
        "a malformed result must not look like an empty success"
    );
}

#[tokio::test]
async fn protocol_metadata_carries_hashes_and_never_a_readable_session() {
    let (_dir, store) = store();
    let result = json!({"content": [{"type": "text", "text": "ok"}]});

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    let metadata = &envelope.protocol_metadata;
    assert_eq!(metadata.protocol.as_deref(), Some("mcp"));
    assert_eq!(metadata.protocol_version.as_deref(), Some("2025-06-18"));
    assert_eq!(metadata.remote_tool_name.as_deref(), Some("render"));
    assert_eq!(metadata.attempt_count, 1);
    // Only the hash is retained, and it never reaches the model.
    assert_eq!(metadata.session_hash.as_deref(), Some("session-hash"));
    assert!(!envelope.model_projection().contains("session-hash"));
    // Every MCP call is recorded as an external effect.
    assert_eq!(envelope.external_effects.len(), 1);
    assert_eq!(envelope.external_effects[0].kind, "mcp_tool_call");
    assert_eq!(envelope.external_effects[0].target, "srv/render");
}

#[tokio::test]
async fn a_rejected_mime_claim_does_not_stop_the_payload_being_stored() {
    let (_dir, store) = store();
    let payload = b"some bytes";
    let result = json!({
        "content": [{
            "type": "image",
            "mimeType": "totally bogus",
            "data": encode_base64(payload)
        }]
    });

    let envelope = envelope_from_mcp_result(&result, &context(Some(&store))).await;

    assert_eq!(envelope.artifacts.len(), 1);
    let artifact = &envelope.artifacts[0];
    assert_eq!(artifact.mime_type, None);
    assert!(!artifact.is_inline_previewable());
    assert_eq!(
        store.get(&artifact.artifact_id).await.unwrap(),
        payload.to_vec()
    );
}
