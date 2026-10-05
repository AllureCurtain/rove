//! `AgentDefinition` — the versioned, author-maintained package source.
//!
//! This is the *source* side of design §8.1. It is deserialized from
//! `agents/<id>/agent.toml`, validated (see [`crate::agents::validation`]),
//! and only then compiled into an immutable
//! [`AgentRuntimeProfile`](crate::agents::profile::AgentRuntimeProfile).
//!
//! Two properties matter more than the field list:
//!
//! * **Nothing here grants authority.** `execution_defaults` are suggestions
//!   bounded by operator caps, and `capability_policy.allow` can only
//!   intersect what the operator already permits. A package cannot widen its
//!   own permissions by editing this file.
//! * **Ordered collections only.** Sets use `BTreeSet` so a manifest whose
//!   arrays were written in a different order still hashes identically.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::hashing::structured_hash;

/// Manifest schema version understood by this build.
pub const AGENT_DEFINITION_SCHEMA_VERSION: u16 = 1;

/// The parsed `agent.toml` manifest.
///
/// `deny_unknown_fields` is intentional: a typo'd or future-only key would
/// otherwise be silently dropped, and a dropped `deny` list is a security
/// regression rather than a cosmetic one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    pub schema_version: u16,
    pub id: String,
    /// Author-maintained version string. Distinct from the runtime-computed
    /// content hash, which is what pinning and resume actually compare.
    pub definition_version: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub runtime_compatibility: RuntimeCompatibility,
    /// Trusted operator policy text. Cannot be overridden by a user task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_instructions_path: Option<PathBuf>,
    /// Overridable role guidance and defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_instructions_path: Option<PathBuf>,
    #[serde(default)]
    pub prompt_slots: PromptSlots,
    #[serde(default)]
    pub execution_defaults: ExecutionDefaults,
    #[serde(default)]
    pub capability_policy: CapabilityPolicy,
    #[serde(default)]
    pub procedure_policy: ProcedurePolicy,
    #[serde(default)]
    pub memory_policy: MemoryPolicy,
    #[serde(default)]
    pub output_defaults: OutputDefaults,
    #[serde(default)]
    pub owner: String,
    #[serde(default)]
    pub tags: BTreeSet<String>,
}

/// Runtime feature requirements a definition may declare (design §8.6).
///
/// These describe *capability needs*, never a specific provider or model. A
/// definition that pinned a vendor would defeat the provider-neutral routing
/// layer, so no such field exists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeCompatibility {
    /// Minimum runtime schema this package expects.
    #[serde(default)]
    pub min_schema_version: u16,
    /// Maximum runtime schema, when the author knows of a later break.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_schema_version: Option<u16>,
    #[serde(default)]
    pub native_tool_use: FeatureRequirement,
    #[serde(default)]
    pub structured_output: FeatureRequirement,
    /// Minimum usable context window in tokens, when the package genuinely
    /// depends on one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_context_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_modalities: BTreeSet<String>,
}

/// How badly a package needs a runtime feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureRequirement {
    /// Not needed.
    #[default]
    Unused,
    /// Used when present; absence is fine.
    Preferred,
    /// Absence must fail activation or degrade explicitly — never be faked.
    Required,
}

impl FeatureRequirement {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unused => "unused",
            Self::Preferred => "preferred",
            Self::Required => "required",
        }
    }

    pub fn is_required(self) -> bool {
        matches!(self, Self::Required)
    }
}

/// Bounded prompt slot files. A slot supplies domain wording inside a
/// runtime-owned contract; it never replaces the contract (design §9.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptSlots {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluator: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replanner: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalizer: Option<PathBuf>,
}

