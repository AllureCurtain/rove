//! Hard read-only code-review contracts.
//!
//! The review domain (data contracts, target snapshots, sanitize/finalize
//! behaviour, and the snapshot-backed submission tools) lives in `rove-review`
//! (crates/review); `rove_runtime::review::*` resolves exactly as before via
//! this re-export shim.

pub use rove_review::*;
