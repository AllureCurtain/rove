//! Typed `ProcedureMetadata` (design §11.1).
//!
//! Frontmatter arrives as untrusted text. This module converts it into typed
//! fields with stable codes, or fails with a bounded diagnostic. Two fields
//! are conspicuously absent:
//!
//! * `trust` — derived from source location, never self-declared (§11.1).
//! * anything provider- or credential-shaped — a procedure describes a method,
//!   not an identity.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::frontmatter::SplitDocument;

/// Procedure frontmatter schema version understood by this build.
pub const PROCEDURE_SCHEMA_VERSION: u16 = 1;
/// Longest accepted procedure ID.
pub const MAX_PROCEDURE_ID_LEN: usize = 128;
/// Longest accepted title/summary.
pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_SUMMARY_CHARS: usize = 500;

/// Lifecycle status of a procedure document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureStatus {
    Draft,
    Active,
    Deprecated,
    Retired,
}

impl ProcedureStatus {
    pub fn code(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Retired => "retired",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "active" => Some(Self::Active),
            "deprecated" => Some(Self::Deprecated),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }

    /// Whether this status may be selected without an explicit opt-in.
    ///
    /// Only `active` may. A draft is unreviewed, and a retired document was
    /// withdrawn — selecting either by default would defeat the review step.
    pub fn is_selectable_by_default(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// What the procedure is for. Keeps diagnosis separable from remediation
/// (design §11.4), so a diagnostic run never hydrates deletion steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcedureMode {
    Diagnose,
    Remediate,
    Verify,
    General,
}

impl ProcedureMode {
    pub fn code(self) -> &'static str {
        match self {
            Self::Diagnose => "diagnose",
            Self::Remediate => "remediate",
            Self::Verify => "verify",
            Self::General => "general",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "diagnose" => Some(Self::Diagnose),
            "remediate" => Some(Self::Remediate),
            "verify" => Some(Self::Verify),
            "general" => Some(Self::General),
            _ => None,
        }
    }

    /// Whether this mode is expected to change external state.
    pub fn mutates_external_state(self) -> bool {
        matches!(self, Self::Remediate)
    }
}

/// How risky following the procedure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

/// The typed metadata block of a procedure document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureMetadata {
    pub schema_version: u16,
    pub id: String,
    pub version: String,
    pub status: ProcedureStatus,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub mode: ProcedureMode,
    /// Agent IDs this procedure applies to. Empty means any agent.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub agents: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub intents: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub tags: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Workspace kinds this applies to. Empty means any kind.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub workspace_kinds: BTreeSet<String>,
    /// Platforms this applies to. Empty means any platform.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub platforms: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_capabilities: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub optional_capabilities: BTreeSet<String>,
    pub risk_level: RiskLevel,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub side_effects: BTreeSet<SideEffect>,
    /// Inputs the procedure expects to be supplied rather than hardcoded.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub declared_parameters: BTreeSet<String>,
    #[serde(default)]
    pub owner: String,
    /// Last human review date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
    /// Hard expiry date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub references: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub conflicts_with: BTreeSet<String>,
}

/// Every key the schema recognizes. Used to detect unknown keys, which are
/// reported rather than ignored so a typo'd `platforms` cannot silently make a
/// Linux-only procedure apply everywhere.
pub const KNOWN_KEYS: &[&str] = &[
    "schema_version",
    "kind",
    "id",
    "version",
    "status",
    "title",
    "summary",
    "mode",
    "agents",
    "intents",
    "tags",
    "scope",
    "workspace_kinds",
    "platforms",
    "required_capabilities",
    "optional_capabilities",
    "risk_level",
    "side_effects",
    "declared_parameters",
    "owner",
    "reviewed_at",
    "valid_until",
    "references",
    "supersedes",
    "conflicts_with",
];

/// A field-level metadata problem, with a stable code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{field}: {message}")]
pub struct MetadataIssue {
    pub code: String,
    pub field: String,
    pub message: String,
}

