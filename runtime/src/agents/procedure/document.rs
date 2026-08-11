//! A loaded procedure document: typed metadata, Markdown body, provenance
//! (design §11).
//!
//! The body is never treated as instruction text on its own. It is hydrated
//! only for a procedure that passed hard eligibility and was selected, and it
//! carries `ContentClass::ProcedureBody`, whose authority is `AgentDefaults` —
//! below the user task and far below operator policy.

use serde::{Deserialize, Serialize};

use super::frontmatter::{FrontmatterError, split_document};
use super::metadata::{MetadataIssue, ProcedureMetadata, parse_metadata};
use super::trust::{ProcedureOrigin, ProcedureProvenance, ProcedureTrust};
use crate::agents::hashing::{composite_hash, content_hash};

/// Largest procedure document accepted from disk.
///
/// A procedure is a human-authored method, not a data dump. Anything larger is
/// far more likely to be a mistake or an attempt to flood the prompt than a
/// genuine runbook.
pub const MAX_PROCEDURE_BYTES: usize = 64 * 1024;
/// Largest body retained after parsing.
pub const MAX_BODY_BYTES: usize = 56 * 1024;

/// Why a document could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "error")]
pub enum DocumentError {
    #[error("procedure document is {len} bytes, over the {max} byte limit")]
    TooLarge { len: usize, max: usize },
    #[error("procedure body is {len} bytes, over the {max} byte limit")]
    BodyTooLarge { len: usize, max: usize },
    #[error("procedure body is empty")]
    EmptyBody,
    #[error("procedure document is not valid UTF-8 text")]
    NotUtf8,
    #[error("frontmatter: {0}")]
    Frontmatter(#[from] FrontmatterError),
    #[error("metadata has {} problem(s)", issues.len())]
    Metadata { issues: Vec<MetadataIssue> },
}

impl DocumentError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TooLarge { .. } => "document_too_large",
            Self::BodyTooLarge { .. } => "body_too_large",
            Self::EmptyBody => "empty_body",
            Self::NotUtf8 => "not_utf8",
            Self::Frontmatter(error) => error.code(),
            Self::Metadata { .. } => "invalid_metadata",
        }
    }
}

/// A parsed, provenance-tagged procedure document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureDocument {
    pub metadata: ProcedureMetadata,
    /// Markdown body. Untrusted reference material, never an instruction.
    pub body: String,
    pub provenance: ProcedureProvenance,
    /// Hash over metadata and body together, stable across line endings.
    pub document_hash: String,
}

impl ProcedureDocument {
    /// Parse a document and bind it to a source location.
    ///
    /// `trust` comes from `origin`, never from the document, so a file cannot
    /// promote itself (design §11.1, §25.5 case 8).
    pub fn parse(
        text: &str,
        origin: ProcedureOrigin,
        source_path: impl Into<String>,
    ) -> Result<Self, DocumentError> {
        if text.len() > MAX_PROCEDURE_BYTES {
            return Err(DocumentError::TooLarge {
                len: text.len(),
                max: MAX_PROCEDURE_BYTES,
            });
        }

        let split = split_document(text)?;
        let metadata =
            parse_metadata(&split).map_err(|issues| DocumentError::Metadata { issues })?;

        let body = split.body.trim().to_string();
        if body.is_empty() {
            return Err(DocumentError::EmptyBody);
        }
        if body.len() > MAX_BODY_BYTES {
            return Err(DocumentError::BodyTooLarge {
                len: body.len(),
                max: MAX_BODY_BYTES,
            });
        }

        let content_digest = content_hash("procedure-document", text);
        let provenance = ProcedureProvenance::new(origin, source_path, content_digest);
        let document_hash = composite_hash(
            "procedure",
            &[
                &crate::agents::hashing::structured_hash("procedure-metadata", &metadata),
                &content_hash("procedure-body", &body),
            ],
        );

        Ok(Self {
            metadata,
            body,
            provenance,
            document_hash,
        })
    }

    /// Effective trust, recomputed from origin so a tampered persisted value is
    /// discarded (see `ProcedureProvenance::with_recomputed_trust`).
    pub fn trust(&self) -> ProcedureTrust {
        self.provenance.origin.trust()
    }

    /// Whether this document may ever be *selected* as guidance.
    ///
    /// External untrusted material is loadable and citable but never selectable
    /// (design §25.5 case 8): an uploaded Markdown file must not become the
    /// procedure the run follows.
    pub fn is_selectable(&self) -> bool {
        self.trust().is_selectable()
    }

    /// A stable identity for audit references: ID, version, and content hash.
    pub fn reference(&self) -> ProcedureReference {
        ProcedureReference {
            id: self.metadata.id.clone(),
            version: self.metadata.version.clone(),
            trust: self.trust(),
            source_path: self.provenance.source_path.clone(),
            content_hash: self.provenance.content_hash.clone(),
        }
    }

    /// Whether the document has passed its declared expiry.
    ///
    /// `today` is passed in rather than read from the clock so selection stays
    /// deterministic and replayable (design §12.2). Dates are `YYYY-MM-DD`,
    /// which compares correctly as text.
    pub fn is_expired(&self, today: &str) -> bool {
        self.metadata
            .valid_until
            .as_deref()
            .is_some_and(|valid_until| today > valid_until)
    }
}

