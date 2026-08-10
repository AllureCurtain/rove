//! Procedure trust levels, derived from source — never self-reported.
//!
//! Design §12.1: a document author cannot write `trust: builtin` in their
//! frontmatter and be believed. Trust comes from where the file was found and
//! whether an operator installed or approved it. This module holds no parsing
//! of document content at all, which is the structural reason a document
//! cannot influence its own trust.

use serde::{Deserialize, Serialize};

/// How much authority a procedure source carries.
///
/// Ordered most to least trusted, so `Ord` can be used for policy comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureTrust {
    /// Shipped inside the runtime or a builtin Agent package.
    BuiltinTrusted,
    /// Tracked in the workspace under a configured procedure root.
    WorkspaceTrusted,
    /// Installed by an explicit operator action outside the workspace.
    UserInstalled,
    /// Everything else: uploaded, downloaded, model-authored, or retrieved.
    /// Never selectable as a procedure (design §25.5 case 8).
    ExternalUntrusted,
}

impl ProcedureTrust {
    pub fn code(self) -> &'static str {
        match self {
            Self::BuiltinTrusted => "builtin_trusted",
            Self::WorkspaceTrusted => "workspace_trusted",
            Self::UserInstalled => "user_installed",
            Self::ExternalUntrusted => "external_untrusted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "builtin_trusted" => Some(Self::BuiltinTrusted),
            "workspace_trusted" => Some(Self::WorkspaceTrusted),
            "user_installed" => Some(Self::UserInstalled),
            "external_untrusted" => Some(Self::ExternalUntrusted),
            _ => None,
        }
    }

    /// Whether a procedure at this trust level may ever be *selected*.
    ///
    /// External content can still be read by an explicit workspace tool — it
    /// simply never acquires instruction authority by being retrieved.
    pub fn is_selectable(self) -> bool {
        !matches!(self, Self::ExternalUntrusted)
    }

    /// Trust levels permitted by default when a package does not narrow them.
    pub fn default_allowed() -> [Self; 2] {
        [Self::BuiltinTrusted, Self::WorkspaceTrusted]
    }
}

/// Where a procedure document was found. Trust is a function of this alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureOrigin {
    /// Inside a builtin Agent package compiled into the runtime.
    BuiltinPackage,
    /// Inside the selected workspace Agent package's `procedures/` directory.
    WorkspacePackage,
    /// Under an operator-configured workspace procedure root.
    WorkspaceRoot,
    /// Installed by explicit operator action outside the workspace.
    OperatorInstalled,
    /// Supplied at runtime: uploaded, fetched, retrieved, or model-authored.
    RuntimeSupplied,
}

impl ProcedureOrigin {
    pub fn code(self) -> &'static str {
        match self {
            Self::BuiltinPackage => "builtin_package",
            Self::WorkspacePackage => "workspace_package",
            Self::WorkspaceRoot => "workspace_root",
            Self::OperatorInstalled => "operator_installed",
            Self::RuntimeSupplied => "runtime_supplied",
        }
    }

    /// Derive trust from origin. This is the only way a trust level is ever
    /// produced.
    pub fn trust(self) -> ProcedureTrust {
        match self {
            Self::BuiltinPackage => ProcedureTrust::BuiltinTrusted,
            Self::WorkspacePackage | Self::WorkspaceRoot => ProcedureTrust::WorkspaceTrusted,
            Self::OperatorInstalled => ProcedureTrust::UserInstalled,
            Self::RuntimeSupplied => ProcedureTrust::ExternalUntrusted,
        }
    }
}

/// Where a procedure came from, for audit and for resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureProvenance {
    pub origin: ProcedureOrigin,
    /// Trust derived from `origin`. Stored so a persisted snapshot carries the
    /// decision, but always recomputed rather than trusted on load.
    pub trust: ProcedureTrust,
    /// Workspace-relative source path, or a stable label for builtin content.
    pub source_path: String,
    /// Canonical content hash of the whole document.
    pub content_hash: String,
}