impl MetadataIssue {
    fn new(code: &str, field: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            field: field.to_string(),
            message: message
                .into()
                .chars()
                .filter(|character| !character.is_control())
                .take(200)
                .collect(),
        }
    }
}

/// Read a required scalar.
fn required_scalar(
    document: &SplitDocument,
    key: &str,
    max_chars: usize,
) -> Result<String, MetadataIssue> {
    let value = document
        .frontmatter
        .get(key)
        .ok_or_else(|| MetadataIssue::new("missing_field", key, "required field is absent"))?;
    let scalar = value.as_scalar().ok_or_else(|| {
        MetadataIssue::new(
            "expected_scalar",
            key,
            "expected a scalar, found a sequence",
        )
    })?;
    let trimmed = scalar.trim();
    if trimmed.is_empty() {
        return Err(MetadataIssue::new("empty_field", key, "value is empty"));
    }
    if trimmed.chars().count() > max_chars {
        return Err(MetadataIssue::new(
            "field_too_long",
            key,
            format!("value exceeds {max_chars} characters"),
        ));
    }
    Ok(trimmed.to_string())
}

/// Read an optional scalar. Present-but-empty is an error rather than `None`,
/// because an empty `valid_until:` most likely means the author intended a date.
fn optional_scalar(
    document: &SplitDocument,
    key: &str,
    max_chars: usize,
) -> Result<Option<String>, MetadataIssue> {
    match document.frontmatter.get(key) {
        None => Ok(None),
        Some(value) => required_scalar(document, key, max_chars)
            .map(Some)
            .map_err(|issue| {
                if value.as_sequence().is_some() {
                    MetadataIssue::new(
                        "expected_scalar",
                        key,
                        "expected a scalar, found a sequence",
                    )
                } else {
                    issue
                }
            }),
    }
}

/// Read an optional sequence of bounded, lowercase-safe tokens.
fn optional_token_set(
    document: &SplitDocument,
    key: &str,
) -> Result<BTreeSet<String>, MetadataIssue> {
    let Some(value) = document.frontmatter.get(key) else {
        return Ok(BTreeSet::new());
    };
    let items = value.as_sequence().ok_or_else(|| {
        MetadataIssue::new(
            "expected_sequence",
            key,
            "expected a sequence such as [a, b], found a scalar",
        )
    })?;
    let mut set = BTreeSet::new();
    for item in items {
        let token = item.trim();
        if token.is_empty() {
            return Err(MetadataIssue::new(
                "empty_item",
                key,
                "sequence has an empty item",
            ));
        }
        if token.chars().count() > MAX_PROCEDURE_ID_LEN {
            return Err(MetadataIssue::new(
                "item_too_long",
                key,
                format!("item exceeds {MAX_PROCEDURE_ID_LEN} characters"),
            ));
        }
        if !set.insert(token.to_string()) {
            return Err(MetadataIssue::new(
                "duplicate_item",
                key,
                format!("item '{token}' appears more than once"),
            ));
        }
    }
    Ok(set)
}

/// Read an optional sequence of free-form references, which may be URLs and so
/// are allowed to be longer than a token but are still bounded.
fn optional_reference_set(
    document: &SplitDocument,
    key: &str,
) -> Result<BTreeSet<String>, MetadataIssue> {
    let Some(value) = document.frontmatter.get(key) else {
        return Ok(BTreeSet::new());
    };
    let items = value.as_sequence().ok_or_else(|| {
        MetadataIssue::new(
            "expected_sequence",
            key,
            "expected a sequence such as [a, b], found a scalar",
        )
    })?;
    let mut set = BTreeSet::new();
    for item in items {
        let reference = item.trim();
        if reference.is_empty() {
            return Err(MetadataIssue::new(
                "empty_item",
                key,
                "sequence has an empty item",
            ));
        }
        if reference.chars().count() > MAX_SUMMARY_CHARS {
            return Err(MetadataIssue::new(
                "item_too_long",
                key,
                format!("item exceeds {MAX_SUMMARY_CHARS} characters"),
            ));
        }
        set.insert(reference.to_string());
    }
    Ok(set)
}

