//! Data-leaf types sunk from `rove-runtime`'s `agents` behaviour modules.
//!
//! These are serialisable identity/diagnostic records only — the behaviour
//! that produces or consumes them (activation, selection, hydration,
//! validation) stays in `rove-runtime::agents`.

use serde::{Deserialize, Serialize};

use crate::selector::AgentSelector;

/// How much authority a procedure source carries.
///
/// Ordered most to least trusted, so `Ord` can be used for policy comparisons.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
)]
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

/// How risky following the procedure is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

impl RiskLevel {
    pub fn code(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// Declared side effects.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    ReadOnly,
    WritesWorkspace,
    WritesExternalState,
    DeletesData,
    RestartsService,
    SendsNotification,
}

impl SideEffect {
    pub fn code(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WritesWorkspace => "writes_workspace",
            Self::WritesExternalState => "writes_external_state",
            Self::DeletesData => "deletes_data",
            Self::RestartsService => "restarts_service",
            Self::SendsNotification => "sends_notification",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read_only" => Some(Self::ReadOnly),
            "writes_workspace" => Some(Self::WritesWorkspace),
            "writes_external_state" => Some(Self::WritesExternalState),
            "deletes_data" => Some(Self::DeletesData),
            "restarts_service" => Some(Self::RestartsService),
            "sends_notification" => Some(Self::SendsNotification),
            _ => None,
        }
    }

    /// Whether this effect is observable outside the current workspace.
    pub fn is_destructive(self) -> bool {
        matches!(
            self,
            Self::DeletesData | Self::RestartsService | Self::WritesExternalState
        )
    }
}

/// An audit-grade pointer to a procedure, safe to persist and to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProcedureReference {
    pub id: String,
    pub version: String,
    pub trust: ProcedureTrust,
    pub source_path: String,
    pub content_hash: String,
}

/// Stable, content-free identity safe for events, reports, and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentProfileIdentity {
    pub selector: AgentSelector,
    pub agent_id: String,
    pub display_name: String,
    pub definition_version: String,
    pub manifest_hash: String,
    pub package_hash: String,
    pub profile_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction_bundle_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub procedures: Vec<ProcedureReference>,
}

/// One bounded diagnostic produced while resolving an Agent context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AgentDiagnostic {
    pub code: String,
    pub subject: String,
    pub message: String,
}
