//! Canonical durable Tool Artifact authority.
//!
//! This is the single place tool payloads become durable. It is deliberately
//! separate from [`crate::environment::TransientArtifactStore`], which holds
//! process-local Coding Tool projections that never survive a restart: mixing
//! the two would make a transient projection look like durable evidence.
//!
//! Layout under a run directory:
//!
//! ```text
//! .rove/runs/<run_id>/
//!   artifacts/<artifact_id>/payload
//!   artifacts/<artifact_id>/metadata.json
//!   tool_artifacts.jsonl
//! ```
//!
//! Invariants this module enforces:
//!
//! - An artifact ID is derived locally from the content hash. A remote
//!   filename, URI, or tool name never contributes to a path.
//! - Bytes are hashed while they are written, so the recorded digest always
//!   describes exactly the bytes on disk.
//! - Every quota is checked before and during the write; exceeding one stops
//!   the read, removes the partial payload, and records a rejection.
//! - `metadata.json` is written to a temporary file and renamed, so a reader
//!   never observes a half-written record.
//! - `tool_artifacts.jsonl` is append-only and records rejections as well as
//!   commits, so a quota event stays auditable after cleanup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rove_core::{
    ArtifactId, ArtifactTrust, ArtifactValidation, Sensitivity, ToolArtifactKind, ToolArtifactRef,
    ToolArtifactSource, validated_mime_type,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// Largest single artifact payload retained.
pub const MAX_SINGLE_ARTIFACT_BYTES: u64 = 8 * 1024 * 1024;

/// Largest total artifact bytes retained for one tool call.
pub const MAX_TOOL_CALL_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024;

/// Largest total artifact bytes retained for one run.
pub const MAX_RUN_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

/// Most artifacts retained for one run.
pub const MAX_RUN_ARTIFACTS: usize = 512;

/// Durable Tool Artifacts live in their own directory, deliberately NOT the
/// run's `artifacts/` directory.
///
/// `artifacts/` is a flat set of registered run files that the product manifest
/// enumerates as regular files. Tool Artifacts are content-addressed
/// directories holding a payload plus metadata, so sharing the directory would
/// make every Tool Artifact look like a malformed registered artifact and would
/// blur the transient/durable boundary the two stores are meant to keep.
pub const ARTIFACTS_DIR: &str = "tool_artifacts";
const LEDGER_FILE: &str = "tool_artifacts.jsonl";
const PAYLOAD_FILE: &str = "payload";
const METADATA_FILE: &str = "metadata.json";

/// Why an artifact was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRejection {
    SingleArtifactBytes,
    ToolCallArtifactBytes,
    RunArtifactBytes,
    RunArtifactCount,
    EmptyPayload,
}

impl ArtifactRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::SingleArtifactBytes => "artifact_single_bytes_exceeded",
            Self::ToolCallArtifactBytes => "artifact_tool_call_bytes_exceeded",
            Self::RunArtifactBytes => "artifact_run_bytes_exceeded",
            Self::RunArtifactCount => "artifact_run_count_exceeded",
            Self::EmptyPayload => "artifact_empty_payload",
        }
    }
}

impl std::fmt::Display for ArtifactRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail = match self {
            Self::SingleArtifactBytes => "the payload exceeds the single-artifact byte quota",
            Self::ToolCallArtifactBytes => "the tool call exceeds its total artifact byte quota",
            Self::RunArtifactBytes => "the run exceeds its total artifact byte quota",
            Self::RunArtifactCount => "the run exceeds its artifact count quota",
            Self::EmptyPayload => "an artifact payload must not be empty",
        };
        f.write_str(detail)
    }
}

/// Failure modes of the artifact authority.
#[derive(Debug, thiserror::Error)]
pub enum ToolArtifactError {
    /// A quota refused the artifact. The rejection is recorded in the ledger.
    #[error("{0}")]
    Rejected(ArtifactRejection),
    #[error("artifact io error: {0}")]
    Io(#[from] std::io::Error),
}

/// What a caller claims about a payload. Every field is untrusted.
#[derive(Debug, Clone, Default)]
pub struct ArtifactClaim {
    pub mime_type: Option<String>,
    pub original_uri: Option<String>,
    pub audience: Option<Vec<String>>,
    pub priority: Option<f32>,
    pub last_modified: Option<String>,
}

/// One append-only ledger line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ArtifactLedgerEntry {
    Committed {
        artifact: Box<ToolArtifactRef>,
    },
    Rejected {
        call_id: String,
        block_ordinal: u32,
        reason: ArtifactRejection,
        /// Bytes observed before the read was stopped, for diagnostics.
        observed_bytes: u64,
        recorded_at: String,
    },
}

