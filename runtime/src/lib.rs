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
//!   foundation/   types, events, session, runtime identity, secret authority
//! ```
//!
//! `rove-foundation` (crates/foundation) holds the serialized identity and
//! lifecycle data extracted from this crate. Historic flat public paths
//! (`execution`, `executor`, `hooks`, `types`, `events`, `foundation`, …)
//! remain available as crate-root re-exports so apps and integration tests do
//! not need a mass import rewrite in this PR.

pub mod agents;
pub mod context;
pub mod conversation;
pub mod engine;
pub mod environment;
pub mod memory;
pub mod planning;
pub mod review;
pub mod state;
pub mod tools;

// ---------------------------------------------------------------------------
// Foundation crate re-export (rove-foundation holds the serialized data)
// ---------------------------------------------------------------------------

/// Serialized identity/lifecycle records extracted to `rove-foundation`.
/// `rove_runtime::foundation::*` resolves exactly as before.
pub mod foundation {
    pub use rove_foundation::capability;
    pub use rove_foundation::compaction_facts;
    pub use rove_foundation::events;
    pub use rove_foundation::runtime_identity;
    pub use rove_foundation::secrets;
    pub use rove_foundation::session;

    /// Task snapshot and checkpoint types that stay in `rove-runtime` because
    /// they embed the behaviour aggregate `AgentRuntimeProfile`.
    pub mod task_state;

    /// `rove_runtime::foundation::types` = foundation data types + the
    /// runtime-side task snapshot types.
    pub mod types {
        pub use crate::foundation::task_state::*;
        pub use rove_foundation::types::*;
    }

    pub use capability::*;
    pub use events::*;
    pub use runtime_identity::*;
    pub use session::*;
    pub use types::*;
}

// ---------------------------------------------------------------------------
// Stable public path aliases (pre-W2b flat module names)
// ---------------------------------------------------------------------------

pub use rove_foundation::capability;
pub use rove_foundation::events;
pub use rove_foundation::runtime_identity;
pub use rove_foundation::secrets;
pub use rove_foundation::session;
pub use rove_foundation::user_input as tool_input;

/// Historic `rove_runtime::types` path: foundation's data types plus the
/// runtime-side task snapshot types.
pub mod types {
    pub use crate::foundation::task_state::*;
    pub use rove_foundation::types::*;
}

pub use planning::execution;
pub use planning::planner;

pub use tools::executor;
pub use tools::hooks;

/// Workspace detection + boundary types live in `rove-foundation`;
/// `rove_runtime::workspace::*` resolves exactly as before.
pub mod workspace {
    pub mod boundary {
        pub use rove_foundation::boundary::*;
    }
    pub mod root {
        pub use rove_foundation::root::*;
    }
    pub use boundary::*;
    pub use root::*;
}

pub use workspace::boundary;

pub use context::compaction;
pub use rove_foundation::prompt_metadata;

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
