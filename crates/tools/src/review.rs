//! Snapshot-backed tools for hard read-only Review runs.
//!
//! These tools live in `rove-review` (crates/review/src/tool.rs);
//! `rove_runtime::tools::review::*` resolves exactly as before via this
//! re-export shim.

pub use rove_review::tool::*;