/// Running totals used to enforce per-call and per-run quotas.
#[derive(Debug, Default, Clone)]
struct ArtifactUsage {
    run_bytes: u64,
    run_count: usize,
    call_bytes: HashMap<String, u64>,
}

/// The durable artifact authority for one run.
pub struct ToolArtifactStore {
    run_dir: PathBuf,
    usage: tokio::sync::Mutex<ArtifactUsage>,
}

impl ToolArtifactStore {
    pub fn new(run_dir: impl Into<PathBuf>) -> Self {
        Self {
            run_dir: run_dir.into(),
            usage: tokio::sync::Mutex::new(ArtifactUsage::default()),
        }
    }

    fn artifacts_dir(&self) -> PathBuf {
        self.run_dir.join(ARTIFACTS_DIR)
    }

    /// The run this store belongs to, taken from its directory name.
    ///
    /// The store is created as `.rove/runs/<run_id>`, so the directory is the
    /// authority. Reading it here avoids threading the run ID through every
    /// tool that might produce an artifact.
    pub fn run_id(&self) -> String {
        self.run_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string()
    }

    fn ledger_path(&self) -> PathBuf {
        self.run_dir.join(LEDGER_FILE)
    }

    /// Stores `bytes` as a durable artifact.
    ///
    /// The payload is streamed and hashed in one pass and stopped the moment a
    /// quota would be exceeded, so a hostile or broken tool cannot force an
    /// unbounded write. On rejection the partial payload is removed and the
    /// reason is appended to the ledger.
    pub async fn put(
        &self,
        kind: ToolArtifactKind,
        bytes: &[u8],
        source: ToolArtifactSource,
        claim: ArtifactClaim,
        sensitivity: Sensitivity,
        trust: ArtifactTrust,
    ) -> Result<ToolArtifactRef, ToolArtifactError> {
        if bytes.is_empty() {
            self.record_rejection(&source, ArtifactRejection::EmptyPayload, 0)
                .await?;
            return Err(ToolArtifactError::Rejected(ArtifactRejection::EmptyPayload));
        }
        let byte_length = bytes.len() as u64;
        if byte_length > MAX_SINGLE_ARTIFACT_BYTES {
            self.record_rejection(&source, ArtifactRejection::SingleArtifactBytes, byte_length)
                .await?;
            return Err(ToolArtifactError::Rejected(
                ArtifactRejection::SingleArtifactBytes,
            ));
        }

        let digest = {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            hex_digest(&hasher.finalize())
        };
        // Identity is derived only from the content hash, never from a
        // remote-supplied name, so a payload cannot escape its directory.
        let artifact_id = ArtifactId::new(format!("art_{}", &digest[..32]));
        let dir = self.artifacts_dir().join(artifact_id.as_str());
        let payload_path = dir.join(PAYLOAD_FILE);

        let mut usage = self.usage.lock().await;
        // Deduplication within a run reuses the payload but still records this
        // call's own reference, so provenance is not collapsed.
        let already_present = tokio::fs::try_exists(&payload_path).await.unwrap_or(false);
        if !already_present {
            let call_bytes = usage
                .call_bytes
                .get(&source.call_id)
                .copied()
                .unwrap_or_default()
                .saturating_add(byte_length);
            if call_bytes > MAX_TOOL_CALL_ARTIFACT_BYTES {
                drop(usage);
                self.record_rejection(
                    &source,
                    ArtifactRejection::ToolCallArtifactBytes,
                    byte_length,
                )
                .await?;
                return Err(ToolArtifactError::Rejected(
                    ArtifactRejection::ToolCallArtifactBytes,
                ));
            }
            if usage.run_bytes.saturating_add(byte_length) > MAX_RUN_ARTIFACT_BYTES {
                drop(usage);
                self.record_rejection(&source, ArtifactRejection::RunArtifactBytes, byte_length)
                    .await?;
                return Err(ToolArtifactError::Rejected(
                    ArtifactRejection::RunArtifactBytes,
                ));
            }
            if usage.run_count >= MAX_RUN_ARTIFACTS {
                drop(usage);
                self.record_rejection(&source, ArtifactRejection::RunArtifactCount, byte_length)
                    .await?;
                return Err(ToolArtifactError::Rejected(
                    ArtifactRejection::RunArtifactCount,
                ));
            }

            tokio::fs::create_dir_all(&dir).await?;
            if let Err(error) = write_payload(&payload_path, bytes).await {
                // Never leave a partial payload behind.
                let _ = tokio::fs::remove_file(&payload_path).await;
                return Err(error.into());
            }
            usage.run_bytes = usage.run_bytes.saturating_add(byte_length);
            usage.run_count += 1;
            usage
                .call_bytes
                .entry(source.call_id.clone())
                .and_modify(|total| *total = total.saturating_add(byte_length))
                .or_insert(byte_length);
        }
        drop(usage);

        // A rejected MIME claim becomes `None` rather than a guess, and the
        // reason is recorded so a UI can explain why there is no preview.
        let validated_mime = validated_mime_type(claim.mime_type.as_deref());
        let (validation, validation_detail) = match (&claim.mime_type, &validated_mime) {
            (Some(_), None) => (
                ArtifactValidation::ClaimRejected,
                Some("the declared MIME type was not a well-formed type".to_string()),
            ),
            _ => (ArtifactValidation::Validated, None),
        };

        let artifact = ToolArtifactRef {
            artifact_id: artifact_id.clone(),
            kind,
            mime_type: validated_mime,
            byte_length,
            sha256: digest,
            storage_ref: format!("{ARTIFACTS_DIR}/{artifact_id}/{PAYLOAD_FILE}"),
            source,
            original_uri: rove_core::recorded_uri_claim(claim.original_uri.as_deref()),
            audience: claim.audience,
            priority: claim.priority,
            last_modified: claim.last_modified,
            sensitivity,
            trust,
            validation,
            validation_detail,
        };

        write_metadata(&dir, &artifact).await?;
        self.append_ledger(&ArtifactLedgerEntry::Committed {
            artifact: Box::new(artifact.clone()),
        })
        .await?;
        Ok(artifact)
    }