/// Parse an enum-valued scalar, listing the accepted codes on failure so an
/// author does not have to guess.
fn parse_enum<T>(
    document: &SplitDocument,
    key: &str,
    parse: impl Fn(&str) -> Option<T>,
    accepted: &[&str],
) -> Result<T, MetadataIssue> {
    let raw = required_scalar(document, key, 64)?;
    parse(&raw).ok_or_else(|| {
        MetadataIssue::new(
            "unknown_value",
            key,
            format!("'{raw}' is not one of: {}", accepted.join(", ")),
        )
    })
}

/// Validate a `YYYY-MM-DD` date without pulling in a date library, since the
/// only requirement here is that the field be comparable and well-formed.
fn validate_iso_date(field: &str, value: &str) -> Result<(), MetadataIssue> {
    let invalid = || {
        MetadataIssue::new(
            "invalid_date",
            field,
            format!("'{value}' is not a YYYY-MM-DD date"),
        )
    };
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(invalid());
    }
    if !parts
        .iter()
        .all(|part| part.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let month: u32 = parts[1].parse().map_err(|_| invalid())?;
    let day: u32 = parts[2].parse().map_err(|_| invalid())?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(invalid());
    }
    Ok(())
}

/// Validate a procedure ID. Same rules as an agent ID plus dots, because
/// procedure IDs are conventionally dotted (`build.cargo-clippy-failure`).
fn validate_procedure_id(field: &str, value: &str) -> Result<(), MetadataIssue> {
    let invalid = |reason: &str| {
        MetadataIssue::new(
            "invalid_id",
            field,
            format!("'{}': {reason}", value.chars().take(64).collect::<String>()),
        )
    };
    if value.len() > MAX_PROCEDURE_ID_LEN {
        return Err(invalid("exceeds the length limit"));
    }
    if value.contains("..") {
        return Err(invalid("must not contain '..'"));
    }
    if value.starts_with('.') || value.ends_with('.') {
        return Err(invalid("must not start or end with '.'"));
    }
    let allowed = value
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if !allowed {
        return Err(invalid(
            "must use only lowercase letters, digits, '-', '_', and '.'",
        ));
    }
    Ok(())
}

/// Record an issue and yield `None`, so a single pass can collect every problem
/// instead of returning at the first one.
fn push<T>(result: Result<T, MetadataIssue>, issues: &mut Vec<MetadataIssue>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(issue) => {
            issues.push(issue);
            None
        }
    }
}

