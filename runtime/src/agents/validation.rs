//! Validation that must succeed before an Agent package may activate.
//!
//! Design §9.1 lists the checks. They run *before* any model or tool call, so
//! a malformed or over-reaching package fails at load rather than halfway
//! through a run when it has already touched the workspace.
//!
//! Two rules shape the code here:
//!
//! * **Errors block activation, warnings do not.** A missing referenced file
//!   is an error; a package with no `owner` is a warning. Silently downgrading
//!   an error to a warning would defeat the point.
//! * **Untrusted text never reaches a message verbatim.** Manifest strings are
//!   attacker-influenced, so anything echoed into an issue is bounded and
//!   stripped of control characters.

use std::collections::BTreeSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use super::definition::{AGENT_DEFINITION_SCHEMA_VERSION, AgentDefinition, FeatureRequirement};
use super::selector::{AgentSelector, validate_agent_id};

/// Largest instruction or prompt-slot file that may be injected.
pub const MAX_INSTRUCTION_FILE_BYTES: usize = 64 * 1024;
/// Largest total injectable content across one package.
pub const MAX_PACKAGE_INJECTABLE_BYTES: usize = 256 * 1024;
/// Upper bound on capability ID length.
pub const MAX_CAPABILITY_ID_LEN: usize = 128;
/// Upper bound on how many issues one report retains.
pub const MAX_VALIDATION_ISSUES: usize = 64;

/// Severity of one validation finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    /// Blocks activation.
    Error,
    /// Recorded and surfaced, but activation proceeds.
    Warning,
}

impl IssueSeverity {
    pub fn code(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

/// One validation finding with a stable code and a bounded message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub severity: IssueSeverity,
    /// Stable machine-readable code. Consumers match on this, not the message.
    pub code: String,
    /// Which manifest field or file the issue concerns.
    pub field: String,
    /// Human-readable detail. Bounded and control-character free.
    pub message: String,
}

impl ValidationIssue {
    pub fn error(code: &str, field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Error,
            code: code.to_string(),
            field: field.to_string(),
            message: bound_message(&message.into()),
        }
    }

    pub fn warning(code: &str, field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: IssueSeverity::Warning,
            code: code.to_string(),
            field: field.to_string(),
            message: bound_message(&message.into()),
        }
    }
}

/// The result of validating a package.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ValidationIssue>,
    /// True when issues were dropped because the bound was reached. Recorded
    /// so a truncated report is never mistaken for a clean one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl ValidationReport {
    pub fn push(&mut self, issue: ValidationIssue) {
        if self.issues.len() >= MAX_VALIDATION_ISSUES {
            self.truncated = true;
            return;
        }
        self.issues.push(issue);
    }

    pub fn extend(&mut self, issues: impl IntoIterator<Item = ValidationIssue>) {
        for issue in issues {
            self.push(issue);
        }
    }

    /// Activation is blocked when any error is present, or when the report was
    /// truncated — a truncated report cannot prove the absence of an error.
    pub fn blocks_activation(&self) -> bool {
        self.truncated
            || self
                .issues
                .iter()
                .any(|issue| issue.severity == IssueSeverity::Error)
    }

    /// Number of error-severity issues.
    pub fn error_count(&self) -> usize {
        self.errors().count()
    }

    pub fn errors(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Error)
    }

    pub fn warnings(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.severity == IssueSeverity::Warning)
    }

    pub fn codes(&self) -> Vec<&str> {
        self.issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect()
    }

    pub fn has_code(&self, code: &str) -> bool {
        self.issues.iter().any(|issue| issue.code == code)
    }
}

/// Bound untrusted text and strip control characters before it is reported.
fn bound_message(message: &str) -> String {
    let cleaned = message
        .chars()
        .map(|character| {
            if character.is_control() && character != ' ' {
                ' '
            } else {
                character
            }
        })
        .take(256)
        .collect::<String>();
    cleaned.trim().to_string()
}

/// Bound one untrusted field value for inclusion in a message.
fn quote(value: &str) -> String {
    let cleaned = value
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect::<String>();
    format!("'{cleaned}'")
}

/// Operator-side caps a definition is validated against.
///
/// Passing these in rather than reading global config keeps validation a pure
/// function, which is what lets the intersection rules be tested exhaustively.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorConstraints {
    /// Capabilities the operator permits at all. Empty means "no explicit
    /// allow-list configured", which is not the same as "nothing allowed".
    pub allowed_capabilities: BTreeSet<String>,
    /// Capabilities the operator forbids. Always wins.
    pub denied_capabilities: BTreeSet<String>,
    pub max_steps_cap: Option<u32>,
    pub max_tool_calls_cap: Option<u32>,
    pub max_procedure_selections_cap: Option<u32>,
    /// Memory scopes/types the operator permits.
    pub allowed_memory_scopes: BTreeSet<String>,
    pub allowed_memory_types: BTreeSet<String>,
    /// Runtime schema version this build implements.
    pub runtime_schema_version: u16,
}

