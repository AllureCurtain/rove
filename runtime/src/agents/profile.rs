//! `AgentRuntimeProfile` — the immutable compiled run snapshot (design §8.1).
//!
//! An [`AgentDefinition`] is *source*: author-maintained, editable, and only a
//! request. A profile is what a run actually executes against. Compiling one
//! resolves every request against operator policy once, at activation, and then
//! freezes the result:
//!
//! * execution bounds are **clamped** by operator caps, never merely compared,
//! * the capability set is `operator allow ∩ agent allow − every deny`, so a
//!   package cannot widen its own permissions (§8.7, §16.3),
//! * the instruction bundle and procedure selection are pinned by content hash,
//!   so a mid-run file edit cannot change what the run believes it agreed to.
//!
//! The type carries no interior mutability and exposes no setters. Everything
//! needed to explain or replay a decision is a field, because a bound that was
//! recomputed at read time could disagree with the one the run was admitted
//! under.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::definition::{
    ExecutionDefaults, PromptSlotRole, ReferencedFileKind, RuntimeCompatibility,
};
use super::hashing::composite_hash;
use super::instructions::InstructionBundle;
use super::package::AgentPackage;
use super::procedure::hydration::HydratedProcedure;
use super::procedure::selection::ProcedureSelection;
use super::selector::AgentSelector;
use super::validation::{
    FeatureEvaluation, OperatorConstraints, ValidationReport, evaluate_feature_requirement,
    resolve_effective_capabilities,
};

/// Profile schema version, recorded in the snapshot so an old artifact can be
/// read back without guessing its shape.
pub const AGENT_PROFILE_SCHEMA_VERSION: u16 = 1;

/// Why a profile could not be compiled.
///
/// Every variant is a refusal to start, not a degradation. A run that cannot
/// prove its bounds must not begin: a half-resolved profile would be a profile
/// nobody authorized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "error")]
pub enum ProfileBuildError {
    #[error("package '{agent_id}' cannot activate: validation produced errors")]
    PackageBlocked { agent_id: String },
    #[error("required capability '{capability}' is not available in the resolved set")]
    RequiredCapabilityUnavailable { capability: String },
    #[error("required capability '{capability}' is denied by policy")]
    RequiredCapabilityDenied { capability: String },
    #[error("required runtime feature '{feature}' is unsatisfied by the resolved model")]
    RequiredFeatureUnsatisfied { feature: String },
    #[error(
        "resolved context window {available} tokens is below the {required} tokens the package requires"
    )]
    ContextWindowTooSmall { required: u32, available: u32 },
    #[error("required modality '{modality}' is not offered by the resolved model")]
    RequiredModalityUnavailable { modality: String },
    #[error("hydrated procedure '{id}' was not selected by this profile")]
    HydratedProcedureNotSelected { id: String },
    #[error("hydrated procedure '{id}' does not match the selected identity")]
    HydratedProcedureIdentityMismatch { id: String },
    #[error("hydrated procedure '{id}' has an invalid body hash")]
    HydratedProcedureBodyHashMismatch { id: String },
    #[error("agent profile snapshot hash does not match its content")]
    SnapshotHashMismatch,
}

impl ProfileBuildError {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::PackageBlocked { .. } => "package_blocked",
            Self::RequiredCapabilityUnavailable { .. } => "required_capability_unavailable",
            Self::RequiredCapabilityDenied { .. } => "required_capability_denied",
            Self::RequiredFeatureUnsatisfied { .. } => "required_feature_unsatisfied",
            Self::ContextWindowTooSmall { .. } => "context_window_too_small",
            Self::RequiredModalityUnavailable { .. } => "required_modality_unavailable",
            Self::HydratedProcedureNotSelected { .. } => "hydrated_procedure_not_selected",
            Self::HydratedProcedureIdentityMismatch { .. } => {
                "hydrated_procedure_identity_mismatch"
            }
            Self::HydratedProcedureBodyHashMismatch { .. } => {
                "hydrated_procedure_body_hash_mismatch"
            }
            Self::SnapshotHashMismatch => "snapshot_hash_mismatch",
        }
    }
}

/// What the resolved model can actually do.
///
/// Supplied by routing rather than read from the definition: a package declares
/// *needs*, and only the runtime knows what was resolved. Keeping this separate
/// is what stops a manifest from asserting a capability into existence (§8.6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedRuntimeFacts {
    /// Capability IDs the runtime can actually dispatch right now.
    pub available_capabilities: BTreeSet<String>,
    pub native_tool_use: bool,
    pub structured_output: bool,
    /// Usable context window in tokens, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub modalities: BTreeSet<String>,
}

/// Execution bounds after clamping, plus what was asked for.
///
/// Both sides are retained deliberately. "Requested 200, granted 50" is an
/// operator-visible fact, and reporting only the granted number would hide a
/// package quietly asking for more than it may have.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedBound {
    /// What the package asked for, if it asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested: Option<u32>,
    /// The operator cap in force, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cap: Option<u32>,
    /// The value the run executes under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective: Option<u32>,
}

