//! Runtime-owned Agent activation and per-run context compilation.
//!
//! This is the bridge between the filesystem-neutral definition types and the
//! shared Engine. It resolves one explicit selector, snapshots trusted
//! workspace instructions, filters and selects procedures, and produces the
//! immutable profile used by a run. A same-run resume consumes the saved
//! snapshot after structural and current-policy validation; it never reloads a
//! newer package under the old identity.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use rove_models::Message;
use serde::{Deserialize, Serialize};

use super::instructions::{InstructionBundle, InstructionDiscoveryError};
use super::package::{AgentPackage, PackageLoadError, load_agent_package};
use super::procedure::catalog::{EligibilityContext, ProcedureCatalog};
use super::procedure::document::{DocumentError, ProcedureDocument, ProcedureReference};
use super::procedure::hydration::{HydratedProcedure, hydrate};
use super::procedure::metadata::ProcedureMode;
use super::procedure::selection::{ProcedureSelection, SelectionRequest, select_procedures};
use super::procedure::trust::{ProcedureOrigin, ProcedureTrust};
use super::profile::{
    AGENT_PROFILE_SCHEMA_VERSION, AgentRuntimeProfile, ProfileBuildError, ProfileInputs,
    ResolvedRuntimeFacts, attach_hydrated_procedures, attach_instruction_bundle,
    build_runtime_profile, legacy_profile,
};
use super::selector::{AgentSelector, AgentSource};
use super::validation::OperatorConstraints;
use crate::capability::CapabilitySnapshot;
use crate::context::prompt_metadata::stable_hash;
use crate::execution::{ProcedureApplication, ProcedureCapabilityBinding};
use crate::workspace::boundary::resolve_workspace_read_path_without_links;
use crate::workspace::{Workspace, WorkspaceKind};

/// Maximum bytes of Agent-owned material injected into one prompt prefix.
/// The full source snapshot may be larger, but prompt assembly is a separate,
/// tighter budget shared with history, memory, and the current task.
pub const MAX_AGENT_PROMPT_CONTEXT_BYTES: usize = 64 * 1024;
/// Dynamic nested-instruction budget. The source bundle is already bounded to
/// 128 KiB; this headroom covers scope banners without truncating an admitted
/// rule before a path-bound action is reconsidered.
pub const MAX_SCOPED_INSTRUCTION_CONTEXT_BYTES: usize =
    super::instructions::MAX_BUNDLE_BYTES + (32 * 1024);
/// Maximum workspace procedure roots admitted from one package.
pub const MAX_WORKSPACE_PROCEDURE_ROOTS: usize = 8;
/// Maximum additional procedure files loaded outside the package directory.
pub const MAX_WORKSPACE_PROCEDURES: usize = 128;
/// Maximum filesystem entries inspected while loading configured procedure
/// roots. A root containing mostly irrelevant or malformed files must not turn
/// activation into an unbounded walk.
pub const MAX_WORKSPACE_PROCEDURE_ENTRIES: usize = 4_096;
/// Maximum diagnostics retained by activation.
pub const MAX_AGENT_DIAGNOSTICS: usize = 64;

/// Operator-owned activation settings. No field is sourced from an Agent
/// package, so a package cannot authorize its own loading or widen a bound.
#[derive(Debug, Clone)]
pub struct AgentActivationConfig {
    pub selector: AgentSelector,
    pub workspace_source_authorized: bool,
    pub load_workspace_instructions: bool,
    pub allow_remediation_procedures: bool,
    pub constraints: OperatorConstraints,
    pub context_tokens: Option<u32>,
}

impl Default for AgentActivationConfig {
    fn default() -> Self {
        Self {
            selector: AgentSelector::legacy(),
            workspace_source_authorized: false,
            load_workspace_instructions: false,
            allow_remediation_procedures: false,
            constraints: OperatorConstraints::unconstrained(),
            context_tokens: None,
        }
    }
}

