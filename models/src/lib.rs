// Legacy HTTP client modules are **test-only** parity oracles for
// `provider/protocols/*` (not production assembly). Production uses
// `provider/*` exclusively via bootstrap registry. Full deletion of these
// modules is allowed once parity tests no longer import them (follow-up).
#[cfg(test)]
mod anthropic;
mod error;
pub mod fake;
pub mod health;
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

pub use error::ModelError;
pub use fake::{FakeModelClient, FakeTurn};
pub use options::ProviderOptions;
pub use protocol::{Message, Role, ToolCallRef, ToolSchema, Usage};
pub use traits::{ModelClient, ModelClientId, ModelEvent};