impl ResolvedBound {
    /// Resolve one bound: the request, clamped by the cap.
    ///
    /// An absent request defers to the cap rather than to an invented default,
    /// and an absent cap leaves the request intact. `min` is the whole rule —
    /// there is no branch in which the effective value exceeds the cap.
    pub fn resolve(requested: Option<u32>, cap: Option<u32>) -> Self {
        let effective = match (requested, cap) {
            (Some(requested), Some(cap)) => Some(requested.min(cap)),
            (Some(requested), None) => Some(requested),
            (None, Some(cap)) => Some(cap),
            (None, None) => None,
        };
        Self {
            requested,
            cap,
            effective,
        }
    }

    /// Whether the request was reduced by the cap. Worth surfacing: the package
    /// is running under tighter limits than its author expected.
    pub fn was_clamped(&self) -> bool {
        match (self.requested, self.effective) {
            (Some(requested), Some(effective)) => effective < requested,
            _ => false,
        }
    }
}

/// The immutable compiled profile a run executes against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeProfile {
    pub schema_version: u16,
    pub selector: AgentSelector,
    pub agent_id: String,
    pub display_name: String,
    /// Author-declared version. Human-facing; not what pinning compares.
    pub definition_version: String,
    /// Hash of the manifest's semantic content.
    pub manifest_hash: String,
    /// Hash over the manifest and every injected file.
    pub package_hash: String,
    /// Resolved strategy name, when the package suggested one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    pub max_steps: ResolvedBound,
    pub max_tool_calls: ResolvedBound,
    pub max_procedure_selections: ResolvedBound,
    /// The capability set the run may dispatch. Already intersected and denied.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub effective_capabilities: BTreeSet<String>,
    /// Capabilities the package listed as optional that were not available.
    /// Not fatal, but a run behaving differently than its author expected is
    /// worth recording.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub unavailable_optional_capabilities: BTreeSet<String>,
    /// Feature requirement outcomes, keyed by feature name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub feature_evaluations: BTreeMap<String, FeatureEvaluation>,
    /// Injected policy text, already bounded and canonicalized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_instructions: Option<String>,
    /// Prompt slot text keyed by role code.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prompt_slots: BTreeMap<String, String>,
    /// Workspace instruction bundle in force for this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<InstructionBundle>,
    /// Procedure selection pinned at activation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub procedures: Option<ProcedureSelection>,
    /// Exact bounded procedure bodies used by the run. These remain in the
    /// task/checkpoint snapshot and are never projected into events or reports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hydrated_procedures: Vec<HydratedProcedure>,
    /// Validation warnings carried forward. Errors would have refused the
    /// build, so anything here is advisory by construction.
    pub validation: ValidationReport,
    /// Identity of the whole profile.
    pub profile_hash: String,
}

impl AgentRuntimeProfile {
    /// Effective step budget, or `None` when neither side bounded it.
    pub fn effective_max_steps(&self) -> Option<u32> {
        self.max_steps.effective
    }

    /// Whether a capability may be dispatched under this profile.
    ///
    /// The single question every tool dispatch should ask. It consults the
    /// frozen set rather than recomputing, so a policy reload mid-run cannot
    /// silently widen what an in-flight run may do.
    pub fn allows_capability(&self, capability: &str) -> bool {
        self.effective_capabilities.contains(capability)
    }

    /// Prompt slot text for a role, if the package supplied one.
    pub fn prompt_slot(&self, role: PromptSlotRole) -> Option<&str> {
        self.prompt_slots.get(role.code()).map(String::as_str)
    }

    /// Whether this profile is the legacy compatibility profile.
    pub fn is_legacy(&self) -> bool {
        self.selector.is_legacy()
    }

    /// Bounds that were reduced by an operator cap, by field name.
    pub fn clamped_bounds(&self) -> Vec<&'static str> {
        [
            ("max_steps", &self.max_steps),
            ("max_tool_calls", &self.max_tool_calls),
            ("max_procedure_selections", &self.max_procedure_selections),
        ]
        .into_iter()
        .filter(|(_, bound)| bound.was_clamped())
        .map(|(name, _)| name)
        .collect()
    }

    /// Identity components a run record pins, in a fixed order.
    pub fn identity_components(&self) -> Vec<String> {
        let mut components = vec![
            format!("agent:{}", self.agent_id),
            format!("source:{}", self.selector.source.code()),
            format!("manifest:{}", self.manifest_hash),
            format!("package:{}", self.package_hash),
        ];
        if let Some(bundle) = self.instructions.as_ref() {
            components.push(format!("instructions:{}", bundle.bundle_hash()));
        }
        if let Some(selection) = self.procedures.as_ref() {
            for component in selection.identity_components() {
                components.push(format!("procedure:{component}"));
            }
        }
        for procedure in &self.hydrated_procedures {
            components.push(format!(
                "hydrated:{}@{}#{}",
                procedure.reference.id, procedure.reference.version, procedure.body_hash
            ));
        }
        components
    }

    /// Validate a persisted profile before exact resume.
    pub fn validate_snapshot(&self) -> Result<(), ProfileBuildError> {
        for hydrated in &self.hydrated_procedures {
            let expected_body_hash =
                super::hashing::content_hash("hydrated-procedure-body", &hydrated.body);
            if hydrated.body_hash != expected_body_hash {
                return Err(ProfileBuildError::HydratedProcedureBodyHashMismatch {
                    id: hydrated.reference.id.clone(),
                });
            }
            let selected = self
                .procedures
                .as_ref()
                .and_then(|selection| {
                    selection
                        .selected
                        .iter()
                        .find(|selected| selected.reference.id == hydrated.reference.id)
                })
                .ok_or_else(|| ProfileBuildError::HydratedProcedureNotSelected {
                    id: hydrated.reference.id.clone(),
                })?;
            if selected.reference != hydrated.reference {
                return Err(ProfileBuildError::HydratedProcedureIdentityMismatch {
                    id: hydrated.reference.id.clone(),
                });
            }
        }

        let expected = compute_profile_hash(self);
        if self.profile_hash != expected {
            return Err(ProfileBuildError::SnapshotHashMismatch);
        }
        Ok(())
    }
}