/// Stable, content-free identity safe for events, reports, and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl AgentProfileIdentity {
    pub fn from_profile(profile: &AgentRuntimeProfile) -> Self {
        Self {
            selector: profile.selector.clone(),
            agent_id: profile.agent_id.clone(),
            display_name: profile.display_name.clone(),
            definition_version: profile.definition_version.clone(),
            manifest_hash: profile.manifest_hash.clone(),
            package_hash: profile.package_hash.clone(),
            profile_hash: profile.profile_hash.clone(),
            instruction_bundle_hash: profile
                .instructions
                .as_ref()
                .map(InstructionBundle::bundle_hash),
            procedures: profile
                .procedures
                .as_ref()
                .map(|selection| {
                    selection
                        .selected
                        .iter()
                        .map(|selected| selected.reference.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// One bounded diagnostic produced while resolving an Agent context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDiagnostic {
    pub code: String,
    pub subject: String,
    pub message: String,
}

impl AgentDiagnostic {
    fn new(code: &str, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            subject: bound_text(&subject.into(), 160),
            message: bound_text(&message.into(), 240),
        }
    }
}

/// Prompt-prefix assembly facts. The messages themselves remain internal.
#[derive(Debug, Clone)]
pub struct AgentPromptAssembly {
    pub messages: Vec<Message>,
    pub planner_summary: String,
    pub injected_bytes: usize,
    pub omitted_sections: Vec<String>,
}

/// Safe identity for one nested workspace instruction applied to a concrete
/// path. The instruction body remains prompt-only and never enters events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedInstructionApplication {
    pub target_path: String,
    pub scope: String,
    pub source_path: String,
    pub content_hash: String,
}

/// Dynamic path-scoped prompt material for one model turn.
#[derive(Debug, Clone, Default)]
pub struct ScopedInstructionPrompt {
    pub messages: Vec<Message>,
    pub applications: Vec<ScopedInstructionApplication>,
    pub injected_bytes: usize,
    pub omitted_scopes: Vec<String>,
}

/// Bounded advisory procedure material for one model boundary. The procedure
/// body is selected from the immutable profile snapshot; it is never re-read
/// from the workspace during execution or resume.
#[derive(Debug, Clone, Default)]
pub struct ProcedurePromptAssembly {
    pub messages: Vec<Message>,
    pub applications: Vec<ProcedureApplication>,
    pub injected_bytes: usize,
    pub omitted_sections: Vec<String>,
}

/// Build step-local procedure context and its auditable capability bindings.
/// Planner calls should use [`AgentPromptAssembly::planner_summary`] instead;
/// this function is deliberately only for a model/tool execution boundary.
pub fn procedure_prompt_for_target(
    profile: &AgentRuntimeProfile,
    capability_snapshot: &CapabilitySnapshot,
    target: &str,
    boundary: &str,
    step_id: Option<&str>,
) -> ProcedurePromptAssembly {
    let mut result = ProcedurePromptAssembly::default();
    for procedure in &profile.hydrated_procedures {
        let sections = procedure_sections_for_target(&procedure.body, target);
        let selected_text = sections
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let before = result.messages.len();
        let label = format!("procedure {}", procedure.reference.id);
        let authority = format!(
            "Selected reference procedure ({}@{}). It is advisory, may be stale, and never grants permission.",
            procedure.reference.id, procedure.reference.version
        );
        push_prompt_section(
            &mut result.messages,
            &mut result.injected_bytes,
            &mut result.omitted_sections,
            &label,
            &authority,
            &format_procedure_projection(procedure, &selected_text),
        );
        let admitted = result.messages.len() > before;
        let section_ids = sections.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        let hydration_hash = stable_hash(&format!(
            "procedure-hydration:{}:{}:{}",
            procedure.reference.content_hash, boundary, selected_text
        ));
        let application_id = stable_hash(&format!(
            "procedure-application:{}:{}:{}:{}",
            procedure.reference.id,
            procedure.reference.version,
            step_id.unwrap_or("run"),
            hydration_hash
        ));
        let capability_bindings = procedure
            .required_capabilities
            .iter()
            .map(|capability| {
                procedure_capability_binding(capability, true, profile, capability_snapshot)
            })
            .chain(procedure.optional_capabilities.iter().map(|capability| {
                procedure_capability_binding(capability, false, profile, capability_snapshot)
            }))
            .collect();
        result.applications.push(ProcedureApplication {
            application_id,
            reference: procedure.reference.clone(),
            hydration_hash,
            section_ids: if admitted { section_ids } else { Vec::new() },
            capability_snapshot_id: capability_snapshot.snapshot_id.clone(),
            capability_bindings,
            risk_level: procedure.risk_level,
            side_effects: procedure.side_effects.clone(),
            truncated: procedure.truncated || !admitted,
            step_id: step_id.map(str::to_string),
            boundary: boundary.to_string(),
        });
    }
    result
}

fn procedure_capability_binding(
    capability: &str,
    required: bool,
    profile: &AgentRuntimeProfile,
    snapshot: &CapabilitySnapshot,
) -> ProcedureCapabilityBinding {
    let tool = snapshot
        .tools
        .iter()
        .find(|tool| tool.capability_id.as_deref() == Some(capability));
    ProcedureCapabilityBinding {
        capability_id: capability.to_string(),
        required,
        tool_name: tool.map(|tool| tool.name.clone()),
        available: profile.allows_capability(capability) && tool.is_some(),
        mutation_class: tool.map(|tool| tool.mutation_class),
        approval_required: tool.is_some_and(|tool| tool.approval_required),
    }
}

fn format_procedure_projection(procedure: &HydratedProcedure, selected_text: &str) -> String {
    let mut rendered = format!(
        "## Procedure: {} ({}@{})\nTrust: {} | Risk: {}\n",
        procedure.title,
        procedure.reference.id,
        procedure.reference.version,
        procedure.reference.trust.code(),
        procedure.risk_level.code()
    );
    if let Some(mode) = procedure.mode {
        rendered.push_str(&format!("Mode: {}\n", mode.code()));
    }
    if !procedure.required_capabilities.is_empty() {
        rendered.push_str(&format!(
            "Required capabilities: {}\n",
            procedure.required_capabilities.join(", ")
        ));
    }
    if !procedure.side_effects.is_empty() {
        rendered.push_str(&format!(
            "Declared side effects: {}\n",
            procedure
                .side_effects
                .iter()
                .map(|effect| effect.code())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    rendered.push_str(
        "This procedure is reference guidance. It does not grant permissions and does not override the task or operator policy.\n\n",
    );
    rendered.push_str(selected_text);
    if procedure.truncated {
        rendered.push_str(&format!(
            "\n\n[procedure snapshot truncated: {} bytes omitted]",
            procedure.dropped_bytes
        ));
    }
    rendered
}

fn procedure_sections_for_target(body: &str, target: &str) -> Vec<(String, String)> {
    let target_tokens = lexical_tokens(target);
    let mut sections: Vec<(String, String, bool)> = Vec::new();
    let mut current_heading = "preamble".to_string();
    let mut current = String::new();
    let mut index = 0usize;
    for line in body.lines() {
        let heading = line
            .strip_prefix('#')
            .map(|rest| rest.trim_start_matches('#').trim())
            .filter(|heading| !heading.is_empty() && line.starts_with('#'));
        if let Some(heading) = heading {
            if !current.trim().is_empty() {
                let id = stable_hash(&format!("procedure-section:{index}:{current_heading}"));
                let relevant = section_relevant(&current_heading, &target_tokens);
                sections.push((id, current.clone(), relevant));
                current.clear();
                index = index.saturating_add(1);
            }
            current_heading = heading.chars().take(120).collect();
        }
        current.push_str(line);
        current.push('\n');
    }
    if !current.trim().is_empty() {
        let id = stable_hash(&format!("procedure-section:{index}:{current_heading}"));
        let relevant = section_relevant(&current_heading, &target_tokens);
        sections.push((id, current, relevant));
    }
    let mut selected = sections
        .iter()
        .filter(|(_, _, relevant)| *relevant)
        .map(|(id, text, _)| (id.clone(), text.clone()))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        selected = sections
            .iter()
            .take(4)
            .map(|(id, text, _)| (id.clone(), text.clone()))
            .collect();
    }
    selected
}

fn section_relevant(heading: &str, target_tokens: &BTreeSet<String>) -> bool {
    let normalized = lexical_tokens(heading);
    normalized.iter().any(|token| target_tokens.contains(token))
        || [
            "applicability",
            "precondition",
            "safety",
            "evidence",
            "validation",
            "verification",
            "rollback",
            "stop",
            "escalation",
            "limitation",
        ]
        .iter()
        .any(|anchor| normalized.contains(*anchor))
}

/// Fully resolved per-run Agent context.
#[derive(Debug, Clone)]
pub struct ResolvedAgentRun {
    pub profile: AgentRuntimeProfile,
    pub identity: AgentProfileIdentity,
    pub prompt: AgentPromptAssembly,
    pub diagnostics: Vec<AgentDiagnostic>,
    pub resumed_from_snapshot: bool,
}

/// Failures that refuse activation before any model or tool work begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "error")]
pub enum AgentActivationError {
    #[error("workspace Agent sources are not authorized by Project Trust")]
    WorkspaceSourceNotAuthorized,
    #[error("builtin Agent '{agent_id}' is not implemented")]
    UnsupportedBuiltin { agent_id: String },
    #[error(transparent)]
    Package(#[from] PackageLoadError),
    #[error(transparent)]
    Instructions(#[from] InstructionDiscoveryError),
    #[error(transparent)]
    Profile(#[from] ProfileBuildError),
    #[error("saved Agent profile schema {found} is not supported (expected {expected})")]
    UnsupportedSavedProfile { found: u16, expected: u16 },
    #[error("saved Agent profile capability '{capability}' is no longer available")]
    SavedCapabilityUnavailable { capability: String },
    #[error("saved Agent profile capability '{capability}' is denied by current policy")]
    SavedCapabilityDenied { capability: String },
    #[error("saved Agent profile bound '{field}' exceeds the current cap")]
    SavedBoundExceedsCurrentCap { field: String },
}

impl AgentActivationError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::WorkspaceSourceNotAuthorized => "workspace_source_not_authorized",
            Self::UnsupportedBuiltin { .. } => "unsupported_builtin_agent",
            Self::Package(error) => error.code(),
            Self::Instructions(_) => "instruction_discovery_failed",
            Self::Profile(error) => error.code(),
            Self::UnsupportedSavedProfile { .. } => "unsupported_saved_profile",
            Self::SavedCapabilityUnavailable { .. } => "saved_capability_unavailable",
            Self::SavedCapabilityDenied { .. } => "saved_capability_denied",
            Self::SavedBoundExceedsCurrentCap { .. } => "saved_bound_exceeds_current_cap",
        }
    }
}

/// Engine-owned activation source. Loading happens once; lexical selection and
/// profile compilation happen for each new run.
#[derive(Debug, Clone)]
pub struct AgentRuntime {
    workspace: Workspace,
    config: AgentActivationConfig,
    facts: ResolvedRuntimeFacts,
    package: Option<AgentPackage>,
    instructions: Option<InstructionBundle>,
    extra_procedures: Vec<ProcedureDocument>,
    load_diagnostics: Vec<AgentDiagnostic>,
}

impl AgentRuntime {
    pub fn load(
        workspace: &Workspace,
        config: AgentActivationConfig,
        facts: ResolvedRuntimeFacts,
    ) -> Result<Self, AgentActivationError> {
        let workspace_content_requested =
            config.load_workspace_instructions || config.selector.source == AgentSource::Workspace;
        if workspace_content_requested && !config.workspace_source_authorized {
            return Err(AgentActivationError::WorkspaceSourceNotAuthorized);
        }

        let instructions = if config.load_workspace_instructions {
            Some(InstructionBundle::discover(&workspace.root)?)
        } else {
            None
        };

        let package = match config.selector.source {
            AgentSource::Builtin if config.selector.is_legacy() => None,
            AgentSource::Builtin => {
                return Err(AgentActivationError::UnsupportedBuiltin {
                    agent_id: config.selector.agent_id.clone(),
                });
            }
            AgentSource::Workspace => {
                let package_relative = format!("agents/{}", config.selector.agent_id);
                let package_root =
                    resolve_workspace_read_path_without_links(&workspace.root, &package_relative)
                        .map_err(|error| PackageLoadError::UnresolvableRoot {
                        path: package_relative,
                        reason: error.to_string(),
                    })?;
                Some(load_agent_package(
                    &package_root,
                    &config.selector,
                    &config.constraints,
                )?)
            }
        };

        let mut load_diagnostics = Vec::new();
        let extra_procedures = if let Some(package) = package.as_ref() {
            load_workspace_procedures(workspace, package, &mut load_diagnostics)
        } else {
            Vec::new()
        };

        let runtime = Self {
            workspace: workspace.clone(),
            config,
            facts,
            package,
            instructions,
            extra_procedures,
            load_diagnostics,
        };
        // Resolve a no-query profile now so a selected package with an
        // incompatible runtime, denied requirement, or invalid immutable
        // snapshot fails Engine assembly. Goal-dependent procedure ranking is
        // still repeated per run.
        runtime.fresh_profile("")?;
        Ok(runtime)
    }

    pub fn selector(&self) -> &AgentSelector {
        &self.config.selector
    }

    pub fn source_profile_identity(&self) -> AgentProfileIdentity {
        let profile = self
            .fresh_profile("")
            .expect("AgentRuntime source profile was validated during activation");
        AgentProfileIdentity::from_profile(&profile)
    }

    /// Resolve a fresh profile, or validate and reuse an exact same-run
    /// snapshot. The caller decides whether a saved task is a same-run resume.
    pub fn resolve_for_run(
        &self,
        user_message: &str,
        pinned: Option<&AgentRuntimeProfile>,
    ) -> Result<ResolvedAgentRun, AgentActivationError> {
        let (profile, resumed_from_snapshot) = match pinned {
            Some(profile) => {
                self.validate_pinned_profile(profile)?;
                (profile.clone(), true)
            }
            None => (self.fresh_profile(user_message)?, false),
        };
        let identity = AgentProfileIdentity::from_profile(&profile);
        let prompt = assemble_prompt(&profile);
        let mut diagnostics = self.load_diagnostics.clone();
        collect_profile_diagnostics(&profile, &mut diagnostics);
        diagnostics.truncate(MAX_AGENT_DIAGNOSTICS);

        Ok(ResolvedAgentRun {
            profile,
            identity,
            prompt,
            diagnostics,
            resumed_from_snapshot,
        })
    }

    /// Resolve against the capability catalog pinned for this run.
    ///
    /// Model/runtime compatibility facts remain those validated at Engine
    /// assembly; only tool capabilities can change through an atomic registry
    /// refresh. Cloning here keeps the loaded package and instruction sources
    /// immutable while allowing a new run to see a newly published catalog.
    pub fn resolve_for_run_with_capabilities(
        &self,
        user_message: &str,
        pinned: Option<&AgentRuntimeProfile>,
        available_capabilities: BTreeSet<String>,
    ) -> Result<ResolvedAgentRun, AgentActivationError> {
        let mut runtime = self.clone();
        runtime.facts.available_capabilities = available_capabilities;
        runtime.resolve_for_run(user_message, pinned)
    }

    fn fresh_profile(
        &self,
        user_message: &str,
    ) -> Result<AgentRuntimeProfile, AgentActivationError> {
        let Some(package) = self.package.as_ref() else {
            return Ok(attach_instruction_bundle(
                legacy_profile(&self.config.constraints, &self.facts),
                self.instructions
                    .clone()
                    .filter(|bundle| !bundle.is_empty()),
            ));
        };

        let mut documents = package.procedures.clone();
        documents.extend(self.extra_procedures.clone());
        let mut eligibility = EligibilityContext::new(
            &package.definition.id,
            std::env::consts::OS,
            chrono::Utc::now()
                .date_naive()
                .format("%Y-%m-%d")
                .to_string(),
        )
        .with_capabilities(self.facts.available_capabilities.iter().cloned())
        .with_workspace_kinds(workspace_kinds(&self.workspace));
        eligibility.allowed_trust_levels = package
            .definition
            .procedure_policy
            .allowed_trust_levels
            .iter()
            .filter_map(|value| ProcedureTrust::parse(value))
            .collect();
        eligibility.required_tags = package.definition.procedure_policy.required_tags.clone();
        if self.config.allow_remediation_procedures {
            eligibility.allowed_modes.insert(ProcedureMode::Remediate);
        }

        let catalog = ProcedureCatalog::build(documents, &eligibility);
        let lexical = lexical_tokens(user_message);
        let selection = select_procedures(
            &catalog,
            &SelectionRequest {
                intents: lexical.clone(),
                tags: lexical,
                scope: lexical_scope(user_message),
                max_selected: Some(
                    usize::try_from(package.definition.procedure_policy.max_selected)
                        .unwrap_or(usize::MAX),
                ),
                max_risk_level: None,
            },
            self.config
                .constraints
                .max_procedure_selections_cap
                .and_then(|cap| usize::try_from(cap).ok()),
        );
        let hydrated = hydrate_selection(&selection, &catalog)?;
        let profile = build_runtime_profile(ProfileInputs {
            package,
            constraints: &self.config.constraints,
            facts: &self.facts,
            instructions: self
                .instructions
                .as_ref()
                .filter(|bundle| !bundle.is_empty()),
            procedures: Some(&selection),
        })?;
        attach_hydrated_procedures(profile, hydrated).map_err(Into::into)
    }

    fn validate_pinned_profile(
        &self,
        profile: &AgentRuntimeProfile,
    ) -> Result<(), AgentActivationError> {
        if profile.schema_version != AGENT_PROFILE_SCHEMA_VERSION {
            return Err(AgentActivationError::UnsupportedSavedProfile {
                found: profile.schema_version,
                expected: AGENT_PROFILE_SCHEMA_VERSION,
            });
        }
        if (profile.selector.source == AgentSource::Workspace || profile.instructions.is_some())
            && !self.config.workspace_source_authorized
        {
            return Err(AgentActivationError::WorkspaceSourceNotAuthorized);
        }
        profile.validate_snapshot()?;

        for capability in &profile.effective_capabilities {
            if !self.facts.available_capabilities.contains(capability) {
                return Err(AgentActivationError::SavedCapabilityUnavailable {
                    capability: capability.clone(),
                });
            }
            if self
                .config
                .constraints
                .denied_capabilities
                .contains(capability)
                || (!self.config.constraints.allowed_capabilities.is_empty()
                    && !self
                        .config
                        .constraints
                        .allowed_capabilities
                        .contains(capability))
            {
                return Err(AgentActivationError::SavedCapabilityDenied {
                    capability: capability.clone(),
                });
            }
        }
        validate_saved_bound(
            "max_steps",
            profile.max_steps.effective,
            self.config.constraints.max_steps_cap,
        )?;
        validate_saved_bound(
            "max_tool_calls",
            profile.max_tool_calls.effective,
            self.config.constraints.max_tool_calls_cap,
        )?;
        validate_saved_bound(
            "max_procedure_selections",
            profile.max_procedure_selections.effective,
            self.config.constraints.max_procedure_selections_cap,
        )?;
        Ok(())
    }
}

fn hydrate_selection(
    selection: &ProcedureSelection,
    catalog: &ProcedureCatalog,
) -> Result<Vec<HydratedProcedure>, AgentActivationError> {
    selection
        .selected
        .iter()
        .map(|selected| {
            hydrate(selected, catalog).map_err(|error| {
                ProfileBuildError::HydratedProcedureIdentityMismatch {
                    id: format!("{} ({})", selected.reference.id, error.code()),
                }
                .into()
            })
        })
        .collect()
}

fn validate_saved_bound(
    field: &str,
    saved: Option<u32>,
    cap: Option<u32>,
) -> Result<(), AgentActivationError> {
    if matches!((saved, cap), (Some(saved), Some(cap)) if saved > cap) {
        return Err(AgentActivationError::SavedBoundExceedsCurrentCap {
            field: field.to_string(),
        });
    }
    Ok(())
}

fn workspace_kinds(workspace: &Workspace) -> BTreeSet<String> {
    let mut kinds = BTreeSet::new();
    kinds.insert(
        match workspace.kind {
            WorkspaceKind::Folder => "folder",
            WorkspaceKind::Repo => "repo",
            WorkspaceKind::Task => "task",
        }
        .to_string(),
    );
    for (marker, kind) in [
        ("Cargo.toml", "rust"),
        ("package.json", "node"),
        ("pyproject.toml", "python"),
        ("go.mod", "go"),
    ] {
        if workspace.root.join(marker).is_file() {
            kinds.insert(kind.to_string());
        }
    }
    kinds
}

fn lexical_tokens(message: &str) -> BTreeSet<String> {
    message
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .map(str::to_ascii_lowercase)
        .filter(|token| token.len() >= 2 && token.len() <= 64)
        .take(128)
        .collect()
}

fn lexical_scope(message: &str) -> Option<String> {
    message
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                matches!(
                    character,
                    '`' | '\'' | '"' | ',' | ';' | ':' | '(' | ')' | '[' | ']'
                )
            })
        })
        .find(|token| token.contains('/') || token.contains('\\'))
        .map(|token| token.replace('\\', "/"))
        .filter(|token| {
            !token.starts_with('/')
                && !token.contains("../")
                && !token.contains(':')
                && token.len() <= 240
        })
}