/// Parse typed metadata from a split document.
///
/// Collects every issue rather than stopping at the first, so an author fixing
/// a document sees the whole list in one pass. Unknown keys are reported
/// (design §21.1): a misspelled `platforms` would otherwise silently widen a
/// platform-specific procedure to every platform.
pub fn parse_metadata(document: &SplitDocument) -> Result<ProcedureMetadata, Vec<MetadataIssue>> {
    let mut issues = Vec::new();

    for key in document.frontmatter.keys() {
        if !KNOWN_KEYS.contains(&key.as_str()) {
            issues.push(MetadataIssue::new(
                "unknown_key",
                key,
                "key is not part of the procedure schema",
            ));
        }
    }

    let schema_version = match required_scalar(document, "schema_version", 8) {
        Ok(raw) => match raw.parse::<u16>() {
            Ok(version) if version == PROCEDURE_SCHEMA_VERSION => Some(version),
            Ok(version) => {
                issues.push(MetadataIssue::new(
                    "unsupported_schema_version",
                    "schema_version",
                    format!("version {version} is not supported by this build (expected {PROCEDURE_SCHEMA_VERSION})"),
                ));
                None
            }
            Err(_) => {
                issues.push(MetadataIssue::new(
                    "invalid_schema_version",
                    "schema_version",
                    format!("'{raw}' is not an integer"),
                ));
                None
            }
        },
        Err(issue) => {
            issues.push(issue);
            None
        }
    };

    let id = push(
        required_scalar(document, "id", MAX_PROCEDURE_ID_LEN),
        &mut issues,
    );
    if let Some(id) = id.as_deref()
        && let Err(issue) = validate_procedure_id("id", id)
    {
        issues.push(issue);
    }
    let version = push(required_scalar(document, "version", 32), &mut issues);
    let title = push(
        required_scalar(document, "title", MAX_TITLE_CHARS),
        &mut issues,
    );
    let status = push(
        parse_enum(
            document,
            "status",
            ProcedureStatus::parse,
            &["draft", "active", "deprecated", "retired"],
        ),
        &mut issues,
    );
    let mode = push(
        parse_enum(
            document,
            "mode",
            ProcedureMode::parse,
            &["diagnose", "remediate", "verify", "general"],
        ),
        &mut issues,
    );
    let risk_level = push(
        parse_enum(
            document,
            "risk_level",
            RiskLevel::parse,
            &["low", "medium", "high"],
        ),
        &mut issues,
    );

    let summary = push(
        optional_scalar(document, "summary", MAX_SUMMARY_CHARS),
        &mut issues,
    )
    .flatten()
    .unwrap_or_default();
    let scope = push(
        optional_scalar(document, "scope", MAX_TITLE_CHARS),
        &mut issues,
    )
    .flatten();
    let owner = push(
        optional_scalar(document, "owner", MAX_TITLE_CHARS),
        &mut issues,
    )
    .flatten()
    .unwrap_or_default();
    let supersedes = push(
        optional_scalar(document, "supersedes", MAX_PROCEDURE_ID_LEN),
        &mut issues,
    )
    .flatten();
    if let Some(supersedes) = supersedes.as_deref()
        && let Err(issue) = validate_procedure_id("supersedes", supersedes)
    {
        issues.push(issue);
    }

    let reviewed_at = push(optional_scalar(document, "reviewed_at", 16), &mut issues).flatten();
    if let Some(date) = reviewed_at.as_deref()
        && let Err(issue) = validate_iso_date("reviewed_at", date)
    {
        issues.push(issue);
    }
    let valid_until = push(optional_scalar(document, "valid_until", 16), &mut issues).flatten();
    if let Some(date) = valid_until.as_deref()
        && let Err(issue) = validate_iso_date("valid_until", date)
    {
        issues.push(issue);
    }

    let agents = push(optional_token_set(document, "agents"), &mut issues).unwrap_or_default();
    let intents = push(optional_token_set(document, "intents"), &mut issues).unwrap_or_default();
    let tags = push(optional_token_set(document, "tags"), &mut issues).unwrap_or_default();
    let workspace_kinds =
        push(optional_token_set(document, "workspace_kinds"), &mut issues).unwrap_or_default();
    let platforms =
        push(optional_token_set(document, "platforms"), &mut issues).unwrap_or_default();
    let required_capabilities = push(
        optional_token_set(document, "required_capabilities"),
        &mut issues,
    )
    .unwrap_or_default();
    let optional_capabilities = push(
        optional_token_set(document, "optional_capabilities"),
        &mut issues,
    )
    .unwrap_or_default();
    let declared_parameters = push(
        optional_token_set(document, "declared_parameters"),
        &mut issues,
    )
    .unwrap_or_default();
    let conflicts_with =
        push(optional_token_set(document, "conflicts_with"), &mut issues).unwrap_or_default();
    let references =
        push(optional_reference_set(document, "references"), &mut issues).unwrap_or_default();

    let side_effects = match document.frontmatter.get("side_effects") {
        None => BTreeSet::new(),
        Some(value) => match value.as_sequence() {
            None => {
                issues.push(MetadataIssue::new(
                    "expected_sequence",
                    "side_effects",
                    "expected a sequence such as [read_only], found a scalar",
                ));
                BTreeSet::new()
            }
            Some(items) => {
                let mut effects = BTreeSet::new();
                for item in items {
                    match SideEffect::parse(item.trim()) {
                        Some(effect) => {
                            effects.insert(effect);
                        }
                        None => issues.push(MetadataIssue::new(
                            "unknown_value",
                            "side_effects",
                            format!(
                                "'{}' is not a known side effect",
                                item.chars().take(64).collect::<String>()
                            ),
                        )),
                    }
                }
                effects
            }
        },
    };

    // A remediation that claims to be read-only is a contradiction: the
    // declaration is what gating decisions are made from, so it must be
    // coherent before the document is usable.
    if let Some(mode) = mode {
        if mode.mutates_external_state() && side_effects == BTreeSet::from([SideEffect::ReadOnly]) {
            issues.push(MetadataIssue::new(
                "contradictory_side_effects",
                "side_effects",
                "mode 'remediate' cannot declare only 'read_only'",
            ));
        }
        if !mode.mutates_external_state()
            && side_effects.iter().any(|effect| effect.is_destructive())
        {
            issues.push(MetadataIssue::new(
                "contradictory_side_effects",
                "side_effects",
                format!(
                    "mode '{}' cannot declare destructive side effects",
                    mode.code()
                ),
            ));
        }
    }

    // A capability cannot be simultaneously required and optional; leaving the
    // conflict in place would make the eligibility result depend on evaluation
    // order rather than on the document.
    for capability in required_capabilities.intersection(&optional_capabilities) {
        issues.push(MetadataIssue::new(
            "capability_required_and_optional",
            "optional_capabilities",
            format!("'{capability}' is listed as both required and optional"),
        ));
    }

    if let Some(id) = id.as_deref() {
        if conflicts_with.contains(id) {
            issues.push(MetadataIssue::new(
                "self_conflict",
                "conflicts_with",
                "a procedure cannot conflict with itself",
            ));
        }
        if supersedes.as_deref() == Some(id) {
            issues.push(MetadataIssue::new(
                "self_supersede",
                "supersedes",
                "a procedure cannot supersede itself",
            ));
        }
    }

    if !issues.is_empty() {
        return Err(issues);
    }

    Ok(ProcedureMetadata {
        schema_version: schema_version.expect("schema_version validated above"),
        id: id.expect("id validated above"),
        version: version.expect("version validated above"),
        status: status.expect("status validated above"),
        title: title.expect("title validated above"),
        summary,
        mode: mode.expect("mode validated above"),
        agents,
        intents,
        tags,
        scope,
        workspace_kinds,
        platforms,
        required_capabilities,
        optional_capabilities,
        risk_level: risk_level.expect("risk_level validated above"),
        side_effects,
        declared_parameters,
        owner,
        reviewed_at,
        valid_until,
        references,
        supersedes,
        conflicts_with,
    })
}

