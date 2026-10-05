use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;

use crate::state::tool_artifacts::is_valid_artifact_id;
use crate::tools::runtime_context::runtime_tool_services;
use rove_core::{
    ArtifactId, Sensitivity, Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput,
};

const MAX_ARTIFACT_RESOLVE_BYTES: usize = 64 * 1024;

/// Bounded resolver for canonical durable Tool Artifact references.
/// An opaque reference is data, never a filesystem path or capability grant.
#[derive(Default)]
pub struct ResolveToolArtifactTool;

#[derive(Serialize)]
struct ArtifactResolution {
    artifact_id: String,
    status: &'static str,
    content: String,
    offset: usize,
    end: usize,
    total_bytes: usize,
    sha256: String,
    mime_type: Option<String>,
    truncated: bool,
    continuation: Option<String>,
}

#[async_trait]
impl Tool for ResolveToolArtifactTool {
    fn schema(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "resolve_tool_artifact".to_string(),
            description: "Resolve a bounded UTF-8 range from a canonical local Tool Artifact reference. Missing, expired, sensitive, non-text, malformed, or oversized references fail explicitly.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "artifact_id": { "type": "string", "minLength": 36, "maxLength": 36 },
                    "offset": { "type": "integer", "minimum": 0, "maximum": 8388608, "default": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 65536, "default": 65536 }
                },
                "required": ["artifact_id"],
                "additionalProperties": false
            }),
            destructive: false,
            parallel_safe: true,
            capability_id: Some("runtime.artifact.read".to_string()),
            capability: None,
        }
    }

    async fn execute(&self, args: Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError> {
        let raw_id = args
            .get("artifact_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs {
                reason: "Missing required argument: artifact_id".to_string(),
            })?;
        if !is_valid_artifact_id(raw_id) {
            return Err(ToolError::ArtifactUnavailable {
                reason: "artifact reference is malformed".to_string(),
            });
        }
        let services = runtime_tool_services(ctx)?;
        let store =
            services
                .tool_artifacts
                .as_ref()
                .ok_or_else(|| ToolError::ArtifactUnavailable {
                    reason: "canonical artifact authority is unavailable in this embedding"
                        .to_string(),
                })?;
        let artifact_id = ArtifactId::new(raw_id);
        let metadata =
            store
                .metadata(&artifact_id)
                .await
                .map_err(|_| ToolError::ArtifactUnavailable {
                    reason: "artifact reference is missing".to_string(),
                })?;
        if metadata.sensitivity == Sensitivity::Sensitive {
            return Err(ToolError::PermissionDenied {
                reason: "sensitive artifact content is not model-previewable".to_string(),
            });
        }
        if !metadata
            .mime_type
            .as_deref()
            .is_some_and(|mime| mime.starts_with("text/"))
        {
            return Err(ToolError::ArtifactUnavailable {
                reason: "artifact is not validated text content".to_string(),
            });
        }
        let payload = store.get(&artifact_id).await.map_err(|_| ToolError::ArtifactUnavailable {
            reason: "artifact payload is expired or missing; metadata and provenance remain available".to_string(),
        })?;
        let text = std::str::from_utf8(&payload).map_err(|_| ToolError::ArtifactUnavailable {
            reason: "artifact payload is not UTF-8 text".to_string(),
        })?;
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_ARTIFACT_RESOLVE_BYTES as u64) as usize;
        if offset > payload.len() || !text.is_char_boundary(offset) {
            return Err(ToolError::InvalidInput {
                reason: "artifact offset is outside the payload or not a UTF-8 boundary"
                    .to_string(),
            });
        }
        let mut end = offset.saturating_add(limit).min(payload.len());
        while end > offset && !text.is_char_boundary(end) {
            end -= 1;
        }
        let truncated = end < payload.len();
        Ok(ToolOutput::text(
            serde_json::to_string(&ArtifactResolution {
                artifact_id: raw_id.to_string(),
                status: "available",
                content: text[offset..end].to_string(),
                offset,
                end,
                total_bytes: payload.len(),
                sha256: metadata.sha256,
                mime_type: metadata.mime_type,
                truncated,
                continuation: truncated.then(|| format!("offset:{end}")),
            })
            .map_err(|error| ToolError::ExecutionFailed {
                reason: error.to_string(),
            })?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::environment::local_environment;
    use crate::memory::paths::MemoryPaths;
    use crate::state::tool_artifacts::{ArtifactClaim, ToolArtifactStore};
    use crate::tools::runtime_context::runtime_tool_context_with_artifacts;
    use crate::types::{ApprovalPolicy, CallId};
    use crate::workspace::Workspace;
    use rove_core::{ArtifactTrust, ToolArtifactKind, ToolArtifactSource};
    use tokio_util::sync::CancellationToken;

    fn context<'a>(workspace: &'a Workspace, store: Arc<ToolArtifactStore>) -> ToolContext<'a> {
        runtime_tool_context_with_artifacts(
            CallId::new(),
            workspace,
            MemoryPaths::from_workspace(workspace, 8),
            ApprovalPolicy::Auto,
            None,
            CancellationToken::new(),
            local_environment(workspace),
            Some(store),
        )
    }

    #[tokio::test]
    async fn resolver_reports_available_expired_sensitive_and_hostile_refs_explicitly() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        let store = Arc::new(ToolArtifactStore::new(temp.path().join("runs/run_test")));
        let source = ToolArtifactSource {
            run_id: "run_test".to_string(),
            call_id: "call".to_string(),
            captured_at: "2026-08-12T00:00:00Z".to_string(),
            ..ToolArtifactSource::default()
        };
        let available = store
            .put(
                ToolArtifactKind::Text,
                b"retained text",
                source.clone(),
                ArtifactClaim {
                    mime_type: Some("text/plain".to_string()),
                    ..ArtifactClaim::default()
                },
                Sensitivity::Normal,
                ArtifactTrust::LocalTool,
            )
            .await
            .unwrap();
        let tool = ResolveToolArtifactTool;
        let output = tool
            .execute(
                serde_json::json!({"artifact_id":available.artifact_id.to_string()}),
                &context(&workspace, store.clone()),
            )
            .await
            .unwrap();
        assert!(output.content.contains("retained text"));

        store.expire_payload(&available.artifact_id).await.unwrap();
        assert!(matches!(
            tool.execute(
                serde_json::json!({"artifact_id":available.artifact_id.to_string()}),
                &context(&workspace, store.clone()),
            ).await,
            Err(ToolError::ArtifactUnavailable { reason }) if reason.contains("expired or missing")
        ));
        assert!(matches!(
            tool.execute(
                serde_json::json!({"artifact_id":"../../outside"}),
                &context(&workspace, store.clone()),
            ).await,
            Err(ToolError::ArtifactUnavailable { reason }) if reason.contains("malformed")
        ));
        assert!(matches!(
            tool.execute(
                serde_json::json!({"artifact_id":"art_00000000000000000000000000000000"}),
                &context(&workspace, store.clone()),
            ).await,
            Err(ToolError::ArtifactUnavailable { reason }) if reason.contains("missing")
        ));

        let utf8 = store
            .put(
                ToolArtifactKind::Text,
                "\u{e9}vidence".as_bytes(),
                ToolArtifactSource {
                    call_id: "utf8".to_string(),
                    ..source.clone()
                },
                ArtifactClaim {
                    mime_type: Some("text/plain".to_string()),
                    ..ArtifactClaim::default()
                },
                Sensitivity::Normal,
                ArtifactTrust::LocalTool,
            )
            .await
            .unwrap();
        assert!(matches!(
            tool.execute(
                serde_json::json!({
                    "artifact_id":utf8.artifact_id.to_string(),
                    "offset":1
                }),
                &context(&workspace, store.clone()),
            ).await,
            Err(ToolError::InvalidInput { reason }) if reason.contains("UTF-8 boundary")
        ));

        let non_text = store
            .put(
                ToolArtifactKind::Image,
                b"not-previewed",
                ToolArtifactSource {
                    call_id: "image".to_string(),
                    ..source.clone()
                },
                ArtifactClaim {
                    mime_type: Some("image/png".to_string()),
                    ..ArtifactClaim::default()
                },
                Sensitivity::Normal,
                ArtifactTrust::LocalTool,
            )
            .await
            .unwrap();
        assert!(matches!(
            tool.execute(
                serde_json::json!({"artifact_id":non_text.artifact_id.to_string()}),
                &context(&workspace, store.clone()),
            ).await,
            Err(ToolError::ArtifactUnavailable { reason }) if reason.contains("not validated text")
        ));

        let sensitive = store
            .put(
                ToolArtifactKind::Text,
                b"private",
                ToolArtifactSource {
                    call_id: "sensitive".to_string(),
                    ..source
                },
                ArtifactClaim {
                    mime_type: Some("text/plain".to_string()),
                    ..ArtifactClaim::default()
                },
                Sensitivity::Sensitive,
                ArtifactTrust::LocalTool,
            )
            .await
            .unwrap();
        assert!(matches!(
            tool.execute(
                serde_json::json!({"artifact_id":sensitive.artifact_id.to_string()}),
                &context(&workspace, store),
            )
            .await,
            Err(ToolError::PermissionDenied { .. })
        ));
    }
}