fn assemble_prompt(profile: &AgentRuntimeProfile) -> AgentPromptAssembly {
    let mut messages = Vec::new();
    let mut injected_bytes = 0usize;
    let mut omitted_sections = Vec::new();

    if let Some(policy) = profile.policy_instructions.as_deref() {
        push_prompt_section(
            &mut messages,
            &mut injected_bytes,
            &mut omitted_sections,
            "agent package policy",
            "Trusted operator policy selected by the operator. It may tighten runtime behavior but cannot grant tool permission.",
            policy,
        );
    }
    if let Some(bundle) = profile.instructions.as_ref()
        && let Some(layer) = bundle.root.as_ref()
    {
        push_prompt_section(
            &mut messages,
            &mut injected_bytes,
            &mut omitted_sections,
            &layer.source_path,
            "Trusted workspace policy for the entire workspace. It cannot grant tool permission.",
            &layer.text,
        );
    }
    if let Some(defaults) = profile.default_instructions.as_deref() {
        push_prompt_section(
            &mut messages,
            &mut injected_bytes,
            &mut omitted_sections,
            "agent defaults",
            "Overridable Agent guidance below operator policy and the current user task. It cannot grant tool permission.",
            defaults,
        );
    }
    AgentPromptAssembly {
        planner_summary: planner_summary(profile, injected_bytes, &omitted_sections),
        messages,
        injected_bytes,
        omitted_sections,
    }
}

