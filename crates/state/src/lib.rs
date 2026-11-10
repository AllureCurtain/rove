//! `rove-state` — durable run state extracted from `rove-runtime`.
//!
//! Holds the durable side of the runtime: `state/` (StateStore, trace,
//! task_state snapshots, report, SQLite index, repair, reconciliation, Tool
//! Artifacts), `memory/` durable side (session/durable memory), and the
//! `TaskState`/`RunRequest`/`PromptCheckpoint`/`MessageDeliveryRecord`
//! snapshot types that embed `AgentRuntimeProfile`.

pub mod memory;
pub mod state;
pub mod task_state;

/// Merged data view: foundation's serialized data types plus this crate's
/// task-snapshot types, mirroring the layout this code had when it lived in
/// the runtime crate.
pub mod types {
    pub use crate::task_state::*;
    pub use rove_foundation::types::*;
}

/// Whether a run of `session_id` may inherit `resume_state`'s breaker.
///
/// A run only continues the session it records into. Fork bootstrap hands the
/// child the *parent's* snapshot under a new session id, and a host can adopt
/// another session's run outright (the CLI's `/resume <other-run>`). Everything
/// else in such a seed is still the run's starting state, but the breaker is
/// not: its failure count and the cooldown it armed describe failures this
/// session never had. Inheriting them would refuse the new session's automatic
/// compaction for the other session's outage and grow a count that belongs to
/// someone else.
///
/// The predicate is deliberately one function: the snapshot writer and the run
/// path have to agree about which session a breaker belongs to, or one of them
/// would inherit what the other carries forward.
///
/// This lives in `rove-state` next to `TaskState` — the session-breaker
/// contract is a property of the persisted snapshot, not of the compaction
/// behaviour that lives in `rove-engine::context`.
pub fn inherits_session_breaker(
    resume_state: &task_state::TaskState,
    session_id: rove_protocol::SessionId,
) -> bool {
    resume_state.session_id == session_id
}