/// Everything needed to compile a profile, gathered in one place so the build
/// is a pure function of its inputs and therefore reproducible on resume.
#[derive(Debug, Clone)]
pub struct ProfileInputs<'a> {
    pub package: &'a AgentPackage,
    pub constraints: &'a OperatorConstraints,
    pub facts: &'a ResolvedRuntimeFacts,
    pub instructions: Option<&'a InstructionBundle>,
    pub procedures: Option<&'a ProcedureSelection>,
}

/// Compile an [`AgentPackage`] into an immutable [`AgentRuntimeProfile`].
///
/// Order is the security property, so it is fixed here rather than left to a
/// caller: activation eligibility, then capability resolution, then required
/// checks against the *resolved* set, then feature checks, and only then the
/// snapshot. Checking a requirement against the requested set instead of the
/// resolved one is exactly the mistake that would let a denied capability pass.
pub fn build_runtime_profile(
    inputs: ProfileInputs<'_>,
) -> Result<AgentRuntimeProfile, ProfileBuildError> {
    let ProfileInputs {
        package,
        constraints,
        facts,
        instructions,
        procedures,
    } = inputs;

    // A package that failed validation never reaches resolution.
    if !package.may_activate() {
        return Err(ProfileBuildError::PackageBlocked {
            agent_id: package.definition.id.clone(),
        });
    }

    let definition = &package.definition;
    let policy = &definition.capability_policy;

    let effective_capabilities = resolve_effective_capabilities(
        &facts.available_capabilities,
        &policy.allow,
        &policy.deny,
        &constraints.allowed_capabilities,
        &constraints.denied_capabilities,
    );

    // Required capabilities are checked against the resolved set. A deny is
    // reported distinctly from mere absence: "policy forbids this" and "this
    // build has no such tool" need different operator responses.
    for capability in &policy.required {
        if effective_capabilities.contains(capability) {
            continue;
        }
        if policy.deny.contains(capability) || constraints.denied_capabilities.contains(capability)
        {
            return Err(ProfileBuildError::RequiredCapabilityDenied {
                capability: capability.clone(),
            });
        }
        return Err(ProfileBuildError::RequiredCapabilityUnavailable {
            capability: capability.clone(),
        });
    }

    let unavailable_optional_capabilities = policy
        .optional
        .iter()
        .filter(|capability| !effective_capabilities.contains(*capability))
        .cloned()
        .collect();

    let feature_evaluations = evaluate_features(&definition.runtime_compatibility, facts)?;

    let ExecutionDefaults {
        strategy,
        max_steps,
        max_tool_calls,
        max_procedure_selections,
    } = definition.execution_defaults.clone();

    let profile = AgentRuntimeProfile {
        schema_version: AGENT_PROFILE_SCHEMA_VERSION,
        selector: package.selector.clone(),
        agent_id: definition.id.clone(),
        display_name: definition.display_name.clone(),
        definition_version: definition.definition_version.clone(),
        manifest_hash: definition.manifest_hash(),
        package_hash: package.package_hash.clone(),
        strategy,
        max_steps: ResolvedBound::resolve(max_steps, constraints.max_steps_cap),
        max_tool_calls: ResolvedBound::resolve(max_tool_calls, constraints.max_tool_calls_cap),
        max_procedure_selections: ResolvedBound::resolve(
            max_procedure_selections.or(Some(definition.procedure_policy.max_selected)),
            constraints.max_procedure_selections_cap,
        ),
        effective_capabilities,
        unavailable_optional_capabilities,
        feature_evaluations,
        policy_instructions: injected_text(package, &ReferencedFileKind::PolicyInstructions),
        default_instructions: injected_text(package, &ReferencedFileKind::DefaultInstructions),
        prompt_slots: collect_prompt_slots(package),
        instructions: instructions.cloned(),
        procedures: procedures.cloned(),
        hydrated_procedures: Vec::new(),
        validation: package.validation.clone(),
        // Filled in below: the hash covers the finished snapshot.
        profile_hash: String::new(),
    };

    Ok(finalize_hash(profile))
}