/// Every declared slot as `(role, path)`, in a fixed order.
impl PromptSlots {
    pub fn declared(&self) -> Vec<(PromptSlotRole, &PathBuf)> {
        [
            (PromptSlotRole::Planner, self.planner.as_ref()),
            (PromptSlotRole::Evaluator, self.evaluator.as_ref()),
            (PromptSlotRole::Replanner, self.replanner.as_ref()),
            (PromptSlotRole::Finalizer, self.finalizer.as_ref()),
        ]
        .into_iter()
        .filter_map(|(role, path)| path.map(|path| (role, path)))
        .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.declared().is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSlotRole {
    Planner,
    Evaluator,
    Replanner,
    Finalizer,
}

impl PromptSlotRole {
    pub fn code(self) -> &'static str {
        match self {
            Self::Planner => "planner",
            Self::Evaluator => "evaluator",
            Self::Replanner => "replanner",
            Self::Finalizer => "finalizer",
        }
    }
}

/// Execution preferences a package may suggest.
///
/// Every field is `Option` because "unset" and "set to the default" are
/// different facts: an unset budget defers to the operator cap, while a set
/// one is an explicit request that still gets clamped by that cap (§8.7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionDefaults {
    /// Suggested strategy, e.g. `plan_react` or `react`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_procedure_selections: Option<u32>,
}

/// Capability requirements and restrictions (design §16.3).
///
/// `allow` is a *further restriction* of the operator's set, not an addition
/// to it, and `deny` always wins. Both are stored sorted so manifest ordering
/// cannot change the definition hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityPolicy {
    /// Must resolve to a concrete tool or activation fails.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required: BTreeSet<String>,
    /// Enables extra workflows; absence is not fatal.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub optional: BTreeSet<String>,
    /// When non-empty, narrows the visible set to this intersection.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allow: BTreeSet<String>,
    /// Always removed, even if also present in `allow` or `required`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub deny: BTreeSet<String>,
}

/// Which procedures this Agent may consider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcedurePolicy {
    /// Workspace-relative roots to index. Empty means the package's own
    /// `procedures/` directory only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_tags: BTreeSet<String>,
    /// Trust levels eligible for selection. Defaults to trusted sources only,
    /// so an unreviewed external document cannot be selected by accident.
    #[serde(default = "default_allowed_trust_levels")]
    pub allowed_trust_levels: BTreeSet<String>,
    /// Upper bound on selected procedures — a cap, not a quota to fill.
    #[serde(default = "default_max_selected")]
    pub max_selected: u32,
}

fn default_allowed_trust_levels() -> BTreeSet<String> {
    ["builtin_trusted", "workspace_trusted"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn default_max_selected() -> u32 {
    3
}

impl Default for ProcedurePolicy {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            required_tags: BTreeSet::new(),
            allowed_trust_levels: default_allowed_trust_levels(),
            max_selected: default_max_selected(),
        }
    }
}

/// Memory scope a package may use. Cannot widen operator memory policy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryPolicy {
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_scopes: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allowed_types: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall_limit: Option<u32>,
    #[serde(default)]
    pub promotion_mode: MemoryPromotionMode,
}

/// Whether the Agent may promote a run observation into durable memory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPromotionMode {
    /// No promotion. The conservative default.
    #[default]
    Disabled,
    /// Promotion requires an explicit user or operator decision.
    Explicit,
}

impl MemoryPromotionMode {
    pub fn code(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Explicit => "explicit",
        }
    }
}

/// Presentation preferences. A user task may override any of these (§7.2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl AgentDefinition {
    /// Canonical hash of the manifest's semantic content.
    ///
    /// Computed from the parsed value rather than the file bytes, so
    /// reordering TOML keys or reflowing whitespace does not invalidate a
    /// pinned run. The full package hash additionally covers referenced file
    /// contents; see [`crate::agents::package`].
    pub fn manifest_hash(&self) -> String {
        structured_hash("agent-definition", self)
    }

    /// Every file path the manifest references, in a fixed order.
    ///
    /// Validation walks exactly this list. Nothing else in the package
    /// directory is injectable, which is what keeps `README.md`, evals, and
    /// stray notes out of the prompt (§8.3).
    pub fn referenced_paths(&self) -> Vec<(ReferencedFileKind, &PathBuf)> {
        let mut paths = Vec::new();
        if let Some(path) = self.policy_instructions_path.as_ref() {
            paths.push((ReferencedFileKind::PolicyInstructions, path));
        }
        if let Some(path) = self.default_instructions_path.as_ref() {
            paths.push((ReferencedFileKind::DefaultInstructions, path));
        }
        for (role, path) in self.prompt_slots.declared() {
            paths.push((ReferencedFileKind::PromptSlot(role), path));
        }
        paths
    }
}

