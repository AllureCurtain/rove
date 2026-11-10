//! `rove-review` — hard read-only code-review domain extracted from `rove-runtime`.
//!
//! Owns the serialized review contracts (target snapshots, findings, results),
//! the snapshot/capture/sanitize/finalize behaviour, and the snapshot-backed
//! submission tools (`submit_review_findings` and the read-only snapshot
//! readers). It does not add a second agent loop: hosts run the normal Engine
//! with `RunMode::Review`, a review registry, and the immutable snapshot
//! captured here.

pub mod finding;
pub mod result;
pub mod target;
pub mod tool;

pub use finding::*;
pub use result::*;
pub use target::*;
pub use tool::*;