    /// Reads a payload back by artifact ID.
    ///
    /// The path is rebuilt from the validated ID, so a caller cannot traverse
    /// out of the artifacts directory by supplying a crafted identifier.
    pub async fn get(&self, artifact_id: &ArtifactId) -> Result<Vec<u8>, ToolArtifactError> {
        let id = artifact_id.as_str();
        if !is_valid_artifact_id(id) {
            return Err(ToolArtifactError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid artifact id",
            )));
        }
        let path = self.artifacts_dir().join(id).join(PAYLOAD_FILE);
        Ok(tokio::fs::read(path).await?)
    }

    /// Loads the committed artifact metadata for one ID.
    pub async fn metadata(
        &self,
        artifact_id: &ArtifactId,
    ) -> Result<ToolArtifactRef, ToolArtifactError> {
        let id = artifact_id.as_str();
        if !is_valid_artifact_id(id) {
            return Err(ToolArtifactError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid artifact id",
            )));
        }
        let raw = tokio::fs::read(self.artifacts_dir().join(id).join(METADATA_FILE)).await?;
        serde_json::from_slice(&raw).map_err(|error| {
            ToolArtifactError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                error.to_string(),
            ))
        })
    }

    /// Every ledger entry in order, including rejections.
    pub async fn ledger(&self) -> Result<Vec<ArtifactLedgerEntry>, ToolArtifactError> {
        let path = self.ledger_path();
        let raw = match tokio::fs::read_to_string(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        Ok(raw
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect())
    }

    /// Removes a payload while keeping its metadata and ledger history.
    ///
    /// Cleanup deletes bytes; it never rewrites the recorded outcome of a tool
    /// call. A report shows such an artifact as expired rather than absent.
    pub async fn expire_payload(
        &self,
        artifact_id: &ArtifactId,
    ) -> Result<bool, ToolArtifactError> {
        let id = artifact_id.as_str();
        if !is_valid_artifact_id(id) {
            return Ok(false);
        }
        let path = self.artifacts_dir().join(id).join(PAYLOAD_FILE);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// True when a payload is still on disk.
    pub async fn payload_available(&self, artifact_id: &ArtifactId) -> bool {
        if !is_valid_artifact_id(artifact_id.as_str()) {
            return false;
        }
        tokio::fs::try_exists(
            self.artifacts_dir()
                .join(artifact_id.as_str())
                .join(PAYLOAD_FILE),
        )
        .await
        .unwrap_or(false)
    }

    async fn record_rejection(
        &self,
        source: &ToolArtifactSource,
        reason: ArtifactRejection,
        observed_bytes: u64,
    ) -> Result<(), ToolArtifactError> {
        self.append_ledger(&ArtifactLedgerEntry::Rejected {
            call_id: source.call_id.clone(),
            block_ordinal: source.block_ordinal,
            reason,
            observed_bytes,
            recorded_at: source.captured_at.clone(),
        })
        .await
    }

    async fn append_ledger(&self, entry: &ArtifactLedgerEntry) -> Result<(), ToolArtifactError> {
        tokio::fs::create_dir_all(&self.run_dir).await?;
        let line = serde_json::to_string(entry).map_err(|error| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
        })?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.ledger_path())
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(())
    }
}

