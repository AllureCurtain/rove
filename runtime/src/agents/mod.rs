//! Agent definitions, workspace instructions, and procedural knowledge.
//!
//! Layered so that the authority rules can be reasoned about independently of
//! the filesystem:
//!
//! ```text
//! authority   instruction authority classes and conflict resolution
//! hashing     canonical content hashing (line-ending / order stable)
//! selector    `<source>:<agent-id>` parsing, no silent shadowing
//! definition  AgentDefinition manifest — the author-maintained source
//! validation  everything checked before a package may activate
//! package     bounded loading of a package from disk
//! instructions  root-then-nested `AGENTS.md` bundle and path overlays
//! procedure   typed procedure documents, catalog, selection, hydration
//! profile     AgentRuntimeProfile — the immutable compiled run snapshot
//! ```
//!
//! The invariant that ties them together: a document's authority comes from
//! *where it was found*, never from what it says about itself.

pub mod activation;
pub mod authority;
pub mod definition;
pub mod hashing;
pub mod instructions;
pub mod package;
pub mod procedure;
pub mod profile;
pub mod selector;
pub mod validation;

pub use activation::{
    AgentActivationConfig, AgentActivationError, AgentDiagnostic, AgentProfileIdentity,
    AgentPromptAssembly, AgentRuntime, ProcedurePromptAssembly, ResolvedAgentRun,
    ScopedInstructionApplication, ScopedInstructionPrompt, procedure_prompt_for_target,
    scoped_instruction_path_hints, scoped_instruction_prompt,
};
pub use authority::{AuthorityClass, ContentClass};
pub use definition::{AgentDefinition, legacy_definition};
pub use instructions::{InstructionBundle, InstructionOverlay};
pub use package::{AgentPackage, PackageLoadError, load_agent_package};
pub use profile::{
    AgentRuntimeProfile, ProfileBuildError, ProfileInputs, ResolvedBound, ResolvedRuntimeFacts,
    attach_hydrated_procedures, attach_instruction_bundle, build_runtime_profile, legacy_profile,
};
pub use selector::{AgentSelector, AgentSource, SelectorError};