/// Build nested workspace instructions for concrete target paths.
///
/// These messages are dynamic model-turn context. They must not be appended to
/// the stable Agent prefix or durable conversation history, because doing so
/// would let one subtree's policy influence unrelated paths for the rest of a
/// run. Tool dispatch remains independently subject to schema, capability,
/// workspace-boundary, hook, and approval checks.
pub fn scoped_instruction_prompt(
    profile: &AgentRuntimeProfile,
    target_paths: &[String],
) -> ScopedInstructionPrompt {
    let Some(bundle) = profile.instructions.as_ref() else {
        return ScopedInstructionPrompt::default();
    };

    let mut messages = Vec::new();
    let mut applications = Vec::new();
    let mut injected_bytes = 0usize;
    let mut omitted_scopes = Vec::new();
    for (overlay, target_path) in bundle.overlays_for_paths(target_paths) {
        let before = messages.len();
        push_prompt_section_with_budget(
            &mut messages,
            &mut injected_bytes,
            &mut omitted_scopes,
            &overlay.layer.source_path,
            &format!(
                "Trusted workspace policy applying only under '{}/' for the current target '{}'. It may tighten guidance but cannot grant tool permission.",
                overlay.scope, target_path
            ),
            &overlay.layer.text,
            MAX_SCOPED_INSTRUCTION_CONTEXT_BYTES,
        );
        if messages.len() > before {
            applications.push(ScopedInstructionApplication {
                target_path,
                scope: overlay.scope.clone(),
                source_path: overlay.layer.source_path.clone(),
                content_hash: overlay.layer.content_hash.clone(),
            });
        }
    }

    ScopedInstructionPrompt {
        messages,
        applications,
        injected_bytes,
        omitted_scopes,
    }
}