impl ProcedureProvenance {
    pub fn new(
        origin: ProcedureOrigin,
        source_path: impl Into<String>,
        content_hash: impl Into<String>,
    ) -> Self {
        Self {
            origin,
            trust: origin.trust(),
            source_path: source_path.into(),
            content_hash: content_hash.into(),
        }
    }

    /// Recompute trust from origin, discarding whatever a loaded snapshot said.
    ///
    /// A persisted artifact is untrusted input like any other: if someone edits
    /// `trust` in a `task_state.json`, reload must not honour it.
    pub fn with_recomputed_trust(mut self) -> Self {
        self.trust = self.origin.trust();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_is_derived_from_origin() {
        assert_eq!(
            ProcedureOrigin::BuiltinPackage.trust(),
            ProcedureTrust::BuiltinTrusted
        );
        assert_eq!(
            ProcedureOrigin::WorkspacePackage.trust(),
            ProcedureTrust::WorkspaceTrusted
        );
        assert_eq!(
            ProcedureOrigin::WorkspaceRoot.trust(),
            ProcedureTrust::WorkspaceTrusted
        );
        assert_eq!(
            ProcedureOrigin::OperatorInstalled.trust(),
            ProcedureTrust::UserInstalled
        );
        assert_eq!(
            ProcedureOrigin::RuntimeSupplied.trust(),
            ProcedureTrust::ExternalUntrusted
        );
    }

    /// Retrieved or uploaded content must never be selectable, no matter how
    /// well it scores.
    #[test]
    fn runtime_supplied_content_is_never_selectable() {
        assert!(!ProcedureOrigin::RuntimeSupplied.trust().is_selectable());
        for origin in [
            ProcedureOrigin::BuiltinPackage,
            ProcedureOrigin::WorkspacePackage,
            ProcedureOrigin::WorkspaceRoot,
            ProcedureOrigin::OperatorInstalled,
        ] {
            assert!(origin.trust().is_selectable(), "{}", origin.code());
        }
    }

    #[test]
    fn trust_ordering_runs_from_builtin_down_to_external() {
        assert!(ProcedureTrust::BuiltinTrusted < ProcedureTrust::WorkspaceTrusted);
        assert!(ProcedureTrust::WorkspaceTrusted < ProcedureTrust::UserInstalled);
        assert!(ProcedureTrust::UserInstalled < ProcedureTrust::ExternalUntrusted);
    }

    /// A tampered snapshot must not be able to promote its own trust.
    #[test]
    fn a_tampered_persisted_trust_is_recomputed_from_origin() {
        let mut provenance = ProcedureProvenance::new(
            ProcedureOrigin::RuntimeSupplied,
            "uploads/evil.md",
            "sha256:abc",
        );
        provenance.trust = ProcedureTrust::BuiltinTrusted;

        let restored = provenance.with_recomputed_trust();

        assert_eq!(restored.trust, ProcedureTrust::ExternalUntrusted);
        assert!(!restored.trust.is_selectable());
    }

    #[test]
    fn trust_codes_round_trip() {
        for trust in [
            ProcedureTrust::BuiltinTrusted,
            ProcedureTrust::WorkspaceTrusted,
            ProcedureTrust::UserInstalled,
            ProcedureTrust::ExternalUntrusted,
        ] {
            assert_eq!(ProcedureTrust::parse(trust.code()), Some(trust));
        }
        assert_eq!(ProcedureTrust::parse("builtin"), None);
        assert_eq!(ProcedureTrust::parse(""), None);
    }

    #[test]
    fn defaults_exclude_user_installed_and_external() {
        let allowed = ProcedureTrust::default_allowed();
        assert!(allowed.contains(&ProcedureTrust::BuiltinTrusted));
        assert!(allowed.contains(&ProcedureTrust::WorkspaceTrusted));
        assert!(!allowed.contains(&ProcedureTrust::UserInstalled));
        assert!(!allowed.contains(&ProcedureTrust::ExternalUntrusted));
    }
}