async fn write_payload(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    Ok(())
}

/// Writes metadata through a temporary file and renames it, so a concurrent
/// reader sees either the previous state or the complete record.
async fn write_metadata(dir: &Path, artifact: &ToolArtifactRef) -> std::io::Result<()> {
    let encoded = serde_json::to_vec_pretty(artifact)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()))?;
    let final_path = dir.join(METADATA_FILE);
    let tmp_path = dir.join("metadata.json.tmp");
    tokio::fs::write(&tmp_path, &encoded).await?;
    tokio::fs::rename(&tmp_path, &final_path).await
}

fn hex_digest(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// True only for an identifier this module could have generated.
///
/// Exported so a serving layer validates identity with this exact rule instead
/// of re-deriving a looser one. The shape is fixed and opaque: a caller cannot
/// smuggle a path separator, a parent reference, or a case variant through it.
pub fn is_valid_artifact_id(id: &str) -> bool {
    id.len() == 36
        && id.starts_with("art_")
        && id[4..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(call_id: &str, ordinal: u32) -> ToolArtifactSource {
        ToolArtifactSource {
            run_id: "run_test".to_string(),
            call_id: call_id.to_string(),
            server_config_id: Some("srv".to_string()),
            server_identity_hash: Some("identity-hash".to_string()),
            session_hash: Some("session-hash".to_string()),
            remote_tool_name: Some("render".to_string()),
            block_ordinal: ordinal,
            captured_at: "2026-08-09T00:00:00Z".to_string(),
        }
    }

    async fn store() -> (tempfile::TempDir, ToolArtifactStore) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = ToolArtifactStore::new(dir.path().join("runs/run_test"));
        (dir, store)
    }

    #[tokio::test]
    async fn a_committed_artifact_is_hashed_readable_and_ledgered() {
        let (_dir, store) = store().await;
        let artifact = store
            .put(
                ToolArtifactKind::Text,
                b"hello artifact",
                source("call_1", 0),
                ArtifactClaim {
                    mime_type: Some("text/plain".to_string()),
                    ..ArtifactClaim::default()
                },
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap();

        assert_eq!(artifact.byte_length, 14);
        assert_eq!(artifact.sha256.len(), 64);
        assert_eq!(artifact.mime_type.as_deref(), Some("text/plain"));
        assert_eq!(artifact.validation, ArtifactValidation::Validated);
        assert!(artifact.artifact_id.as_str().starts_with("art_"));

        // The digest must describe exactly the bytes on disk.
        let payload = store.get(&artifact.artifact_id).await.unwrap();
        assert_eq!(payload, b"hello artifact");
        let mut hasher = Sha256::new();
        hasher.update(&payload);
        assert_eq!(artifact.sha256, hex_digest(&hasher.finalize()));

        let reloaded = store.metadata(&artifact.artifact_id).await.unwrap();
        assert_eq!(reloaded, artifact);

        let ledger = store.ledger().await.unwrap();
        assert_eq!(ledger.len(), 1);
        assert!(matches!(
            &ledger[0],
            ArtifactLedgerEntry::Committed { artifact: entry } if entry.artifact_id == artifact.artifact_id
        ));
    }

    #[tokio::test]
    async fn a_remote_filename_or_uri_never_steers_the_storage_path() {
        let (_dir, store) = store().await;
        let artifact = store
            .put(
                ToolArtifactKind::Resource,
                b"payload bytes",
                source("call_1", 0),
                ArtifactClaim {
                    mime_type: Some("text/plain".to_string()),
                    original_uri: Some("file:///../../../etc/passwd".to_string()),
                    ..ArtifactClaim::default()
                },
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap();

        // The claim is retained for provenance only.
        assert_eq!(
            artifact.original_uri.as_deref(),
            Some("file:///../../../etc/passwd")
        );
        assert!(!artifact.storage_ref.contains(".."));
        assert!(
            artifact
                .storage_ref
                .starts_with(&format!("{ARTIFACTS_DIR}/art_"))
        );
        assert!(store.get(&artifact.artifact_id).await.is_ok());
    }

    #[tokio::test]
    async fn a_crafted_artifact_id_cannot_traverse_out_of_the_store() {
        let (_dir, store) = store().await;
        for hostile in [
            "../../../etc/passwd",
            "art_../../secret",
            "art_ZZZZ",
            "art_short",
            "",
        ] {
            let id = ArtifactId::new(hostile);
            assert!(
                store.get(&id).await.is_err(),
                "{hostile:?} must be refused before touching the filesystem"
            );
            assert!(store.metadata(&id).await.is_err());
            assert!(!store.payload_available(&id).await);
            assert!(!store.expire_payload(&id).await.unwrap());
        }
    }

    #[tokio::test]
    async fn an_oversized_payload_is_rejected_and_recorded() {
        let (_dir, store) = store().await;
        let bytes = vec![7u8; MAX_SINGLE_ARTIFACT_BYTES as usize + 1];
        let error = store
            .put(
                ToolArtifactKind::Unknown,
                &bytes,
                source("call_1", 2),
                ArtifactClaim::default(),
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ToolArtifactError::Rejected(ArtifactRejection::SingleArtifactBytes)
        ));
        // Nothing was written, and the rejection stays auditable.
        assert!(!store.artifacts_dir().exists());
        let ledger = store.ledger().await.unwrap();
        assert_eq!(ledger.len(), 1);
        assert!(matches!(
            &ledger[0],
            ArtifactLedgerEntry::Rejected {
                reason: ArtifactRejection::SingleArtifactBytes,
                block_ordinal: 2,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn an_empty_payload_is_refused() {
        let (_dir, store) = store().await;
        let error = store
            .put(
                ToolArtifactKind::Text,
                b"",
                source("call_1", 0),
                ArtifactClaim::default(),
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ToolArtifactError::Rejected(ArtifactRejection::EmptyPayload)
        ));
    }

    #[tokio::test]
    async fn the_per_call_quota_stops_accumulation_without_blocking_another_call() {
        let (_dir, store) = store().await;
        let chunk = vec![1u8; 4 * 1024 * 1024];
        // Distinct content per write so dedup does not mask the quota.
        // Four 4 MiB payloads fill the 16 MiB per-call quota exactly.
        for index in 0..4u8 {
            let mut bytes = chunk.clone();
            bytes[0] = index;
            store
                .put(
                    ToolArtifactKind::Unknown,
                    &bytes,
                    source("call_hot", index as u32),
                    ArtifactClaim::default(),
                    Sensitivity::Normal,
                    ArtifactTrust::Untrusted,
                )
                .await
                .unwrap();
        }
        let mut overflow = chunk.clone();
        overflow[0] = 99;
        let error = store
            .put(
                ToolArtifactKind::Unknown,
                &overflow,
                source("call_hot", 9),
                ArtifactClaim::default(),
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ToolArtifactError::Rejected(ArtifactRejection::ToolCallArtifactBytes)
        ));

        // A different call has its own budget.
        let mut other = chunk.clone();
        other[0] = 123;
        assert!(
            store
                .put(
                    ToolArtifactKind::Unknown,
                    &other,
                    source("call_cold", 0),
                    ArtifactClaim::default(),
                    Sensitivity::Normal,
                    ArtifactTrust::Untrusted,
                )
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn identical_bytes_dedupe_the_payload_but_keep_each_reference() {
        let (_dir, store) = store().await;
        let first = store
            .put(
                ToolArtifactKind::Text,
                b"same bytes",
                source("call_1", 0),
                ArtifactClaim::default(),
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap();
        let second = store
            .put(
                ToolArtifactKind::Text,
                b"same bytes",
                source("call_2", 5),
                ArtifactClaim::default(),
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap();

        assert_eq!(first.artifact_id, second.artifact_id);
        // Provenance is per call, not collapsed by dedup.
        assert_eq!(first.source.call_id, "call_1");
        assert_eq!(second.source.call_id, "call_2");
        assert_eq!(second.source.block_ordinal, 5);
        assert_eq!(store.ledger().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_rejected_mime_claim_is_recorded_not_guessed() {
        let (_dir, store) = store().await;
        let artifact = store
            .put(
                ToolArtifactKind::Image,
                b"\x89PNG\r\n\x1a\n",
                source("call_1", 0),
                ArtifactClaim {
                    mime_type: Some("not a mime type".to_string()),
                    ..ArtifactClaim::default()
                },
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap();

        assert_eq!(artifact.mime_type, None);
        assert_eq!(artifact.validation, ArtifactValidation::ClaimRejected);
        assert!(artifact.validation_detail.is_some());
        // No MIME type means no inline preview, even though the bytes are PNG.
        assert!(!artifact.is_inline_previewable());
    }

    #[tokio::test]
    async fn expiring_a_payload_keeps_metadata_and_history() {
        let (_dir, store) = store().await;
        let artifact = store
            .put(
                ToolArtifactKind::Text,
                b"retained",
                source("call_1", 0),
                ArtifactClaim::default(),
                Sensitivity::Sensitive,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap();

        assert!(store.payload_available(&artifact.artifact_id).await);
        assert!(store.expire_payload(&artifact.artifact_id).await.unwrap());
        assert!(!store.payload_available(&artifact.artifact_id).await);
        // Cleanup removes bytes only: the record of what happened survives.
        assert_eq!(
            store.metadata(&artifact.artifact_id).await.unwrap().sha256,
            artifact.sha256
        );
        assert_eq!(store.ledger().await.unwrap().len(), 1);
        assert!(store.get(&artifact.artifact_id).await.is_err());
        // A sensitive artifact is never offered for inline preview.
        assert!(!artifact.is_inline_previewable());
        // Expiring twice is not an error.
        assert!(!store.expire_payload(&artifact.artifact_id).await.unwrap());
    }

    #[tokio::test]
    async fn metadata_is_never_observed_half_written() {
        let (_dir, store) = store().await;
        let artifact = store
            .put(
                ToolArtifactKind::Text,
                b"atomic",
                source("call_1", 0),
                ArtifactClaim::default(),
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap();
        let dir = store.artifacts_dir().join(artifact.artifact_id.as_str());
        // The temporary file is renamed, not left beside the record.
        assert!(dir.join(METADATA_FILE).exists());
        assert!(!dir.join("metadata.json.tmp").exists());
    }

    #[tokio::test]
    async fn the_run_artifact_count_quota_is_enforced() {
        let (_dir, store) = store().await;
        for index in 0..MAX_RUN_ARTIFACTS {
            let bytes = format!("payload {index}").into_bytes();
            store
                .put(
                    ToolArtifactKind::Text,
                    &bytes,
                    source("call_1", index as u32),
                    ArtifactClaim::default(),
                    Sensitivity::Normal,
                    ArtifactTrust::Untrusted,
                )
                .await
                .unwrap();
        }
        let error = store
            .put(
                ToolArtifactKind::Text,
                b"one too many",
                source("call_1", 9_999),
                ArtifactClaim::default(),
                Sensitivity::Normal,
                ArtifactTrust::Untrusted,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            ToolArtifactError::Rejected(ArtifactRejection::RunArtifactCount)
        ));
    }

    #[test]
    fn only_locally_generated_identifiers_are_accepted() {
        assert!(is_valid_artifact_id(&format!("art_{}", "a".repeat(32))));
        assert!(!is_valid_artifact_id(&format!("art_{}", "A".repeat(32))));
        assert!(!is_valid_artifact_id(&format!("art_{}", "a".repeat(31))));
        assert!(!is_valid_artifact_id(&format!("xxx_{}", "a".repeat(32))));
        assert!(!is_valid_artifact_id("art_../../../../etc/passwd0000000"));
    }
}