#[cfg(test)]
mod tests {
    use super::super::frontmatter::split_document;
    use super::*;

    /// A minimal valid document, used as the base for negative cases so each
    /// test changes exactly one thing.
    fn valid_document() -> String {
        [
            "---",
            "schema_version: 1",
            "id: build.clippy-failure",
            "version: 1.0.0",
            "status: active",
            "title: Resolve a clippy failure",
            "mode: diagnose",
            "risk_level: low",
            "---",
            "",
            "# Steps",
            "",
            "1. Read the failing lint.",
        ]
        .join("\n")
    }

    fn parse(text: &str) -> Result<ProcedureMetadata, Vec<MetadataIssue>> {
        let document = split_document(text).expect("document splits");
        parse_metadata(&document)
    }

    fn codes(issues: &[MetadataIssue]) -> Vec<&str> {
        issues.iter().map(|issue| issue.code.as_str()).collect()
    }

    #[test]
    fn a_minimal_document_parses_with_empty_optional_fields() {
        let metadata = parse(&valid_document()).expect("minimal document parses");
        assert_eq!(metadata.id, "build.clippy-failure");
        assert_eq!(metadata.status, ProcedureStatus::Active);
        assert_eq!(metadata.mode, ProcedureMode::Diagnose);
        assert_eq!(metadata.risk_level, RiskLevel::Low);
        assert!(metadata.tags.is_empty());
        assert!(metadata.side_effects.is_empty());
        assert_eq!(metadata.reviewed_at, None);
    }