/// Evaluate every declared runtime feature requirement.
///
/// A `Required` feature that is absent fails the build. Design §8.6 is explicit
/// that this must not be silently assumed: a package that needs native tool use
/// and does not get it would otherwise emit text the runtime cannot dispatch.
fn evaluate_features(
    compatibility: &RuntimeCompatibility,
    facts: &ResolvedRuntimeFacts,
) -> Result<BTreeMap<String, FeatureEvaluation>, ProfileBuildError> {
    let mut evaluations = BTreeMap::new();

    for (name, requirement, available) in [
        (
            "native_tool_use",
            compatibility.native_tool_use,
            facts.native_tool_use,
        ),
        (
            "structured_output",
            compatibility.structured_output,
            facts.structured_output,
        ),
    ] {
        let evaluation = evaluate_feature_requirement(requirement, available);
        if evaluation.blocks_activation() {
            return Err(ProfileBuildError::RequiredFeatureUnsatisfied {
                feature: name.to_string(),
            });
        }
        evaluations.insert(name.to_string(), evaluation);
    }

    // An unknown context window is not treated as satisfying a hard minimum:
    // assuming it fits is how a run silently truncates its own instructions.
    if let Some(required) = compatibility.min_context_tokens {
        let available = facts.context_tokens.unwrap_or(0);
        if available < required {
            return Err(ProfileBuildError::ContextWindowTooSmall {
                required,
                available,
            });
        }
    }

    for modality in &compatibility.required_modalities {
        if !facts.modalities.contains(modality) {
            return Err(ProfileBuildError::RequiredModalityUnavailable {
                modality: modality.clone(),
            });
        }
    }

    Ok(evaluations)
}

fn injected_text(package: &AgentPackage, kind: &ReferencedFileKind) -> Option<String> {
    package.file(kind).map(|file| file.text.clone())
}

fn collect_prompt_slots(package: &AgentPackage) -> BTreeMap<String, String> {
    [
        PromptSlotRole::Planner,
        PromptSlotRole::Evaluator,
        PromptSlotRole::Replanner,
        PromptSlotRole::Finalizer,
    ]
    .into_iter()
    .filter_map(|role| {
        package
            .file(&ReferencedFileKind::PromptSlot(role))
            .map(|file| (role.code().to_string(), file.text.clone()))
    })
    .collect()
}

/// Compute and attach the profile hash.
///
/// The hash covers the *resolved* snapshot, not the request, so two runs with
/// identical manifests but different operator caps are correctly distinct
/// identities — a resume must not replay under bounds it was never granted.
fn compute_profile_hash(profile: &AgentRuntimeProfile) -> String {
    let mut components = profile.identity_components();
    components.push(format!(
        "bounds:{:?}/{:?}/{:?}",
        profile.max_steps.effective,
        profile.max_tool_calls.effective,
        profile.max_procedure_selections.effective
    ));
    components.push(format!(
        "capabilities:{}",
        profile
            .effective_capabilities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    ));
    if let Some(strategy) = profile.strategy.as_ref() {
        components.push(format!("strategy:{strategy}"));
    }

    let borrowed: Vec<&str> = components.iter().map(String::as_str).collect();
    composite_hash("agent-runtime-profile", &borrowed)
}

fn finalize_hash(mut profile: AgentRuntimeProfile) -> AgentRuntimeProfile {
    profile.profile_hash = compute_profile_hash(&profile);
    profile
}

/// Attach the exact bounded bodies admitted for selected procedures.
pub fn attach_hydrated_procedures(
    mut profile: AgentRuntimeProfile,
    hydrated: Vec<HydratedProcedure>,
) -> Result<AgentRuntimeProfile, ProfileBuildError> {
    let selection = profile.procedures.as_ref();
    let mut seen = BTreeSet::new();
    for procedure in &hydrated {
        if !seen.insert(procedure.reference.id.clone()) {
            return Err(ProfileBuildError::HydratedProcedureIdentityMismatch {
                id: procedure.reference.id.clone(),
            });
        }
        let selected = selection
            .and_then(|selection| {
                selection
                    .selected
                    .iter()
                    .find(|selected| selected.reference.id == procedure.reference.id)
            })
            .ok_or_else(|| ProfileBuildError::HydratedProcedureNotSelected {
                id: procedure.reference.id.clone(),
            })?;
        if selected.reference != procedure.reference {
            return Err(ProfileBuildError::HydratedProcedureIdentityMismatch {
                id: procedure.reference.id.clone(),
            });
        }
        let expected = super::hashing::content_hash("hydrated-procedure-body", &procedure.body);
        if procedure.body_hash != expected {
            return Err(ProfileBuildError::HydratedProcedureBodyHashMismatch {
                id: procedure.reference.id.clone(),
            });
        }
    }
    profile.hydrated_procedures = hydrated;
    Ok(finalize_hash(profile))
}

/// Attach the exact workspace instruction snapshot used for this run.
///
/// Primarily used by the builtin legacy profile, which has no package build
/// step but still participates in trusted workspace instruction discovery.
pub fn attach_instruction_bundle(
    mut profile: AgentRuntimeProfile,
    instructions: Option<InstructionBundle>,
) -> AgentRuntimeProfile {
    profile.instructions = instructions;
    finalize_hash(profile)
}

