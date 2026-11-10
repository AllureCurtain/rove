//! Persistent Rove execution semantics: the Engine facade, run/plan/tool/model
//! turn loops, execution policy and planning, conversation projection, and
//! prompt-context management.
//!
//! ```text
//! crates/engine/src/
//!   engine/       Engine facade, run/plan/tool/model turn loops, delegate turn-driver
//!   planning/     ExecutionPolicy, planner, plan evaluator, finalizer
//!   context/      ContextManager, compaction
//!   conversation  unified conversation message domain
//! ```
//!
//! `rove-foundation` (crates/foundation) holds the serialized identity and
//! lifecycle data, `rove-agents` (crates/agents) the agent catalog,
//! `rove-state` (crates/state) the durable run state and memory, `rove-review`
//! (crates/review) the read-only Review domain, and `rove-tools` (crates/tools)
//! the built-in tools and Executor pipeline. The delegate turn-driver stays in
//! `engine::subrun` because it must call the engine's turn primitives.

pub mod context;
pub mod conversation;
pub mod engine;
pub mod planning;

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