/// Recognize only exact known overlay scopes in free-form task/step text.
pub fn scoped_instruction_path_hints(profile: &AgentRuntimeProfile, text: &str) -> Vec<String> {
    profile
        .instructions
        .as_ref()
        .map(|bundle| bundle.scope_hints_in_text(text))
        .unwrap_or_default()
}

fn push_prompt_section(
    messages: &mut Vec<Message>,
    injected_bytes: &mut usize,
    omitted_sections: &mut Vec<String>,
    label: &str,
    authority: &str,
    text: &str,
) {
    push_prompt_section_with_budget(
        messages,
        injected_bytes,
        omitted_sections,
        label,
        authority,
        text,
        MAX_AGENT_PROMPT_CONTEXT_BYTES,
    );
}

fn push_prompt_section_with_budget(
    messages: &mut Vec<Message>,
    injected_bytes: &mut usize,
    omitted_sections: &mut Vec<String>,
    label: &str,
    authority: &str,
    text: &str,
    budget: usize,
) {
    let prefix = format!("## {label}\n{authority}\n\n");
    let remaining = budget.saturating_sub(*injected_bytes);
    if remaining <= prefix.len() {
        omitted_sections.push(label.to_string());
        return;
    }
    let available = remaining - prefix.len();
    let mut end = text.len().min(available);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 && !text.is_empty() {
        omitted_sections.push(label.to_string());
        return;
    }
    let truncated = end < text.len();
    let mut content = prefix;
    content.push_str(&text[..end]);
    if truncated {
        content.push_str("\n\n[section truncated by Agent prompt budget]");
        omitted_sections.push(format!("{label}:truncated"));
    }
    *injected_bytes = injected_bytes.saturating_add(content.len());
    messages.push(Message::system(content));
}

