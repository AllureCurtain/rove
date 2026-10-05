//! Wire protocol vocabulary for Rove.
//!
//! This is the workspace leaf crate. It owns the types that appear in persisted
//! artifacts, HTTP paths, and SSE payloads, and it depends on nothing but
//! `serde` and `ulid` — no async runtime, no HTTP framework, no OpenAPI
//! derive. That constraint is the point: a consumer that only needs to read a
//! run id or match on a run status can link this crate alone.
//!
//! Historic paths keep working. `rove-runtime` re-exports the identifiers and
//! lifecycle enums from `rove_runtime::types`, and `rove-core` re-exports
//! [`CallId`], so existing call sites are unaffected by the move.
//!
//! OpenAPI schemas are attached at the point of use in `apps/api` via
//! `#[schema(value_type = String, format = "ulid")]`, which is why the
//! identifiers here carry no `utoipa` derive.

pub mod envelope;
pub mod ids;
pub mod lifecycle;
pub mod version;

pub use envelope::Versioned;
pub use ids::{CallId, JobId, RunId, SessionId};
pub use lifecycle::{ApprovalDecision, ApprovalPolicy, RunMode, RunStatus};
pub use version::{PROTOCOL_VERSION, protocol_version};