/// What role a referenced file plays, so validation can apply the right rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ReferencedFileKind {
    /// Trusted operator policy: authority [`TrustedOperatorPolicy`].
    ///
    /// [`TrustedOperatorPolicy`]: super::authority::AuthorityClass::TrustedOperatorPolicy
    PolicyInstructions,
    /// Overridable defaults: authority [`AgentDefaults`].
    ///
    /// [`AgentDefaults`]: super::authority::AuthorityClass::AgentDefaults
    DefaultInstructions,
    PromptSlot(PromptSlotRole),
}

impl ReferencedFileKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::PolicyInstructions => "policy_instructions",
            Self::DefaultInstructions => "default_instructions",
            Self::PromptSlot(PromptSlotRole::Planner) => "prompt_slot_planner",
            Self::PromptSlot(PromptSlotRole::Evaluator) => "prompt_slot_evaluator",
            Self::PromptSlot(PromptSlotRole::Replanner) => "prompt_slot_replanner",
            Self::PromptSlot(PromptSlotRole::Finalizer) => "prompt_slot_finalizer",
        }
    }

    /// Policy text is trusted operator policy; everything else is an
    /// overridable default or a bounded slot.
    pub fn authority(self) -> super::authority::AuthorityClass {
        match self {
            Self::PolicyInstructions => super::authority::AuthorityClass::TrustedOperatorPolicy,
            Self::DefaultInstructions | Self::PromptSlot(_) => {
                super::authority::AuthorityClass::AgentDefaults
            }
        }
    }
}