fn planner_summary(
    profile: &AgentRuntimeProfile,
    injected_bytes: usize,
    omitted_sections: &[String],
) -> String {
    #[derive(Serialize)]
    struct Summary<'a> {
        selector: String,
        profile_hash: &'a str,
        instruction_bundle_hash: Option<String>,
        procedure_ids: Vec<&'a str>,
        procedures: Vec<ProcedurePlannerSummary>,
        workspace_instruction_scopes: Vec<&'a str>,
        effective_capabilities: &'a BTreeSet<String>,
        injected_bytes: usize,
        omitted_sections: &'a [String],
    }

    #[derive(Serialize)]
    struct ProcedurePlannerSummary {
        id: String,
        version: String,
        title: String,
        summary: String,
        mode: Option<ProcedureMode>,
        risk_level: String,
        required_capabilities: Vec<String>,
        optional_capabilities: Vec<String>,
        section_ids: Vec<String>,
        content_hash: String,
    }

    let procedures = profile
        .hydrated_procedures
        .iter()
        .map(|procedure| ProcedurePlannerSummary {
            id: procedure.reference.id.clone(),
            version: procedure.reference.version.clone(),
            title: procedure.title.clone(),
            summary: procedure.summary.chars().take(500).collect(),
            mode: procedure.mode,
            risk_level: procedure.risk_level.code().to_string(),
            required_capabilities: procedure.required_capabilities.clone(),
            optional_capabilities: procedure.optional_capabilities.clone(),
            section_ids: procedure_sections_for_target(&procedure.body, "")
                .into_iter()
                .map(|(id, _)| id)
                .take(24)
                .collect(),
            content_hash: procedure.reference.content_hash.clone(),
        })
        .collect();

    serde_json::to_string(&Summary {
        selector: profile.selector.to_string(),
        profile_hash: &profile.profile_hash,
        instruction_bundle_hash: profile
            .instructions
            .as_ref()
            .map(InstructionBundle::bundle_hash),
        procedure_ids: profile
            .procedures
            .as_ref()
            .map(ProcedureSelection::selected_ids)
            .unwrap_or_default(),
        procedures,
        workspace_instruction_scopes: profile
            .instructions
            .as_ref()
            .map(|bundle| {
                bundle
                    .overlays
                    .iter()
                    .map(|overlay| overlay.scope.as_str())
                    .collect()
            })
            .unwrap_or_default(),
        effective_capabilities: &profile.effective_capabilities,
        injected_bytes,
        omitted_sections,
    })
    .unwrap_or_else(|_| "{\"procedure_ids\":[]}".to_string())
}

fn collect_profile_diagnostics(
    profile: &AgentRuntimeProfile,
    diagnostics: &mut Vec<AgentDiagnostic>,
) {
    for issue in &profile.validation.issues {
        diagnostics.push(AgentDiagnostic::new(
            &issue.code,
            &issue.field,
            &issue.message,
        ));
    }
    if let Some(bundle) = profile.instructions.as_ref() {
        for rejection in &bundle.rejected {
            diagnostics.push(AgentDiagnostic::new(
                &rejection.code,
                &rejection.source_path,
                &rejection.message,
            ));
        }
    }
    for bound in profile.clamped_bounds() {
        diagnostics.push(AgentDiagnostic::new(
            "agent_bound_clamped",
            bound,
            "Agent-requested bound was reduced by current operator policy",
        ));
    }
    if let Some(selection) = profile.procedures.as_ref() {
        if selection.selected.is_empty() {
            diagnostics.push(AgentDiagnostic::new(
                "procedure_selection_no_match",
                "procedures",
                "No eligible procedure matched the current goal; the runtime continued without procedure guidance.",
            ));
        }
        if !selection.risk_excluded.is_empty() {
            diagnostics.push(AgentDiagnostic::new(
                "procedure_risk_excluded",
                "procedures",
                format!(
                    "{} eligible procedure(s) exceeded the accepted risk bound.",
                    selection.risk_excluded.len()
                ),
            ));
        }
        if !selection.conflict_excluded.is_empty() {
            diagnostics.push(AgentDiagnostic::new(
                "procedure_conflict_excluded",
                "procedures",
                format!(
                    "{} procedure(s) were excluded because they conflicted with a selected procedure.",
                    selection.conflict_excluded.len()
                ),
            ));
        }
    }
    for procedure in &profile.hydrated_procedures {
        if procedure.truncated {
            diagnostics.push(AgentDiagnostic::new(
                "procedure_snapshot_truncated",
                &procedure.reference.id,
                format!(
                    "procedure snapshot is bounded; {} bytes were omitted",
                    procedure.dropped_bytes
                ),
            ));
        }
        for capability in &procedure.optional_capabilities {
            if !profile.effective_capabilities.contains(capability) {
                diagnostics.push(AgentDiagnostic::new(
                    "procedure_optional_capability_unavailable",
                    format!("{}:{capability}", procedure.reference.id),
                    "optional procedure capability is unavailable in this run",
                ));
            }
        }
    }
}

