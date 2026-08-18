//! Persistent Rove execution semantics.
//!
//! Physical layout is domain-oriented:
//!
//! ```text
//! runtime/src/
//!   agents/       Agent definitions, instruction bundles, procedural knowledge
//!   engine/       Engine facade, run/plan/tool/model turn loops
//!   planning/     ExecutionPolicy, planner, plan evaluator
//!   tools/        built-in tools, Executor, hooks, tool input
//!   state/        StateStore, trace, artifacts, resume
//!   memory/       durable + session memory
//!   context/      ContextManager, compaction, prompt metadata
//!   workspace/    Workspace detection + path boundary
//!   foundation/   types, events, session, runtime identity
//! ```
//!
//! Historic flat public paths (`execution`, `executor`, `hooks`, `types`,
//! `events`, …) remain available as crate-root re-exports so apps and
//! integration tests do not need a mass import rewrite in this PR.

pub mod agents;
pub mod context;
pub mod conversation;
pub mod engine;
pub mod environment;
pub mod foundation;
pub mod memory;
pub mod planning;
pub mod review;
pub mod state;
pub mod tools;
pub mod workspace;

// ---------------------------------------------------------------------------
// Stable public path aliases (pre-W2b flat module names)
// ---------------------------------------------------------------------------

pub use foundation::capability;
pub use foundation::events;
pub use foundation::runtime_identity;
pub use foundation::session;
pub use foundation::types;

pub use planning::execution;
pub use planning::planner;

pub use tools::executor;
pub use tools::hooks;
#[doc(hidden)]
pub use tools::tool_input;

pub use workspace::boundary;

pub use context::compaction;
pub use context::prompt_metadata;

// ---------------------------------------------------------------------------
// Crate-private historic names for internal `crate::…` call sites
// ---------------------------------------------------------------------------

pub(crate) use engine::model_turn;
pub(crate) use engine::plan_loop;
pub(crate) use engine::run_loop;
pub(crate) use engine::step_runner;
pub(crate) use engine::tool_turn;
pub(crate) use planning::finalizer;
pub(crate) use planning::plan_evaluator;

// ---------------------------------------------------------------------------
// Root convenience re-exports
// ---------------------------------------------------------------------------

pub use foundation::types::{JobId, RunId, RunRequest, SessionId, TaskState};
pub use workspace::{Workspace, WorkspaceKind};