/// The built-in `legacy` definition.
///
/// This is how pre-AgentDefinition behaviour keeps working: the run still
/// takes its system and planner text from the configured prompt files, and the
/// package itself declares no policy, no capability restrictions, and no
/// procedure roots. It references no files, so it cannot fail validation for a
/// missing path — a compatibility path that itself needs a well-formed package
/// on disk would not be a compatibility path.
pub fn legacy_definition() -> AgentDefinition {
    AgentDefinition {
        schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
        id: super::selector::LEGACY_AGENT_ID.to_string(),
        definition_version: "1.0.0".to_string(),
        display_name: "Legacy".to_string(),
        description: "Compatibility profile reproducing pre-AgentDefinition behaviour.".to_string(),
        runtime_compatibility: RuntimeCompatibility {
            min_schema_version: AGENT_DEFINITION_SCHEMA_VERSION,
            ..RuntimeCompatibility::default()
        },
        policy_instructions_path: None,
        default_instructions_path: None,
        prompt_slots: PromptSlots::default(),
        execution_defaults: ExecutionDefaults::default(),
        capability_policy: CapabilityPolicy::default(),
        // No roots: legacy runs do not gain procedure selection implicitly.
        procedure_policy: ProcedurePolicy {
            roots: Vec::new(),
            required_tags: BTreeSet::new(),
            allowed_trust_levels: default_allowed_trust_levels(),
            max_selected: 0,
        },
        memory_policy: MemoryPolicy::default(),
        output_defaults: OutputDefaults::default(),
        owner: "rove".to_string(),
        tags: BTreeSet::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_manifest() -> &'static str {
        r#"
schema_version = 1
id = "ops-diagnostic"
definition_version = "1.2.0"
display_name = "Ops Diagnostic"
"#
    }

    #[test]
    fn a_minimal_manifest_parses_with_conservative_defaults() {
        let definition: AgentDefinition = toml::from_str(minimal_manifest()).unwrap();

        assert_eq!(definition.id, "ops-diagnostic");
        assert_eq!(definition.definition_version, "1.2.0");
        assert!(definition.referenced_paths().is_empty());
        // Defaults must be the safe end of every axis.
        assert_eq!(
            definition.memory_policy.promotion_mode,
            MemoryPromotionMode::Disabled
        );
        assert_eq!(
            definition.runtime_compatibility.native_tool_use,
            FeatureRequirement::Unused
        );
        assert!(definition.capability_policy.deny.is_empty());
        assert_eq!(
            definition.procedure_policy.allowed_trust_levels,
            default_allowed_trust_levels()
        );
    }

    /// A silently dropped key could be a dropped `deny` list, so unknown
    /// fields must be a parse error rather than a shrug.
    #[test]
    fn an_unknown_manifest_field_is_a_parse_error() {
        let manifest = format!("{}\nunknown_future_key = true\n", minimal_manifest());
        let error = toml::from_str::<AgentDefinition>(&manifest).unwrap_err();
        assert!(
            error.to_string().contains("unknown"),
            "unexpected error: {error}"
        );

        let nested = format!(
            "{}\n[capability_policy]\nrequired = []\nsurprise = 1\n",
            minimal_manifest()
        );
        assert!(toml::from_str::<AgentDefinition>(&nested).is_err());
    }

    #[test]
    fn manifest_hash_ignores_key_order_and_array_order() {
        let first: AgentDefinition = toml::from_str(
            r#"
schema_version = 1
id = "a"
definition_version = "1.0.0"
display_name = "A"
tags = ["z", "a"]

[capability_policy]
required = ["workspace.file.read", "execution.shell.run"]
"#,
        )
        .unwrap();
        let second: AgentDefinition = toml::from_str(
            r#"
display_name = "A"
definition_version = "1.0.0"
id = "a"
schema_version = 1
tags = ["a", "z"]

[capability_policy]
required = ["execution.shell.run", "workspace.file.read"]
"#,
        )
        .unwrap();

        assert_eq!(first.manifest_hash(), second.manifest_hash());
        assert!(first.manifest_hash().starts_with("sha256:"));
    }

    #[test]
    fn manifest_hash_changes_when_policy_changes() {
        let base: AgentDefinition = toml::from_str(minimal_manifest()).unwrap();
        let mut tightened = base.clone();
        tightened
            .capability_policy
            .deny
            .insert("execution.shell.run".to_string());

        assert_ne!(base.manifest_hash(), tightened.manifest_hash());
    }

    #[test]
    fn referenced_paths_lists_only_declared_injectable_files() {
        let definition: AgentDefinition = toml::from_str(
            r#"
schema_version = 1
id = "a"
definition_version = "1.0.0"
display_name = "A"
policy_instructions_path = "policy.md"
default_instructions_path = "instructions.md"

[prompt_slots]
planner = "prompts/planner.md"
finalizer = "prompts/finalizer.md"
"#,
        )
        .unwrap();

        let kinds = definition
            .referenced_paths()
            .into_iter()
            .map(|(kind, path)| (kind.code(), path.to_string_lossy().replace('\\', "/")))
            .collect::<Vec<_>>();

        assert_eq!(
            kinds,
            vec![
                ("policy_instructions", "policy.md".to_string()),
                ("default_instructions", "instructions.md".to_string()),
                ("prompt_slot_planner", "prompts/planner.md".to_string()),
                ("prompt_slot_finalizer", "prompts/finalizer.md".to_string()),
            ]
        );
    }

    #[test]
    fn policy_text_outranks_default_instructions() {
        assert!(
            ReferencedFileKind::PolicyInstructions
                .authority()
                .outranks(ReferencedFileKind::DefaultInstructions.authority())
        );
        assert!(
            ReferencedFileKind::PolicyInstructions
                .authority()
                .outranks(ReferencedFileKind::PromptSlot(PromptSlotRole::Planner).authority())
        );
    }

    #[test]
    fn the_legacy_definition_references_no_files_and_selects_no_procedures() {
        let legacy = legacy_definition();

        assert_eq!(legacy.id, "legacy");
        assert!(legacy.referenced_paths().is_empty());
        assert_eq!(legacy.procedure_policy.max_selected, 0);
        assert!(legacy.procedure_policy.roots.is_empty());
        assert!(legacy.capability_policy.required.is_empty());
        assert!(legacy.capability_policy.allow.is_empty());
        assert_eq!(
            legacy.memory_policy.promotion_mode,
            MemoryPromotionMode::Disabled
        );
        // The compatibility profile must be stable across builds.
        assert_eq!(legacy.manifest_hash(), legacy_definition().manifest_hash());
    }

    #[test]
    fn feature_requirements_serialize_as_stable_codes() {
        assert_eq!(
            serde_json::to_value(FeatureRequirement::Required).unwrap(),
            serde_json::json!("required")
        );
        assert!(FeatureRequirement::Required.is_required());
        assert!(!FeatureRequirement::Preferred.is_required());
    }
}