fn load_workspace_procedures(
    workspace: &Workspace,
    package: &AgentPackage,
    diagnostics: &mut Vec<AgentDiagnostic>,
) -> Vec<ProcedureDocument> {
    let mut documents = Vec::new();
    for root in package
        .definition
        .procedure_policy
        .roots
        .iter()
        .take(MAX_WORKSPACE_PROCEDURE_ROOTS)
    {
        let raw = root.to_string_lossy().replace('\\', "/");
        let resolved = match resolve_workspace_read_path_without_links(&workspace.root, &raw) {
            Ok(path) if path.is_dir() => path,
            Ok(_) => {
                diagnostics.push(AgentDiagnostic::new(
                    "procedure_root_not_directory",
                    raw,
                    "configured procedure root is not a directory",
                ));
                continue;
            }
            Err(error) => {
                diagnostics.push(AgentDiagnostic::new(
                    "procedure_root_unavailable",
                    raw,
                    error.to_string(),
                ));
                continue;
            }
        };
        load_procedure_directory(workspace, &resolved, &mut documents, diagnostics);
        if documents.len() >= MAX_WORKSPACE_PROCEDURES {
            break;
        }
    }
    if package.definition.procedure_policy.roots.len() > MAX_WORKSPACE_PROCEDURE_ROOTS {
        diagnostics.push(AgentDiagnostic::new(
            "too_many_procedure_roots",
            "procedure_policy.roots",
            format!("at most {MAX_WORKSPACE_PROCEDURE_ROOTS} roots are loaded"),
        ));
    }
    documents
}

fn load_procedure_directory(
    workspace: &Workspace,
    root: &Path,
    documents: &mut Vec<ProcedureDocument>,
    diagnostics: &mut Vec<AgentDiagnostic>,
) {
    for (inspected_entries, entry) in walkdir::WalkDir::new(root)
        .follow_links(false)
        .max_depth(6)
        .sort_by_file_name()
        .into_iter()
        .enumerate()
    {
        if inspected_entries >= MAX_WORKSPACE_PROCEDURE_ENTRIES {
            diagnostics.push(AgentDiagnostic::new(
                "procedure_discovery_entry_limit",
                root.display().to_string(),
                format!(
                    "at most {MAX_WORKSPACE_PROCEDURE_ENTRIES} filesystem entries are inspected"
                ),
            ));
            return;
        }
        if documents.len() >= MAX_WORKSPACE_PROCEDURES {
            diagnostics.push(AgentDiagnostic::new(
                "procedure_catalog_limit",
                root.display().to_string(),
                format!("at most {MAX_WORKSPACE_PROCEDURES} workspace procedures are loaded"),
            ));
            return;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                diagnostics.push(AgentDiagnostic::new(
                    "procedure_walk_failed",
                    root.display().to_string(),
                    error.to_string(),
                ));
                continue;
            }
        };
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                diagnostics.push(AgentDiagnostic::new(
                    "procedure_metadata_unavailable",
                    relative_path(&workspace.root, path),
                    error.to_string(),
                ));
                continue;
            }
        };
        if crate::workspace::boundary::is_symlink_or_reparse(&metadata) {
            if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
                diagnostics.push(AgentDiagnostic::new(
                    "linked_procedure_refused",
                    relative_path(&workspace.root, path),
                    "linked procedure documents are not loaded",
                ));
            }
            continue;
        }
        if !metadata.is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("md")
        {
            continue;
        }
        let source_path = relative_path(&workspace.root, path);
        match read_procedure(path).and_then(|text| {
            ProcedureDocument::parse(&text, ProcedureOrigin::WorkspaceRoot, &source_path)
        }) {
            Ok(document) => documents.push(document),
            Err(error) => diagnostics.push(AgentDiagnostic::new(
                error.code(),
                source_path,
                error.to_string(),
            )),
        }
    }
}