impl OperatorConstraints {
    /// Constraints with no operator-side narrowing, used by the legacy path
    /// and by tests that are not exercising intersection behaviour.
    pub fn unconstrained() -> Self {
        Self {
            runtime_schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
            ..Self::default()
        }
    }
}

/// Field names that would carry a credential. A definition must never hold one
/// (design §5), so their presence is an error rather than a warning.
const CREDENTIAL_FIELD_TOKENS: &[&str] = &[
    "api_key",
    "apikey",
    "secret",
    "token",
    "password",
    "passwd",
    "credential",
    "cookie",
    "private_key",
    "access_key",
    "bearer",
    "session_id",
];

/// Keys that would imply an auto-executed hook. Packages must not execute
/// anything (design §8.3).
const EXECUTABLE_HOOK_TOKENS: &[&str] = &[
    "hook",
    "script",
    "command",
    "exec",
    "install",
    "postinstall",
    "preinstall",
    "setup",
    "entrypoint",
    "shell",
];

/// Validate the manifest alone — every §9.1 check that needs no filesystem.
///
/// File existence, encoding, and size are checked by the package loader, which
/// is the layer that actually reads bytes.
pub fn validate_definition(
    definition: &AgentDefinition,
    selector: &AgentSelector,
    constraints: &OperatorConstraints,
) -> ValidationReport {
    let mut report = ValidationReport::default();

    validate_schema_and_identity(definition, selector, &mut report);
    validate_runtime_compatibility(definition, constraints, &mut report);
    validate_referenced_paths(definition, &mut report);
    validate_capability_policy(definition, constraints, &mut report);
    validate_execution_defaults(definition, constraints, &mut report);
    validate_procedure_policy(definition, &mut report);
    validate_memory_policy(definition, constraints, &mut report);
    validate_no_credentials_or_hooks(definition, &mut report);
    validate_hash_stability(definition, &mut report);

    report
}

fn validate_schema_and_identity(
    definition: &AgentDefinition,
    selector: &AgentSelector,
    report: &mut ValidationReport,
) {
    if definition.schema_version != AGENT_DEFINITION_SCHEMA_VERSION {
        report.push(ValidationIssue::error(
            "unsupported_schema_version",
            "schema_version",
            format!(
                "manifest schema_version {} is not the supported version {}",
                definition.schema_version, AGENT_DEFINITION_SCHEMA_VERSION
            ),
        ));
    }

    if let Err(error) = validate_agent_id(&definition.id) {
        report.push(ValidationIssue::error(
            "invalid_agent_id",
            "id",
            error.to_string(),
        ));
    }

    // The manifest ID must match the directory the package was found in.
    // Otherwise `workspace:a` could load a package that believes it is `b`,
    // and every hash, event, and pinned snapshot would name the wrong agent.
    if definition.id != selector.agent_id {
        report.push(ValidationIssue::error(
            "agent_id_mismatch",
            "id",
            format!(
                "manifest id {} does not match selector id {}",
                quote(&definition.id),
                quote(&selector.agent_id)
            ),
        ));
    }

    if definition.definition_version.trim().is_empty() {
        report.push(ValidationIssue::error(
            "missing_definition_version",
            "definition_version",
            "definition_version must not be empty",
        ));
    }

    if definition.display_name.trim().is_empty() {
        report.push(ValidationIssue::error(
            "missing_display_name",
            "display_name",
            "display_name must not be empty",
        ));
    }

    if definition.owner.trim().is_empty() {
        report.push(ValidationIssue::warning(
            "missing_owner",
            "owner",
            "owner is unset, so escalation has no named contact",
        ));
    }
}

fn validate_runtime_compatibility(
    definition: &AgentDefinition,
    constraints: &OperatorConstraints,
    report: &mut ValidationReport,
) {
    let compatibility = &definition.runtime_compatibility;
    let runtime = constraints.runtime_schema_version;

    if compatibility.min_schema_version > runtime {
        report.push(ValidationIssue::error(
            "runtime_too_old",
            "runtime_compatibility.min_schema_version",
            format!(
                "package requires runtime schema >= {}, this runtime is {runtime}",
                compatibility.min_schema_version
            ),
        ));
    }
    if let Some(max) = compatibility.max_schema_version {
        if max < runtime {
            report.push(ValidationIssue::error(
                "runtime_too_new",
                "runtime_compatibility.max_schema_version",
                format!("package supports runtime schema <= {max}, this runtime is {runtime}"),
            ));
        }
        if max < compatibility.min_schema_version {
            report.push(ValidationIssue::error(
                "inverted_schema_range",
                "runtime_compatibility",
                format!(
                    "max_schema_version {max} is below min_schema_version {}",
                    compatibility.min_schema_version
                ),
            ));
        }
    }

    // A required feature is recorded, not silently assumed available. Whether
    // the resolved model actually provides it is checked at profile build,
    // where the model is known.
    if compatibility.min_context_tokens == Some(0) {
        report.push(ValidationIssue::warning(
            "meaningless_context_requirement",
            "runtime_compatibility.min_context_tokens",
            "min_context_tokens = 0 states no requirement; omit the field instead",
        ));
    }
    for modality in &compatibility.required_modalities {
        if modality.trim().is_empty() {
            report.push(ValidationIssue::error(
                "empty_modality",
                "runtime_compatibility.required_modalities",
                "required_modalities must not contain an empty entry",
            ));
        }
    }
}