/// The compatibility profile used when no Agent is configured.
///
/// Kept as a real compiled profile rather than an `Option<AgentRuntimeProfile>`
/// at every call site: a run always has a profile, so consumers never need a
/// "no agent" branch that would inevitably drift from the real one. It grants
/// nothing — no policy text, no procedures, and only the capabilities the
/// operator already permits.
pub fn legacy_profile(
    constraints: &OperatorConstraints,
    facts: &ResolvedRuntimeFacts,
) -> AgentRuntimeProfile {
    let definition = super::definition::legacy_definition();
    let effective_capabilities = resolve_effective_capabilities(
        &facts.available_capabilities,
        &BTreeSet::new(),
        &BTreeSet::new(),
        &constraints.allowed_capabilities,
        &constraints.denied_capabilities,
    );

    let profile = AgentRuntimeProfile {
        schema_version: AGENT_PROFILE_SCHEMA_VERSION,
        selector: AgentSelector::legacy(),
        agent_id: definition.id.clone(),
        display_name: definition.display_name.clone(),
        definition_version: definition.definition_version.clone(),
        manifest_hash: definition.manifest_hash(),
        // No package on disk, and saying so is more honest than inventing a
        // hash over an empty directory.
        package_hash: composite_hash("agent-package", &["legacy"]),
        strategy: None,
        max_steps: ResolvedBound::resolve(None, constraints.max_steps_cap),
        max_tool_calls: ResolvedBound::resolve(None, constraints.max_tool_calls_cap),
        max_procedure_selections: ResolvedBound::resolve(
            Some(0),
            constraints.max_procedure_selections_cap,
        ),
        effective_capabilities,
        unavailable_optional_capabilities: BTreeSet::new(),
        feature_evaluations: BTreeMap::new(),
        policy_instructions: None,
        default_instructions: None,
        prompt_slots: BTreeMap::new(),
        instructions: None,
        procedures: None,
        hydrated_procedures: Vec::new(),
        validation: ValidationReport::default(),
        profile_hash: String::new(),
    };

    finalize_hash(profile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::instructions::DiscoveredInstruction;
    use crate::agents::package::load_agent_package;
    use crate::agents::procedure::catalog::{EligibilityContext, ProcedureCatalog};
    use crate::agents::procedure::selection::{SelectionRequest, select_procedures};
    use std::fs;
    use tempfile::TempDir;

    const BASE_MANIFEST: &str = r#"
schema_version = 1
id = "ops"
definition_version = "1.0.0"
display_name = "Ops"
"#;

    fn selector() -> AgentSelector {
        AgentSelector::parse("workspace:ops").unwrap()
    }

    /// Build a package on disk and load it through the real loader, so a
    /// profile is only ever compiled from something that actually validated.
    fn package_with(extra_manifest: &str, files: &[(&str, &str)]) -> (TempDir, AgentPackage) {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("agents").join("ops");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("agent.toml"),
            format!("{BASE_MANIFEST}{extra_manifest}"),
        )
        .unwrap();
        for (relative, contents) in files {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        let package =
            load_agent_package(&root, &selector(), &OperatorConstraints::unconstrained()).unwrap();
        (temp, package)
    }

    fn facts(capabilities: &[&str]) -> ResolvedRuntimeFacts {
        ResolvedRuntimeFacts {
            available_capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
            native_tool_use: true,
            structured_output: true,
            context_tokens: Some(128_000),
            modalities: BTreeSet::from(["text".to_string()]),
        }
    }

    fn set(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn build(
        package: &AgentPackage,
        constraints: &OperatorConstraints,
        facts: &ResolvedRuntimeFacts,
    ) -> Result<AgentRuntimeProfile, ProfileBuildError> {
        build_runtime_profile(ProfileInputs {
            package,
            constraints,
            facts,
            instructions: None,
            procedures: None,
        })
    }

    #[test]
    fn a_minimal_package_compiles_into_a_profile() {
        let (_temp, package) = package_with("", &[]);
        let profile = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap();

        assert_eq!(profile.schema_version, AGENT_PROFILE_SCHEMA_VERSION);
        assert_eq!(profile.agent_id, "ops");
        assert!(!profile.is_legacy());
        assert!(profile.allows_capability("fs.read"));
        assert!(!profile.profile_hash.is_empty());
        assert!(profile.clamped_bounds().is_empty());
    }

    #[test]
    fn referenced_instruction_and_slot_text_is_carried_into_the_profile() {
        let (_temp, package) = package_with(
            "policy_instructions_path = \"policy.md\"\ndefault_instructions_path = \"instructions.md\"\n\n[prompt_slots]\nplanner = \"prompts/planner.md\"\n",
            &[
                ("policy.md", "Never touch production.\n"),
                ("instructions.md", "Prefer read-only checks.\n"),
                ("prompts/planner.md", "Plan in small steps.\n"),
            ],
        );
        let profile = build(&package, &OperatorConstraints::unconstrained(), &facts(&[])).unwrap();

        assert!(
            profile
                .policy_instructions
                .as_deref()
                .unwrap()
                .contains("Never touch production.")
        );
        assert!(profile.default_instructions.is_some());
        assert_eq!(
            profile.prompt_slot(PromptSlotRole::Planner),
            Some("Plan in small steps.")
        );
        assert_eq!(
            profile.prompt_slot(PromptSlotRole::Evaluator),
            None,
            "an undeclared slot must not be invented"
        );
    }

    // --- bound clamping (§8.7) ---

    #[test]
    fn a_request_above_the_operator_cap_is_clamped_not_honoured() {
        let (_temp, package) = package_with(
            "[execution_defaults]\nmax_steps = 500\nmax_tool_calls = 900\n",
            &[],
        );
        let constraints = OperatorConstraints {
            max_steps_cap: Some(50),
            max_tool_calls_cap: Some(80),
            ..OperatorConstraints::unconstrained()
        };

        let profile = build(&package, &constraints, &facts(&[])).unwrap();

        assert_eq!(profile.effective_max_steps(), Some(50));
        assert_eq!(profile.max_steps.requested, Some(500));
        assert_eq!(profile.max_steps.cap, Some(50));
        assert!(profile.max_steps.was_clamped());
        assert_eq!(profile.max_tool_calls.effective, Some(80));
        assert_eq!(
            profile.clamped_bounds(),
            vec!["max_steps", "max_tool_calls"],
            "both reductions must be operator-visible"
        );
    }

    #[test]
    fn a_request_below_the_cap_is_honoured_untouched() {
        let (_temp, package) = package_with("[execution_defaults]\nmax_steps = 10\n", &[]);
        let constraints = OperatorConstraints {
            max_steps_cap: Some(50),
            ..OperatorConstraints::unconstrained()
        };

        let profile = build(&package, &constraints, &facts(&[])).unwrap();

        assert_eq!(profile.effective_max_steps(), Some(10));
        assert!(!profile.max_steps.was_clamped());
    }

    /// "Unset" defers to the cap; it must not become an invented default that
    /// happens to exceed it.
    #[test]
    fn an_unset_bound_defers_to_the_operator_cap() {
        let (_temp, package) = package_with("", &[]);
        let constraints = OperatorConstraints {
            max_steps_cap: Some(7),
            ..OperatorConstraints::unconstrained()
        };

        let profile = build(&package, &constraints, &facts(&[])).unwrap();

        assert_eq!(profile.max_steps.requested, None);
        assert_eq!(profile.max_steps.effective, Some(7));
        assert!(!profile.max_steps.was_clamped());
    }

    #[test]
    fn a_bound_with_neither_request_nor_cap_stays_unbounded() {
        let bound = ResolvedBound::resolve(None, None);
        assert_eq!(bound.effective, None);
        assert!(!bound.was_clamped());
    }

    #[test]
    fn resolving_a_bound_never_exceeds_the_cap() {
        for requested in [0u32, 1, 49, 50, 51, u32::MAX] {
            for cap in [0u32, 1, 50] {
                let bound = ResolvedBound::resolve(Some(requested), Some(cap));
                assert!(
                    bound.effective.unwrap() <= cap,
                    "requested {requested} under cap {cap} resolved to {:?}",
                    bound.effective
                );
            }
        }
    }

    // --- capability resolution (§16.3) ---

    #[test]
    fn an_agent_allow_list_narrows_rather_than_extends() {
        let (_temp, package) = package_with(
            "[capability_policy]\nallow = [\"fs.read\", \"net.http\"]\n",
            &[],
        );
        let constraints = OperatorConstraints {
            allowed_capabilities: set(&["fs.read", "shell.exec"]),
            ..OperatorConstraints::unconstrained()
        };

        let profile = build(
            &package,
            &constraints,
            &facts(&["fs.read", "shell.exec", "net.http"]),
        )
        .unwrap();

        assert_eq!(
            profile.effective_capabilities,
            set(&["fs.read"]),
            "the agent may only intersect the operator set, never add to it"
        );
        assert!(!profile.allows_capability("net.http"));
        assert!(!profile.allows_capability("shell.exec"));
    }

    #[test]
    fn an_operator_deny_beats_every_agent_allow() {
        let (_temp, package) = package_with(
            "[capability_policy]\nallow = [\"fs.read\", \"shell.exec\"]\n",
            &[],
        );
        let constraints = OperatorConstraints {
            denied_capabilities: set(&["shell.exec"]),
            ..OperatorConstraints::unconstrained()
        };

        let profile = build(&package, &constraints, &facts(&["fs.read", "shell.exec"])).unwrap();

        assert!(!profile.allows_capability("shell.exec"));
        assert!(profile.allows_capability("fs.read"));
    }

    #[test]
    fn an_agent_deny_removes_a_capability_it_also_allows() {
        let (_temp, package) = package_with(
            "[capability_policy]\nallow = [\"fs.read\", \"fs.write\"]\ndeny = [\"fs.write\"]\n",
            &[],
        );
        let profile = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read", "fs.write"]),
        )
        .unwrap();

        assert_eq!(profile.effective_capabilities, set(&["fs.read"]));
    }

    /// A required capability the runtime cannot dispatch must refuse the build,
    /// not start a run that will fail on first use.
    #[test]
    fn a_required_capability_missing_from_the_resolved_set_refuses_the_build() {
        let (_temp, package) =
            package_with("[capability_policy]\nrequired = [\"shell.exec\"]\n", &[]);

        let error = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap_err();

        assert_eq!(error.code(), "required_capability_unavailable");
    }

    /// A denied requirement is reported distinctly from an absent one: they need
    /// different operator responses.
    #[test]
    fn a_required_capability_denied_by_the_agent_is_reported_as_denied() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("agents").join("ops");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("agent.toml"),
            format!(
                "{BASE_MANIFEST}[capability_policy]\nrequired = [\"shell.exec\"]\ndeny = [\"shell.exec\"]\n"
            ),
        )
        .unwrap();

        let error = load_agent_package(&root, &selector(), &OperatorConstraints::unconstrained())
            .unwrap_err();
        assert!(matches!(
            error,
            crate::agents::package::PackageLoadError::ValidationFailed { report, .. }
                if report.has_code("required_capability_denied")
        ));
    }

    #[test]
    fn a_capability_denied_only_by_the_operator_is_reported_as_denied_at_build_time() {
        let (_temp, package) =
            package_with("[capability_policy]\nrequired = [\"shell.exec\"]\n", &[]);
        let constraints = OperatorConstraints {
            denied_capabilities: set(&["shell.exec"]),
            ..OperatorConstraints::unconstrained()
        };

        // Validation flags this too; the build must independently refuse rather
        // than relying on a caller having checked the report.
        let error = build(&package, &constraints, &facts(&["shell.exec"])).unwrap_err();
        assert_eq!(error.code(), "required_capability_denied");
    }

    #[test]
    fn an_unavailable_optional_capability_is_recorded_but_not_fatal() {
        let (_temp, package) =
            package_with("[capability_policy]\noptional = [\"net.http\"]\n", &[]);

        let profile = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap();

        assert_eq!(
            profile.unavailable_optional_capabilities,
            set(&["net.http"])
        );
    }

    #[test]
    fn a_package_that_failed_validation_is_never_compiled() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("agents").join("ops");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("agent.toml"), BASE_MANIFEST).unwrap();
        let mut package =
            load_agent_package(&root, &selector(), &OperatorConstraints::unconstrained()).unwrap();

        // Simulate a report carrying an error, as a caller reusing a package
        // across a policy reload could produce.
        package
            .validation
            .push(crate::agents::validation::ValidationIssue::error(
                "synthetic",
                "field",
                "injected for this test",
            ));

        let error = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap_err();
        assert_eq!(error.code(), "package_blocked");
    }

    // --- runtime feature requirements (§8.6) ---

    #[test]
    fn a_required_feature_the_model_lacks_refuses_the_build() {
        let (_temp, package) = package_with(
            "[runtime_compatibility]\nmin_schema_version = 1\nnative_tool_use = \"required\"\n",
            &[],
        );
        let mut facts = facts(&[]);
        facts.native_tool_use = false;

        let error = build(&package, &OperatorConstraints::unconstrained(), &facts).unwrap_err();

        assert_eq!(error.code(), "required_feature_unsatisfied");
    }

    #[test]
    fn a_preferred_feature_the_model_lacks_records_degradation_and_continues() {
        let (_temp, package) = package_with(
            "[runtime_compatibility]\nmin_schema_version = 1\nnative_tool_use = \"preferred\"\n",
            &[],
        );
        let mut facts = facts(&[]);
        facts.native_tool_use = false;

        let profile = build(&package, &OperatorConstraints::unconstrained(), &facts).unwrap();

        assert_eq!(
            profile.feature_evaluations.get("native_tool_use"),
            Some(&FeatureEvaluation::Degraded)
        );
    }

    /// An unknown context window must not be optimistically treated as large
    /// enough: that is how a run silently truncates its own instructions.
    #[test]
    fn an_unknown_context_window_does_not_satisfy_a_hard_minimum() {
        let (_temp, package) = package_with(
            "[runtime_compatibility]\nmin_schema_version = 1\nmin_context_tokens = 200000\n",
            &[],
        );
        let mut facts = facts(&[]);
        facts.context_tokens = None;

        let error = build(&package, &OperatorConstraints::unconstrained(), &facts).unwrap_err();

        assert_eq!(error.code(), "context_window_too_small");
    }

    #[test]
    fn a_context_window_below_the_declared_minimum_refuses_the_build() {
        let (_temp, package) = package_with(
            "[runtime_compatibility]\nmin_schema_version = 1\nmin_context_tokens = 200000\n",
            &[],
        );
        let error =
            build(&package, &OperatorConstraints::unconstrained(), &facts(&[])).unwrap_err();
        assert_eq!(error.code(), "context_window_too_small");
    }

    #[test]
    fn a_missing_required_modality_refuses_the_build() {
        let (_temp, package) = package_with(
            "[runtime_compatibility]\nmin_schema_version = 1\nrequired_modalities = [\"image\"]\n",
            &[],
        );
        let error =
            build(&package, &OperatorConstraints::unconstrained(), &facts(&[])).unwrap_err();
        assert_eq!(error.code(), "required_modality_unavailable");
    }

    // --- pinned identity ---

    #[test]
    fn the_profile_hash_changes_when_an_operator_cap_changes() {
        let (_temp, package) = package_with("[execution_defaults]\nmax_steps = 500\n", &[]);
        let loose = build(
            &package,
            &OperatorConstraints {
                max_steps_cap: Some(400),
                ..OperatorConstraints::unconstrained()
            },
            &facts(&[]),
        )
        .unwrap();
        let tight = build(
            &package,
            &OperatorConstraints {
                max_steps_cap: Some(10),
                ..OperatorConstraints::unconstrained()
            },
            &facts(&[]),
        )
        .unwrap();

        assert_ne!(
            loose.profile_hash, tight.profile_hash,
            "a resume must not replay under bounds it was never granted"
        );
    }

    #[test]
    fn the_profile_hash_changes_when_the_capability_set_changes() {
        let (_temp, package) = package_with("", &[]);
        let wide = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read", "shell.exec"]),
        )
        .unwrap();
        let narrow = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap();

        assert_ne!(wide.profile_hash, narrow.profile_hash);
    }

    #[test]
    fn the_profile_hash_is_stable_across_identical_builds() {
        let (_temp, package) = package_with("[execution_defaults]\nmax_steps = 12\n", &[]);
        let first = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap();
        let second = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap();

        assert_eq!(first.profile_hash, second.profile_hash);
    }

    #[test]
    fn the_instruction_bundle_and_procedure_selection_are_pinned_by_hash() {
        let (_temp, package) = package_with("", &[]);
        let bundle = InstructionBundle::assemble([DiscoveredInstruction::new(
            "AGENTS.md",
            "Run the formatter before committing.\n",
        )]);

        let document = crate::agents::procedure::document::ProcedureDocument::parse(
            "---\nschema_version: 1\nid: build.fmt\nversion: 1.0.0\nstatus: active\ntitle: Format\nmode: diagnose\nrisk_level: low\nintents: [format]\n---\n\nRun it.\n",
            crate::agents::procedure::trust::ProcedureOrigin::WorkspacePackage,
            "procedures/fmt.md",
        )
        .unwrap();
        let catalog = ProcedureCatalog::build(
            [document],
            &EligibilityContext::new("ops", "windows", "2026-08-09"),
        );
        let selection = select_procedures(
            &catalog,
            &SelectionRequest {
                intents: set(&["format"]),
                ..SelectionRequest::default()
            },
            None,
        );
        assert_eq!(selection.selected.len(), 1, "fixture must select something");

        let profile = build_runtime_profile(ProfileInputs {
            package: &package,
            constraints: &OperatorConstraints::unconstrained(),
            facts: &facts(&[]),
            instructions: Some(&bundle),
            procedures: Some(&selection),
        })
        .unwrap();

        let components = profile.identity_components();
        assert!(
            components
                .iter()
                .any(|component| component.starts_with("instructions:")),
            "{components:?}"
        );
        assert!(
            components
                .iter()
                .any(|component| component.contains("procedure:build.fmt@1.0.0#sha256:")),
            "{components:?}"
        );

        // Editing the instruction text must change the pinned identity.
        let edited = InstructionBundle::assemble([DiscoveredInstruction::new(
            "AGENTS.md",
            "Run the formatter and the linter before committing.\n",
        )]);
        let other = build_runtime_profile(ProfileInputs {
            package: &package,
            constraints: &OperatorConstraints::unconstrained(),
            facts: &facts(&[]),
            instructions: Some(&edited),
            procedures: Some(&selection),
        })
        .unwrap();
        assert_ne!(profile.profile_hash, other.profile_hash);
    }

    #[test]
    fn a_profile_round_trips_through_serialization() {
        let (_temp, package) = package_with(
            "policy_instructions_path = \"policy.md\"\n[execution_defaults]\nmax_steps = 20\n",
            &[("policy.md", "Be careful.\n")],
        );
        let profile = build(
            &package,
            &OperatorConstraints::unconstrained(),
            &facts(&["fs.read"]),
        )
        .unwrap();

        let json = serde_json::to_string(&profile).unwrap();
        let restored: AgentRuntimeProfile = serde_json::from_str(&json).unwrap();

        assert_eq!(profile, restored);
        assert_eq!(restored.profile_hash, profile.profile_hash);
    }

    // --- legacy compatibility ---

    #[test]
    fn the_legacy_profile_grants_nothing_beyond_operator_policy() {
        let constraints = OperatorConstraints {
            allowed_capabilities: set(&["fs.read"]),
            ..OperatorConstraints::unconstrained()
        };
        let profile = legacy_profile(&constraints, &facts(&["fs.read", "shell.exec"]));

        assert!(profile.is_legacy());
        assert_eq!(profile.effective_capabilities, set(&["fs.read"]));
        assert!(profile.policy_instructions.is_none());
        assert!(profile.procedures.is_none());
        assert!(profile.instructions.is_none());
        assert_eq!(
            profile.max_procedure_selections.effective,
            Some(0),
            "a legacy run must not gain procedure selection implicitly"
        );
        assert!(!profile.validation.blocks_activation());
    }

    #[test]
    fn the_legacy_profile_respects_an_operator_deny() {
        let constraints = OperatorConstraints {
            denied_capabilities: set(&["shell.exec"]),
            ..OperatorConstraints::unconstrained()
        };
        let profile = legacy_profile(&constraints, &facts(&["fs.read", "shell.exec"]));

        assert!(!profile.allows_capability("shell.exec"));
    }

    #[test]
    fn the_legacy_profile_hash_is_stable() {
        let constraints = OperatorConstraints::unconstrained();
        let first = legacy_profile(&constraints, &facts(&["fs.read"]));
        let second = legacy_profile(&constraints, &facts(&["fs.read"]));
        assert_eq!(first.profile_hash, second.profile_hash);
    }
}