fn read_procedure(path: &Path) -> Result<String, DocumentError> {
    let file = std::fs::File::open(path).map_err(|_| DocumentError::NotUtf8)?;
    let mut bytes = Vec::new();
    file.take((super::procedure::document::MAX_PROCEDURE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| DocumentError::NotUtf8)?;
    if bytes.len() > super::procedure::document::MAX_PROCEDURE_BYTES {
        return Err(DocumentError::TooLarge {
            len: bytes.len(),
            max: super::procedure::document::MAX_PROCEDURE_BYTES,
        });
    }
    String::from_utf8(bytes).map_err(|_| DocumentError::NotUtf8)
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn bound_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn facts() -> ResolvedRuntimeFacts {
        ResolvedRuntimeFacts {
            available_capabilities: BTreeSet::from([
                "workspace.fs.read".to_string(),
                "workspace.fs.write".to_string(),
            ]),
            native_tool_use: true,
            structured_output: false,
            context_tokens: Some(30_000),
            modalities: BTreeSet::new(),
        }
    }

    fn authorized(selector: AgentSelector) -> AgentActivationConfig {
        AgentActivationConfig {
            selector,
            workspace_source_authorized: true,
            load_workspace_instructions: true,
            allow_remediation_procedures: true,
            constraints: OperatorConstraints {
                max_steps_cap: Some(20),
                max_tool_calls_cap: Some(40),
                max_procedure_selections_cap: Some(3),
                ..OperatorConstraints::unconstrained()
            },
            context_tokens: Some(30_000),
        }
    }

    fn write_agent(workspace: &Workspace) {
        let root = workspace.root.join("agents/ops");
        std::fs::create_dir_all(root.join("procedures")).unwrap();
        std::fs::write(
            root.join("agent.toml"),
            r#"
schema_version = 1
id = "ops"
definition_version = "1.0.0"
display_name = "Ops"
default_instructions_path = "instructions.md"

[capability_policy]
allow = ["workspace.fs.read"]

[procedure_policy]
max_selected = 2
"#,
        )
        .unwrap();
        std::fs::write(
            root.join("instructions.md"),
            "Inspect before changing anything.",
        )
        .unwrap();
        std::fs::write(
            root.join("procedures/rollback.md"),
            "---\nschema_version: 1\nid: ops.rollback\nversion: 1.0.0\nstatus: active\ntitle: Roll back\nmode: diagnose\nrisk_level: low\nintents: [rollback]\n---\n\n# Inspect\n\nVerify the current deployment before rollback.\n",
        )
        .unwrap();
    }

    #[test]
    fn legacy_activation_can_snapshot_trusted_workspace_instructions() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        std::fs::write(workspace.root.join("AGENTS.md"), "Workspace rule.").unwrap();
        let runtime =
            AgentRuntime::load(&workspace, authorized(AgentSelector::legacy()), facts()).unwrap();

        let resolved = runtime.resolve_for_run("inspect", None).unwrap();

        assert!(resolved.profile.is_legacy());
        assert!(resolved.profile.instructions.is_some());
        assert!(
            resolved
                .prompt
                .messages
                .iter()
                .any(|message| message.content.contains("Workspace rule."))
        );
    }

    #[test]
    fn nested_workspace_instructions_are_only_rendered_for_matching_paths() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        std::fs::create_dir_all(workspace.root.join("apps/web")).unwrap();
        std::fs::create_dir_all(workspace.root.join("apps/api")).unwrap();
        std::fs::write(workspace.root.join("AGENTS.md"), "Root rule.").unwrap();
        std::fs::write(
            workspace.root.join("apps/web/AGENTS.md"),
            "Web subtree rule.",
        )
        .unwrap();
        std::fs::write(
            workspace.root.join("apps/api/AGENTS.md"),
            "API subtree rule.",
        )
        .unwrap();
        let runtime =
            AgentRuntime::load(&workspace, authorized(AgentSelector::legacy()), facts()).unwrap();
        let resolved = runtime
            .resolve_for_run("update apps/web/page.tsx", None)
            .unwrap();

        let stable = resolved
            .prompt
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(stable.contains("Root rule."));
        assert!(!stable.contains("Web subtree rule."));
        assert!(!stable.contains("API subtree rule."));

        let scoped =
            scoped_instruction_prompt(&resolved.profile, &["apps/web/page.tsx".to_string()]);
        assert_eq!(scoped.applications.len(), 1);
        assert_eq!(scoped.applications[0].scope, "apps/web");
        assert!(scoped.messages[0].content.contains("Web subtree rule."));
        assert!(!scoped.messages[0].content.contains("API subtree rule."));
    }

    #[test]
    fn workspace_agent_filters_capabilities_and_hydrates_matching_procedure() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        write_agent(&workspace);
        let runtime = AgentRuntime::load(
            &workspace,
            authorized(AgentSelector::parse("workspace:ops").unwrap()),
            facts(),
        )
        .unwrap();

        let resolved = runtime.resolve_for_run("diagnose rollback", None).unwrap();

        assert_eq!(
            resolved.profile.effective_capabilities,
            BTreeSet::from(["workspace.fs.read".to_string()])
        );
        assert_eq!(
            resolved.profile.procedures.as_ref().unwrap().selected_ids(),
            vec!["ops.rollback"]
        );
        assert_eq!(resolved.profile.hydrated_procedures.len(), 1);
        assert!(resolved.profile.validate_snapshot().is_ok());
    }

    #[test]
    fn same_run_resume_uses_saved_snapshot_after_source_changes() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        write_agent(&workspace);
        let config = authorized(AgentSelector::parse("workspace:ops").unwrap());
        let runtime = AgentRuntime::load(&workspace, config.clone(), facts()).unwrap();
        let original = runtime
            .resolve_for_run("diagnose rollback", None)
            .unwrap()
            .profile;
        std::fs::write(
            workspace.root.join("agents/ops/instructions.md"),
            "Changed source.",
        )
        .unwrap();
        let reloaded = AgentRuntime::load(&workspace, config, facts()).unwrap();

        let resumed = reloaded
            .resolve_for_run("diagnose rollback", Some(&original))
            .unwrap();

        assert!(resumed.resumed_from_snapshot);
        assert_eq!(resumed.profile, original);
        assert!(
            resumed
                .prompt
                .messages
                .iter()
                .any(|message| message.content.contains("Inspect before changing"))
        );
        assert!(
            resumed
                .prompt
                .messages
                .iter()
                .all(|message| !message.content.contains("Changed source"))
        );
    }

    #[test]
    fn workspace_content_fails_closed_without_project_trust() {
        let temp = tempfile::TempDir::new().unwrap();
        let workspace = Workspace::detect(temp.path()).unwrap();
        let error = AgentRuntime::load(
            &workspace,
            AgentActivationConfig {
                selector: AgentSelector::legacy(),
                load_workspace_instructions: true,
                ..AgentActivationConfig::default()
            },
            facts(),
        )
        .unwrap_err();

        assert_eq!(error.code(), "workspace_source_not_authorized");
    }
}