fn validate_referenced_paths(definition: &AgentDefinition, report: &mut ValidationReport) {
    for (kind, path) in definition.referenced_paths() {
        let field = kind.code().to_string();
        if let Err(issue) = validate_package_relative_path(path, &field) {
            report.push(issue);
        }
    }
    if definition.policy_instructions_path.is_none()
        && definition.default_instructions_path.is_none()
        && definition.prompt_slots.is_empty()
    {
        report.push(ValidationIssue::warning(
            "no_injectable_content",
            "instructions",
            "package declares no policy, instructions, or prompt slots",
        ));
    }
}

/// A referenced path must stay inside the package directory.
///
/// Rejecting absolute paths and `..` here is what keeps a manifest from
/// pointing at `/etc/shadow` or at another agent's policy file. Symlink
/// escapes are caught separately by the loader, which can see the filesystem.
pub fn validate_package_relative_path(path: &Path, field: &str) -> Result<(), ValidationIssue> {
    if path.as_os_str().is_empty() {
        return Err(ValidationIssue::error(
            "empty_path",
            field,
            "referenced path must not be empty",
        ));
    }
    if path.is_absolute() {
        return Err(ValidationIssue::error(
            "absolute_path",
            field,
            format!(
                "referenced path {} must be package-relative",
                quote(&path.to_string_lossy())
            ),
        ));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(ValidationIssue::error(
                    "path_escapes_package",
                    field,
                    format!(
                        "referenced path {} escapes the package directory",
                        quote(&path.to_string_lossy())
                    ),
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ValidationIssue::error(
                    "absolute_path",
                    field,
                    format!(
                        "referenced path {} must be package-relative",
                        quote(&path.to_string_lossy())
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// A capability ID must be a dotted lowercase token.
///
/// Format is enforced so an ID cannot smuggle a glob, a path, or prose into
/// the place where capability matching happens.
pub fn validate_capability_id(capability_id: &str) -> Result<(), String> {
    if capability_id.is_empty() {
        return Err("capability id must not be empty".to_string());
    }
    if capability_id.len() > MAX_CAPABILITY_ID_LEN {
        return Err(format!(
            "capability id is {} bytes, over the {MAX_CAPABILITY_ID_LEN} byte limit",
            capability_id.len()
        ));
    }
    let valid = capability_id.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '-' | '_')
    });
    if !valid {
        return Err(format!(
            "capability id {} must be lowercase dotted alphanumeric",
            quote(capability_id)
        ));
    }
    if capability_id.starts_with('.')
        || capability_id.ends_with('.')
        || capability_id.contains("..")
    {
        return Err(format!(
            "capability id {} has an empty dotted segment",
            quote(capability_id)
        ));
    }
    Ok(())
}

fn validate_capability_policy(
    definition: &AgentDefinition,
    constraints: &OperatorConstraints,
    report: &mut ValidationReport,
) {
    let policy = &definition.capability_policy;
    let groups = [
        ("capability_policy.required", &policy.required),
        ("capability_policy.optional", &policy.optional),
        ("capability_policy.allow", &policy.allow),
        ("capability_policy.deny", &policy.deny),
    ];
    for (field, ids) in groups {
        for capability_id in ids {
            if let Err(message) = validate_capability_id(capability_id) {
                report.push(ValidationIssue::error(
                    "invalid_capability_id",
                    field,
                    message,
                ));
            }
        }
    }

    // A capability both required and denied can never resolve, so the package
    // is asking for something impossible. Failing here beats failing later
    // with a confusing "capability unavailable".
    for capability_id in policy.required.intersection(&policy.deny) {
        report.push(ValidationIssue::error(
            "required_capability_denied",
            "capability_policy",
            format!(
                "capability {} is both required and denied",
                quote(capability_id)
            ),
        ));
    }
    for capability_id in policy.allow.intersection(&policy.deny) {
        report.push(ValidationIssue::warning(
            "allow_denied_capability",
            "capability_policy",
            format!(
                "capability {} appears in both allow and deny; deny wins",
                quote(capability_id)
            ),
        ));
    }

    // Operator deny always wins, so a package requiring a denied capability
    // cannot activate. This is the check that stops a package from acquiring
    // permission by declaring a need for it.
    for capability_id in policy
        .required
        .intersection(&constraints.denied_capabilities)
    {
        report.push(ValidationIssue::error(
            "required_capability_denied_by_operator",
            "capability_policy.required",
            format!(
                "capability {} is required but denied by operator policy",
                quote(capability_id)
            ),
        ));
    }

    // An `allow` entry outside the operator's allow-list is not an expansion;
    // the intersection simply drops it. Reported as a warning so the author
    // learns the entry is inert rather than believing it took effect.
    if !constraints.allowed_capabilities.is_empty() {
        for capability_id in policy.allow.difference(&constraints.allowed_capabilities) {
            report.push(ValidationIssue::warning(
                "allow_outside_operator_policy",
                "capability_policy.allow",
                format!(
                    "capability {} is not in operator allow policy and stays unavailable",
                    quote(capability_id)
                ),
            ));
        }
        for capability_id in policy
            .required
            .difference(&constraints.allowed_capabilities)
        {
            report.push(ValidationIssue::error(
                "required_capability_outside_operator_policy",
                "capability_policy.required",
                format!(
                    "capability {} is required but not permitted by operator policy",
                    quote(capability_id)
                ),
            ));
        }
    }

    if !policy.required.is_disjoint(&policy.optional) {
        report.push(ValidationIssue::warning(
            "capability_required_and_optional",
            "capability_policy",
            "a capability listed as both required and optional is simply required",
        ));
    }
}

fn validate_execution_defaults(
    definition: &AgentDefinition,
    constraints: &OperatorConstraints,
    report: &mut ValidationReport,
) {
    let defaults = &definition.execution_defaults;

    if let Some(strategy) = defaults.strategy.as_deref()
        && !matches!(strategy, "react" | "plan_react")
    {
        report.push(ValidationIssue::error(
            "unknown_execution_strategy",
            "execution_defaults.strategy",
            format!("unknown execution strategy {}", quote(strategy)),
        ));
    }

    // Zero budgets would make the run unable to do anything; that is a
    // manifest mistake, not a valid tightening.
    let budgets = [
        ("execution_defaults.max_steps", defaults.max_steps),
        ("execution_defaults.max_tool_calls", defaults.max_tool_calls),
    ];
    for (field, value) in budgets {
        if value == Some(0) {
            report.push(ValidationIssue::error(
                "zero_budget",
                field,
                "a zero budget cannot execute anything; omit the field instead",
            ));
        }
    }

    // Asking for more than the operator cap is not an error — it is clamped
    // (§8.7) — but the author should know the request had no effect.
    let over_cap = [
        (
            "execution_defaults.max_steps",
            defaults.max_steps,
            constraints.max_steps_cap,
        ),
        (
            "execution_defaults.max_tool_calls",
            defaults.max_tool_calls,
            constraints.max_tool_calls_cap,
        ),
        (
            "execution_defaults.max_procedure_selections",
            defaults.max_procedure_selections,
            constraints.max_procedure_selections_cap,
        ),
    ];
    for (field, requested, cap) in over_cap {
        if let (Some(requested), Some(cap)) = (requested, cap)
            && requested > cap
        {
            report.push(ValidationIssue::warning(
                "budget_exceeds_operator_cap",
                field,
                format!("requested {requested} exceeds operator cap {cap} and is clamped"),
            ));
        }
    }
}

fn validate_procedure_policy(definition: &AgentDefinition, report: &mut ValidationReport) {
    let policy = &definition.procedure_policy;

    for root in &policy.roots {
        if let Err(issue) = validate_package_relative_path(root, "procedure_policy.roots") {
            report.push(issue);
        }
    }

    for level in &policy.allowed_trust_levels {
        if super::procedure::ProcedureTrust::parse(level).is_none() {
            report.push(ValidationIssue::error(
                "unknown_trust_level",
                "procedure_policy.allowed_trust_levels",
                format!("unknown procedure trust level {}", quote(level)),
            ));
        }
    }

    // `external_untrusted` must never be selectable: the whole point of the
    // trust taxonomy is that an uploaded document cannot become a selected
    // procedure (design §25.5 case 8).
    if policy
        .allowed_trust_levels
        .contains(super::procedure::ProcedureTrust::ExternalUntrusted.code())
    {
        report.push(ValidationIssue::error(
            "external_untrusted_not_selectable",
            "procedure_policy.allowed_trust_levels",
            "external_untrusted procedures may never be selected",
        ));
    }

    if policy.roots.is_empty() && policy.max_selected > 0 {
        report.push(ValidationIssue::warning(
            "procedure_roots_empty",
            "procedure_policy.roots",
            "no procedure roots declared; only the package procedures directory is indexed",
        ));
    }
}

fn validate_memory_policy(
    definition: &AgentDefinition,
    constraints: &OperatorConstraints,
    report: &mut ValidationReport,
) {
    let policy = &definition.memory_policy;

    // Memory scope cannot be widened by a package (design §9.1). An entry the
    // operator has not permitted is dropped, and saying so is more useful than
    // letting the author assume it worked.
    if !constraints.allowed_memory_scopes.is_empty() {
        for scope in policy
            .allowed_scopes
            .difference(&constraints.allowed_memory_scopes)
        {
            report.push(ValidationIssue::error(
                "memory_scope_outside_operator_policy",
                "memory_policy.allowed_scopes",
                format!(
                    "memory scope {} is not permitted by operator policy",
                    quote(scope)
                ),
            ));
        }
    }
    if !constraints.allowed_memory_types.is_empty() {
        for memory_type in policy
            .allowed_types
            .difference(&constraints.allowed_memory_types)
        {
            report.push(ValidationIssue::error(
                "memory_type_outside_operator_policy",
                "memory_policy.allowed_types",
                format!(
                    "memory type {} is not permitted by operator policy",
                    quote(memory_type)
                ),
            ));
        }
    }
}

/// Reject credential-shaped and hook-shaped content.
///
/// `deny_unknown_fields` already rejects an unknown key such as `api_key` at
/// the top level, so this pass covers the places free-form strings survive:
/// `output_defaults.extra` keys and tag values.
fn validate_no_credentials_or_hooks(definition: &AgentDefinition, report: &mut ValidationReport) {
    let mut check_key = |key: &str, field: &str| {
        let lowered = key.to_ascii_lowercase();
        if CREDENTIAL_FIELD_TOKENS
            .iter()
            .any(|token| lowered.contains(token))
        {
            report.push(ValidationIssue::error(
                "credential_field_present",
                field,
                format!(
                    "{} looks like a credential field; definitions must not carry secrets",
                    quote(key)
                ),
            ));
        }
        if EXECUTABLE_HOOK_TOKENS
            .iter()
            .any(|token| lowered == *token || lowered.ends_with(&format!("_{token}")))
        {
            report.push(ValidationIssue::error(
                "executable_hook_present",
                field,
                format!(
                    "{} looks like an executable hook; packages must not execute anything",
                    quote(key)
                ),
            ));
        }
    };

    for key in definition.output_defaults.extra.keys() {
        check_key(key, "output_defaults.extra");
    }
    for tag in &definition.tags {
        check_key(tag, "tags");
    }
}

/// Confirm the canonical hash is reproducible for this value.
///
/// A definition whose hash is not stable cannot be pinned, so a resumed run
/// could not tell "unchanged" from "changed". Checking it here means the
/// failure surfaces at load rather than at the next resume.
fn validate_hash_stability(definition: &AgentDefinition, report: &mut ValidationReport) {
    let first = definition.manifest_hash();
    let round_tripped = serde_json::to_value(definition)
        .ok()
        .and_then(|value| serde_json::from_value::<AgentDefinition>(value).ok());

    match round_tripped {
        Some(round_tripped) if round_tripped.manifest_hash() == first => {}
        Some(_) => report.push(ValidationIssue::error(
            "unstable_content_hash",
            "manifest",
            "manifest hash is not stable across a serialization round trip",
        )),
        None => report.push(ValidationIssue::error(
            "manifest_not_round_trippable",
            "manifest",
            "manifest cannot be re-parsed from its own serialized form",
        )),
    }
}

/// Whether a required runtime feature is satisfied by the resolved model.
///
/// Kept separate from manifest validation because the model is only known once
/// routing has resolved. The design is explicit that an unmet requirement must
/// fail or degrade visibly, never be silently assumed (§8.6).
pub fn evaluate_feature_requirement(
    requirement: FeatureRequirement,
    available: bool,
) -> FeatureEvaluation {
    match (requirement, available) {
        (FeatureRequirement::Unused, _) => FeatureEvaluation::NotNeeded,
        (_, true) => FeatureEvaluation::Satisfied,
        (FeatureRequirement::Preferred, false) => FeatureEvaluation::Degraded,
        (FeatureRequirement::Required, false) => FeatureEvaluation::Unsatisfied,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureEvaluation {
    NotNeeded,
    Satisfied,
    /// Preferred but absent: run continues, degradation is recorded.
    Degraded,
    /// Required but absent: activation must fail or degrade by explicit policy.
    Unsatisfied,
}

impl FeatureEvaluation {
    pub fn blocks_activation(self) -> bool {
        matches!(self, Self::Unsatisfied)
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::NotNeeded => "not_needed",
            Self::Satisfied => "satisfied",
            Self::Degraded => "degraded",
            Self::Unsatisfied => "unsatisfied",
        }
    }
}

/// Resolve the effective capability set: operator allow ∩ agent allow, minus
/// every deny (design §16.3).
///
/// Written as one function because the ordering is the security property: deny
/// is applied last, so nothing can re-add a denied capability.
pub fn resolve_effective_capabilities(
    available: &BTreeSet<String>,
    agent_allow: &BTreeSet<String>,
    agent_deny: &BTreeSet<String>,
    operator_allow: &BTreeSet<String>,
    operator_deny: &BTreeSet<String>,
) -> BTreeSet<String> {
    available
        .iter()
        .filter(|capability| operator_allow.is_empty() || operator_allow.contains(*capability))
        .filter(|capability| agent_allow.is_empty() || agent_allow.contains(*capability))
        .filter(|capability| !agent_deny.contains(*capability))
        .filter(|capability| !operator_deny.contains(*capability))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::definition::legacy_definition;

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn definition(manifest: &str) -> AgentDefinition {
        toml::from_str(manifest).expect("manifest parses")
    }

    fn valid_manifest() -> AgentDefinition {
        definition(
            r#"
schema_version = 1
id = "ops"
definition_version = "1.0.0"
display_name = "Ops"
owner = "platform-ops"
default_instructions_path = "instructions.md"
"#,
        )
    }

    fn selector() -> AgentSelector {
        AgentSelector::parse("workspace:ops").unwrap()
    }

    #[test]
    fn a_valid_manifest_activates_without_errors() {
        let report = validate_definition(
            &valid_manifest(),
            &selector(),
            &OperatorConstraints::unconstrained(),
        );

        assert!(
            !report.blocks_activation(),
            "unexpected errors: {:?}",
            report.codes()
        );
        assert_eq!(report.errors().count(), 0);
    }

    #[test]
    fn the_legacy_definition_passes_its_own_validation() {
        let report = validate_definition(
            &legacy_definition(),
            &AgentSelector::legacy(),
            &OperatorConstraints::unconstrained(),
        );

        assert!(
            !report.blocks_activation(),
            "legacy must always activate: {:?}",
            report.codes()
        );
    }

    /// A package that loaded from `agents/a/` but calls itself `b` would make
    /// every recorded hash and event name the wrong agent.
    #[test]
    fn a_manifest_id_that_disagrees_with_its_directory_is_rejected() {
        let mut definition = valid_manifest();
        definition.id = "other".to_string();

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints::unconstrained(),
        );

        assert!(report.has_code("agent_id_mismatch"));
        assert!(report.blocks_activation());
    }

    #[test]
    fn an_unsupported_schema_version_blocks_activation() {
        let mut definition = valid_manifest();
        definition.schema_version = 99;

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints::unconstrained(),
        );

        assert!(report.has_code("unsupported_schema_version"));
        assert!(report.blocks_activation());
    }

    #[test]
    fn runtime_compatibility_windows_are_checked_in_both_directions() {
        let mut too_new = valid_manifest();
        too_new.runtime_compatibility.min_schema_version = 5;
        let report = validate_definition(
            &too_new,
            &selector(),
            &OperatorConstraints {
                runtime_schema_version: 1,
                ..OperatorConstraints::default()
            },
        );
        assert!(report.has_code("runtime_too_old"));

        let mut too_old = valid_manifest();
        too_old.runtime_compatibility.max_schema_version = Some(0);
        let report = validate_definition(
            &too_old,
            &selector(),
            &OperatorConstraints {
                runtime_schema_version: 1,
                ..OperatorConstraints::default()
            },
        );
        assert!(report.has_code("runtime_too_new"));
    }

    /// Path escapes are the primary way a manifest could read files it was
    /// never granted.
    #[test]
    fn referenced_paths_that_escape_the_package_are_rejected() {
        for path in [
            "../outside.md",
            "nested/../../outside.md",
            "/etc/passwd",
            "",
        ] {
            let issue = validate_package_relative_path(Path::new(path), "test")
                .expect_err(&format!("{path} must be rejected"));
            assert!(
                matches!(
                    issue.code.as_str(),
                    "path_escapes_package" | "absolute_path" | "empty_path"
                ),
                "{path} produced {}",
                issue.code
            );
        }

        // A benign nested path must still be accepted.
        assert!(validate_package_relative_path(Path::new("prompts/planner.md"), "test").is_ok());
        assert!(validate_package_relative_path(Path::new("./instructions.md"), "test").is_ok());
    }

    #[test]
    fn windows_style_absolute_paths_are_rejected() {
        let mut definition = valid_manifest();
        definition.default_instructions_path = Some(std::path::PathBuf::from(r"C:\secrets.md"));

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints::unconstrained(),
        );

        // On Windows this is a prefix component; elsewhere it is a plain
        // filename. Either way it must not be treated as an escape hatch.
        #[cfg(windows)]
        assert!(report.has_code("absolute_path"), "{:?}", report.codes());
        #[cfg(not(windows))]
        assert!(
            !report.has_code("path_escapes_package"),
            "{:?}",
            report.codes()
        );
    }

    #[test]
    fn capability_ids_must_be_well_formed() {
        assert!(validate_capability_id("workspace.file.read").is_ok());
        for bad in [
            "",
            "Workspace.File",
            "workspace..read",
            ".leading",
            "trailing.",
            "has space",
            "glob*",
            "../escape",
        ] {
            assert!(
                validate_capability_id(bad).is_err(),
                "{bad} must be rejected"
            );
        }
        assert!(validate_capability_id(&"a".repeat(MAX_CAPABILITY_ID_LEN + 1)).is_err());
    }

    #[test]
    fn a_capability_both_required_and_denied_cannot_activate() {
        let mut definition = valid_manifest();
        definition
            .capability_policy
            .required
            .insert("execution.shell.run".to_string());
        definition
            .capability_policy
            .deny
            .insert("execution.shell.run".to_string());

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints::unconstrained(),
        );

        assert!(report.has_code("required_capability_denied"));
        assert!(report.blocks_activation());
    }

    /// The core escalation attempt: a package requiring something the operator
    /// forbids must fail rather than be granted it.
    #[test]
    fn operator_deny_blocks_a_required_capability() {
        let mut definition = valid_manifest();
        definition
            .capability_policy
            .required
            .insert("execution.shell.run".to_string());

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints {
                denied_capabilities: set(&["execution.shell.run"]),
                runtime_schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
                ..OperatorConstraints::default()
            },
        );

        assert!(report.has_code("required_capability_denied_by_operator"));
        assert!(report.blocks_activation());
    }

    #[test]
    fn an_agent_allow_outside_operator_policy_is_inert_not_expansive() {
        let mut definition = valid_manifest();
        definition
            .capability_policy
            .allow
            .insert("execution.shell.run".to_string());

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints {
                allowed_capabilities: set(&["workspace.file.read"]),
                runtime_schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
                ..OperatorConstraints::default()
            },
        );

        // Inert, so a warning — but it must never silently become available.
        assert!(report.has_code("allow_outside_operator_policy"));
        assert!(!report.blocks_activation(), "{:?}", report.codes());
        assert_eq!(
            resolve_effective_capabilities(
                &set(&["workspace.file.read", "execution.shell.run"]),
                &definition.capability_policy.allow,
                &definition.capability_policy.deny,
                &set(&["workspace.file.read"]),
                &BTreeSet::new(),
            ),
            BTreeSet::new(),
            "an inert allow must not produce an available capability"
        );
    }

    #[test]
    fn deny_is_applied_last_so_nothing_can_re_add_a_denied_capability() {
        let available = set(&["a.read", "b.write", "c.exec"]);

        // Agent allow-lists everything, operator denies one entry.
        assert_eq!(
            resolve_effective_capabilities(
                &available,
                &set(&["a.read", "b.write", "c.exec"]),
                &BTreeSet::new(),
                &BTreeSet::new(),
                &set(&["c.exec"]),
            ),
            set(&["a.read", "b.write"])
        );

        // Agent deny also removes, even without an operator deny.
        assert_eq!(
            resolve_effective_capabilities(
                &available,
                &BTreeSet::new(),
                &set(&["b.write"]),
                &BTreeSet::new(),
                &BTreeSet::new(),
            ),
            set(&["a.read", "c.exec"])
        );

        // Intersection of both allow-lists, then deny.
        assert_eq!(
            resolve_effective_capabilities(
                &available,
                &set(&["a.read", "b.write"]),
                &BTreeSet::new(),
                &set(&["b.write", "c.exec"]),
                &set(&["b.write"]),
            ),
            BTreeSet::new()
        );
    }

    #[test]
    fn budget_requests_above_operator_caps_are_clamped_with_a_warning() {
        let mut definition = valid_manifest();
        definition.execution_defaults.max_steps = Some(500);

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints {
                max_steps_cap: Some(20),
                runtime_schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
                ..OperatorConstraints::default()
            },
        );

        assert!(report.has_code("budget_exceeds_operator_cap"));
        // Clamping is not a failure; the package simply does not get more.
        assert!(!report.blocks_activation(), "{:?}", report.codes());
        assert_eq!(
            super::super::authority::bounded_by_operator_cap(Some(500), Some(20)),
            Some(20)
        );
    }

    #[test]
    fn a_zero_or_unknown_execution_default_is_an_error() {
        let mut zero = valid_manifest();
        zero.execution_defaults.max_steps = Some(0);
        assert!(
            validate_definition(&zero, &selector(), &OperatorConstraints::unconstrained())
                .has_code("zero_budget")
        );

        let mut unknown = valid_manifest();
        unknown.execution_defaults.strategy = Some("freestyle".to_string());
        assert!(
            validate_definition(&unknown, &selector(), &OperatorConstraints::unconstrained())
                .has_code("unknown_execution_strategy")
        );
    }

    /// An uploaded document must never be selectable, so a package cannot
    /// opt into the untrusted trust level.
    #[test]
    fn external_untrusted_can_never_be_an_allowed_trust_level() {
        let mut definition = valid_manifest();
        definition
            .procedure_policy
            .allowed_trust_levels
            .insert("external_untrusted".to_string());

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints::unconstrained(),
        );

        assert!(report.has_code("external_untrusted_not_selectable"));
        assert!(report.blocks_activation());
    }

    #[test]
    fn an_unknown_trust_level_is_rejected() {
        let mut definition = valid_manifest();
        definition
            .procedure_policy
            .allowed_trust_levels
            .insert("totally_trusted".to_string());

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints::unconstrained(),
        );

        assert!(report.has_code("unknown_trust_level"));
        assert!(report.blocks_activation());
    }

    #[test]
    fn memory_scope_cannot_be_widened_by_a_package() {
        let mut definition = valid_manifest();
        definition
            .memory_policy
            .allowed_scopes
            .insert("global".to_string());

        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints {
                allowed_memory_scopes: set(&["project"]),
                runtime_schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
                ..OperatorConstraints::default()
            },
        );

        assert!(report.has_code("memory_scope_outside_operator_policy"));
        assert!(report.blocks_activation());
    }

    #[test]
    fn credential_shaped_and_hook_shaped_keys_are_rejected() {
        let mut definition = valid_manifest();
        definition
            .output_defaults
            .extra
            .insert("api_key".to_string(), "value".to_string());
        let report = validate_definition(
            &definition,
            &selector(),
            &OperatorConstraints::unconstrained(),
        );
        assert!(report.has_code("credential_field_present"));
        assert!(report.blocks_activation());

        let mut hooked = valid_manifest();
        hooked
            .output_defaults
            .extra
            .insert("postinstall".to_string(), "./run.sh".to_string());
        let report =
            validate_definition(&hooked, &selector(), &OperatorConstraints::unconstrained());
        assert!(report.has_code("executable_hook_present"));
        assert!(report.blocks_activation());
    }

    /// A top-level credential key is already impossible because the manifest
    /// denies unknown fields; this pins that as a deliberate guarantee.
    #[test]
    fn a_top_level_credential_key_fails_to_parse_at_all() {
        let manifest = r#"
schema_version = 1
id = "ops"
definition_version = "1.0.0"
display_name = "Ops"
api_key = "sk-live-not-a-real-key"
"#;
        assert!(toml::from_str::<AgentDefinition>(manifest).is_err());
    }

    #[test]
    fn issue_messages_are_bounded_and_control_character_free() {
        let issue =
            ValidationIssue::error("test", "field", format!("a\u{7}b\n{}", "x".repeat(500)));

        assert!(!issue.message.contains('\u{7}'));
        assert!(issue.message.len() <= 256, "len {}", issue.message.len());
    }

    /// A truncated report cannot prove that no error exists, so it must block.
    #[test]
    fn a_truncated_report_blocks_activation() {
        let mut report = ValidationReport::default();
        for index in 0..(MAX_VALIDATION_ISSUES + 5) {
            report.push(ValidationIssue::warning(
                "noise",
                "field",
                format!("issue {index}"),
            ));
        }

        assert!(report.truncated);
        assert_eq!(report.issues.len(), MAX_VALIDATION_ISSUES);
        assert!(report.blocks_activation());
    }

    #[test]
    fn required_features_that_are_absent_block_activation() {
        assert_eq!(
            evaluate_feature_requirement(FeatureRequirement::Required, false),
            FeatureEvaluation::Unsatisfied
        );
        assert!(
            evaluate_feature_requirement(FeatureRequirement::Required, false).blocks_activation()
        );
        assert_eq!(
            evaluate_feature_requirement(FeatureRequirement::Preferred, false),
            FeatureEvaluation::Degraded
        );
        assert!(
            !evaluate_feature_requirement(FeatureRequirement::Preferred, false).blocks_activation()
        );
        assert_eq!(
            evaluate_feature_requirement(FeatureRequirement::Unused, false),
            FeatureEvaluation::NotNeeded
        );
        assert_eq!(
            evaluate_feature_requirement(FeatureRequirement::Required, true),
            FeatureEvaluation::Satisfied
        );
    }
}
