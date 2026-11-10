//! `rove-foundation` — bottom-layer crate extracted from `rove-runtime`.
//!
//! Holds the serialized identity/lifecycle data and workspace-boundary types:
//! runtime identity, task types, durable stream events, sessions, secret
//! redaction, capability snapshots, workspace detection + path boundary,
//! prompt metadata, content hashing, agent leaf identities, MCP snapshot
//! records, environment records, and the planning/execution lifecycle
//! records. Behaviour (engine loops, planning evaluation, tool execution,
//! MCP transport, process/filesystem ports) stays in `rove-runtime`.

pub mod agents_leaves;
pub mod boundary;
pub mod capability;
pub mod compaction_facts;
pub mod environment_types;
pub mod events;
pub mod execution;
pub mod hashing;
pub mod mcp_types;
pub mod prompt_metadata;
pub mod root;
pub mod runtime_identity;
pub mod secrets;
pub mod selector;
pub mod session;
pub mod types;
pub mod user_input;

pub use agents_leaves::*;
pub use boundary::*;
pub use capability::*;
pub use compaction_facts::*;
pub use environment_types::*;
pub use events::*;
pub use execution::*;
pub use hashing::*;
pub use mcp_types::*;
pub use prompt_metadata::{
    PromptBuildMetadata, estimate_message_tokens, estimate_messages_tokens, message_bytes,
    prompt_cache_key, prompt_hash, stable_hash, tool_signature,
};
pub use root::*;
pub use runtime_identity::*;
pub use selector::*;
pub use session::*;
pub use types::*;
