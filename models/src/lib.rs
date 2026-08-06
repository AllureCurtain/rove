// Legacy HTTP client modules are **test-only** parity oracles for
// `provider/protocols/*` (not production assembly). Production uses
// `provider/*` exclusively via bootstrap registry. Full deletion of these
// modules is allowed once parity tests no longer import them (follow-up).
#[cfg(test)]
mod anthropic;
pub mod assembly;
mod error;
pub mod fake;
pub mod health;
pub mod history;
#[cfg(test)]
mod ollama;
#[cfg(test)]
mod openai;
#[cfg(test)]
mod openai_responses;
mod options;
mod protocol;
pub mod provider;
pub mod routing;
pub mod traits;

pub use assembly::{TurnAssembler, assemble_turn};
pub use error::ModelError;
pub use fake::{FakeModelClient, FakeTurn};
pub use history::{
    HistoryProjectionError, HistoryProjectionPolicy, HistoryProjector, ProjectedHistory,
    ProjectionDiagnostic,
};
pub use options::ProviderOptions;
pub use protocol::{
    AssistantTurn, CANONICAL_MESSAGE_SCHEMA_VERSION, ContentBlock, InternalCallId,
    MAX_CONTENT_BLOCKS, MAX_CONTENT_BYTES, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_CALLS,
    MAX_TOOL_ID_BYTES, MAX_TOOL_NAME_BYTES, Message, ModelMessage, ModelToolSchema,
    ProtocolValidationError, Role, StopReason, ToolCall, ToolCallRef, ToolResult, ToolResultStatus,
    TurnProvenance, Usage, WireCallReference,
};
pub use traits::{ModelClient, ModelClientId, ModelEvent, ProviderCapabilities};