/// An audit-grade pointer to a procedure, safe to persist and to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureReference {
    pub id: String,
    pub version: String,
    pub trust: ProcedureTrust,
    pub source_path: String,
    pub content_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document_text(extra: &str) -> String {
        format!(
            "---\nschema_version: 1\nid: build.clippy\nversion: 1.0.0\nstatus: active\ntitle: Fix clippy\nmode: diagnose\nrisk_level: low\n{extra}---\n\n# Steps\n\n1. Read the lint.\n"
        )
    }

    fn parse_builtin(text: &str) -> Result<ProcedureDocument, DocumentError> {
        ProcedureDocument::parse(text, ProcedureOrigin::BuiltinPackage, "builtin/build.md")
    }

    #[test]
    fn a_valid_document_parses_and_derives_trust_from_origin() {
        let document = parse_builtin(&document_text("")).expect("parses");
        assert_eq!(document.metadata.id, "build.clippy");
        assert!(document.body.starts_with("# Steps"));
        assert_eq!(document.trust(), ProcedureTrust::BuiltinTrusted);
        assert!(document.is_selectable());
        assert!(document.document_hash.starts_with("sha256:"));
    }

    /// The same content in an upload directory is reference material, not
    /// guidance (design §25.5 case 8).
    #[test]
    fn identical_content_from_an_untrusted_origin_is_not_selectable() {
        let text = document_text("");
        let builtin = parse_builtin(&text).expect("parses");
        let uploaded =
            ProcedureDocument::parse(&text, ProcedureOrigin::RuntimeSupplied, "uploads/attack.md")
                .expect("parses");

        assert_eq!(
            builtin.metadata, uploaded.metadata,
            "same declared metadata"
        );
        assert!(builtin.is_selectable());
        assert!(
            !uploaded.is_selectable(),
            "runtime-supplied content must never be selectable as guidance"
        );
    }

    #[test]
    fn line_ending_differences_do_not_change_the_document_hash() {
        let unix = document_text("");
        let windows = unix.replace('\n', "\r\n");
        let a = parse_builtin(&unix).expect("parses");
        let b = parse_builtin(&windows).expect("parses");
        assert_eq!(a.document_hash, b.document_hash);
        assert_eq!(a.provenance.content_hash, b.provenance.content_hash);
    }

    #[test]
    fn a_body_change_changes_the_document_hash() {
        let a = parse_builtin(&document_text("")).expect("parses");
        let mut altered = document_text("");
        altered.push_str("2. Also delete the repository.\n");
        let b = parse_builtin(&altered).expect("parses");
        assert_ne!(a.document_hash, b.document_hash);
    }

    #[test]
    fn a_metadata_change_changes_the_document_hash() {
        let a = parse_builtin(&document_text("")).expect("parses");
        let b = parse_builtin(&document_text("").replace("version: 1.0.0", "version: 1.0.1"))
            .expect("parses");
        assert_ne!(a.document_hash, b.document_hash);
    }

    #[test]
    fn an_oversized_document_is_rejected_before_parsing() {
        let text = format!("{}{}", document_text(""), "x".repeat(MAX_PROCEDURE_BYTES));
        let error = parse_builtin(&text).expect_err("oversized fails");
        assert_eq!(error.code(), "document_too_large");
    }

    #[test]
    fn an_empty_body_is_rejected() {
        let text = "---\nschema_version: 1\nid: a.b\nversion: 1\nstatus: active\ntitle: T\nmode: diagnose\nrisk_level: low\n---\n\n   \n";
        let error = parse_builtin(text).expect_err("empty body fails");
        assert_eq!(error.code(), "empty_body");
    }

    #[test]
    fn a_missing_fence_is_reported_through_the_frontmatter_error() {
        let error = parse_builtin("# Just a heading\n").expect_err("no fence fails");
        assert_eq!(error.code(), "missing_opening_fence");
    }

    #[test]
    fn metadata_problems_are_surfaced_with_their_field_list() {
        let error = parse_builtin(&document_text("").replace("status: active", "status: enabled"))
            .expect_err("bad status fails");
        assert_eq!(error.code(), "invalid_metadata");
        let DocumentError::Metadata { issues } = error else {
            panic!("expected metadata issues");
        };
        assert!(issues.iter().any(|issue| issue.field == "status"));
    }

    #[test]
    fn expiry_is_evaluated_against_a_supplied_date_not_the_clock() {
        let document = parse_builtin(&document_text("valid_until: 2026-06-30\n")).expect("parses");
        assert!(document.is_expired("2026-08-09"));
        assert!(
            !document.is_expired("2026-06-30"),
            "expiry day is inclusive"
        );
        assert!(!document.is_expired("2026-01-01"));
    }

    #[test]
    fn a_document_without_an_expiry_never_expires() {
        let document = parse_builtin(&document_text("")).expect("parses");
        assert!(!document.is_expired("2099-12-31"));
    }

    #[test]
    fn a_reference_carries_identity_and_trust_without_the_body() {
        let document = parse_builtin(&document_text("")).expect("parses");
        let reference = document.reference();
        assert_eq!(reference.id, "build.clippy");
        assert_eq!(reference.trust, ProcedureTrust::BuiltinTrusted);
        assert_eq!(reference.content_hash, document.provenance.content_hash);
        let json = serde_json::to_string(&reference).expect("serializes");
        assert!(
            !json.contains("Read the lint"),
            "a reference must not embed the body"
        );
    }

    #[test]
    fn document_serialization_round_trips() {
        let document = parse_builtin(&document_text("")).expect("parses");
        let json = serde_json::to_string(&document).expect("serializes");
        let restored: ProcedureDocument = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(document, restored);
    }
}