    #[test]
    fn a_fully_populated_document_parses_every_field() {
        let text = [
            "---",
            "schema_version: 1",
            "id: deploy.rollback",
            "version: 2.1.0",
            "status: active",
            "title: Roll back a failed deploy",
            "summary: Restore the previous release.",
            "mode: remediate",
            "agents: [coder, operator]",
            "intents: [rollback, incident]",
            "tags: [deploy, urgent]",
            "scope: services/api",
            "workspace_kinds: [rust, node]",
            "platforms: [linux, windows]",
            "required_capabilities: [shell]",
            "optional_capabilities: [http]",
            "risk_level: high",
            "side_effects: [restarts_service, writes_external_state]",
            "declared_parameters: [release_tag]",
            "owner: platform-team",
            "reviewed_at: 2026-07-01",
            "valid_until: 2027-01-01",
            "references: [https://runbooks.example/rollback]",
            "supersedes: deploy.rollback-legacy",
            "conflicts_with: [deploy.forward-fix]",
            "---",
            "",
            "Body.",
        ]
        .join("\n");
        let metadata = parse(&text).expect("full document parses");
        assert_eq!(metadata.version, "2.1.0");
        assert_eq!(metadata.mode, ProcedureMode::Remediate);
        assert_eq!(metadata.agents.len(), 2);
        assert_eq!(metadata.platforms.len(), 2);
        assert_eq!(metadata.side_effects.len(), 2);
        assert_eq!(metadata.scope.as_deref(), Some("services/api"));
        assert_eq!(metadata.owner, "platform-team");
        assert_eq!(metadata.valid_until.as_deref(), Some("2027-01-01"));
        assert_eq!(
            metadata.supersedes.as_deref(),
            Some("deploy.rollback-legacy")
        );
    }

    /// A typo'd key is the difference between a Linux-only procedure and one
    /// that applies everywhere, so it must not be silently discarded.
    #[test]
    fn an_unknown_key_is_reported_rather_than_ignored() {
        let text = valid_document().replace("status: active", "status: active\nplatform: linux");
        let issues = parse(&text).expect_err("unknown key fails");
        assert!(codes(&issues).contains(&"unknown_key"), "{issues:?}");
        assert!(issues.iter().any(|issue| issue.field == "platform"));
    }

    #[test]
    fn every_required_field_is_reported_when_absent() {
        let text = "---\nowner: nobody\n---\n\nBody.\n";
        let issues = parse(text).expect_err("empty metadata fails");
        let missing: Vec<&str> = issues
            .iter()
            .filter(|issue| issue.code == "missing_field")
            .map(|issue| issue.field.as_str())
            .collect();
        for field in [
            "schema_version",
            "id",
            "version",
            "title",
            "status",
            "mode",
            "risk_level",
        ] {
            assert!(
                missing.contains(&field),
                "{field} must be reported: {issues:?}"
            );
        }
    }

    /// Collecting all issues means one authoring pass fixes the document.
    #[test]
    fn multiple_independent_problems_are_all_reported_at_once() {
        let text = valid_document()
            .replace("status: active", "status: enabled")
            .replace("risk_level: low", "risk_level: catastrophic");
        let issues = parse(&text).expect_err("two bad enums fail");
        assert_eq!(
            issues.iter().filter(|i| i.code == "unknown_value").count(),
            2
        );
    }

    #[test]
    fn an_unsupported_schema_version_is_rejected_with_the_expected_version() {
        let text = valid_document().replace("schema_version: 1", "schema_version: 99");
        let issues = parse(&text).expect_err("future schema fails");
        assert_eq!(codes(&issues), vec!["unsupported_schema_version"]);
        assert!(issues[0].message.contains("expected 1"));
    }

    #[test]
    fn a_traversal_or_uppercase_id_is_rejected() {
        for bad in [
            "../escape",
            "Build.Clippy",
            "build/clippy",
            ".leading",
            "trailing.",
        ] {
            let text = valid_document().replace("id: build.clippy-failure", &format!("id: {bad}"));
            let Err(issues) = parse(&text) else {
                panic!("id '{bad}' must be rejected");
            };
            assert!(codes(&issues).contains(&"invalid_id"), "{bad}: {issues:?}");
        }
    }

    /// `platforms: linux` and `platforms: [linux]` are different authoring
    /// mistakes; conflating them would hide one.
    #[test]
    fn a_scalar_where_a_sequence_is_expected_is_rejected() {
        let text = valid_document().replace("status: active", "status: active\nplatforms: linux");
        let issues = parse(&text).expect_err("scalar sequence fails");
        assert!(codes(&issues).contains(&"expected_sequence"), "{issues:?}");
    }

    #[test]
    fn a_sequence_where_a_scalar_is_expected_is_rejected() {
        let text = valid_document().replace("title: Resolve a clippy failure", "title: [a, b]");
        let issues = parse(&text).expect_err("sequence title fails");
        assert!(codes(&issues).contains(&"expected_scalar"), "{issues:?}");
    }

    #[test]
    fn a_malformed_date_is_rejected() {
        for bad in [
            "2026-7-1",
            "07-01-2026",
            "2026-13-01",
            "2026-01-32",
            "yesterday",
        ] {
            let text = valid_document().replace(
                "status: active",
                &format!("status: active\nvalid_until: {bad}"),
            );
            let issues = parse(&text).expect_err("bad date fails");
            assert!(
                codes(&issues).contains(&"invalid_date"),
                "{bad}: {issues:?}"
            );
        }
    }

    /// The declared side effects are what gating decisions read, so a
    /// remediation must not be able to present itself as read-only.
    #[test]
    fn a_remediation_cannot_declare_itself_read_only() {
        let text = valid_document()
            .replace("mode: diagnose", "mode: remediate")
            .replace(
                "risk_level: low",
                "risk_level: low\nside_effects: [read_only]",
            );
        let issues = parse(&text).expect_err("contradiction fails");
        assert!(
            codes(&issues).contains(&"contradictory_side_effects"),
            "{issues:?}"
        );
    }

    #[test]
    fn a_diagnostic_cannot_declare_destructive_side_effects() {
        let text = valid_document().replace(
            "risk_level: low",
            "risk_level: low\nside_effects: [deletes_data]",
        );
        let issues = parse(&text).expect_err("contradiction fails");
        assert!(
            codes(&issues).contains(&"contradictory_side_effects"),
            "{issues:?}"
        );
    }

    #[test]
    fn a_capability_cannot_be_both_required_and_optional() {
        let text = valid_document().replace(
            "risk_level: low",
            "risk_level: low\nrequired_capabilities: [shell]\noptional_capabilities: [shell]",
        );
        let issues = parse(&text).expect_err("ambiguous capability fails");
        assert!(
            codes(&issues).contains(&"capability_required_and_optional"),
            "{issues:?}"
        );
    }

    #[test]
    fn a_procedure_cannot_conflict_with_or_supersede_itself() {
        let text = valid_document().replace(
            "risk_level: low",
            "risk_level: low\nconflicts_with: [build.clippy-failure]\nsupersedes: build.clippy-failure",
        );
        let issues = parse(&text).expect_err("self reference fails");
        assert!(codes(&issues).contains(&"self_conflict"), "{issues:?}");
        assert!(codes(&issues).contains(&"self_supersede"), "{issues:?}");
    }

    #[test]
    fn a_duplicate_sequence_item_is_rejected() {
        let text =
            valid_document().replace("risk_level: low", "risk_level: low\ntags: [deploy, deploy]");
        let issues = parse(&text).expect_err("duplicate item fails");
        assert!(codes(&issues).contains(&"duplicate_item"), "{issues:?}");
    }

    #[test]
    fn an_empty_required_field_is_rejected_rather_than_defaulted() {
        let text = valid_document().replace("title: Resolve a clippy failure", "title: \"\"");
        let issues = parse(&text).expect_err("empty title fails");
        assert!(codes(&issues).contains(&"empty_field"), "{issues:?}");
    }

    #[test]
    fn an_over_long_title_is_rejected() {
        let long = "x".repeat(MAX_TITLE_CHARS + 1);
        let text =
            valid_document().replace("title: Resolve a clippy failure", &format!("title: {long}"));
        let issues = parse(&text).expect_err("long title fails");
        assert!(codes(&issues).contains(&"field_too_long"), "{issues:?}");
    }

    /// Trust is derived from where a document lives, so `trust` is not part of
    /// the schema and a document declaring it is rejected outright.
    #[test]
    fn a_document_cannot_declare_its_own_trust_level() {
        let text =
            valid_document().replace("status: active", "status: active\ntrust: builtin_trusted");
        let issues = parse(&text).expect_err("self-declared trust fails");
        assert!(codes(&issues).contains(&"unknown_key"), "{issues:?}");
        assert!(issues.iter().any(|issue| issue.field == "trust"));
    }

    #[test]
    fn only_active_is_selectable_by_default() {
        assert!(ProcedureStatus::Active.is_selectable_by_default());
        for status in [
            ProcedureStatus::Draft,
            ProcedureStatus::Deprecated,
            ProcedureStatus::Retired,
        ] {
            assert!(
                !status.is_selectable_by_default(),
                "{} must not be selectable by default",
                status.code()
            );
        }
    }

    #[test]
    fn enum_codes_round_trip() {
        for status in [
            ProcedureStatus::Draft,
            ProcedureStatus::Active,
            ProcedureStatus::Deprecated,
            ProcedureStatus::Retired,
        ] {
            assert_eq!(ProcedureStatus::parse(status.code()), Some(status));
        }
        for mode in [
            ProcedureMode::Diagnose,
            ProcedureMode::Remediate,
            ProcedureMode::Verify,
            ProcedureMode::General,
        ] {
            assert_eq!(ProcedureMode::parse(mode.code()), Some(mode));
        }
        for risk in [RiskLevel::Low, RiskLevel::Medium, RiskLevel::High] {
            assert_eq!(RiskLevel::parse(risk.code()), Some(risk));
        }
        for effect in [
            SideEffect::ReadOnly,
            SideEffect::WritesWorkspace,
            SideEffect::WritesExternalState,
            SideEffect::DeletesData,
            SideEffect::RestartsService,
            SideEffect::SendsNotification,
        ] {
            assert_eq!(SideEffect::parse(effect.code()), Some(effect));
        }
    }

    #[test]
    fn metadata_serialization_round_trips() {
        let metadata = parse(&valid_document()).expect("parses");
        let json = serde_json::to_string(&metadata).expect("serializes");
        let restored: ProcedureMetadata = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(metadata, restored);
    }

    /// An issue message built from untrusted text must stay bounded and free of
    /// control characters so it cannot corrupt a log line.
    #[test]
    fn issue_messages_from_untrusted_text_are_bounded_and_control_free() {
        let issue = MetadataIssue::new("code", "field", format!("a\nb\r\n{}", "x".repeat(500)));
        assert!(!issue.message.contains('\n'));
        assert!(issue.message.chars().count() <= 200);
    }
}
