pub mod anthropic;
mod error;
pub mod fake;
pub mod health;
pub mod ollama;
pub mod openai;
pub mod openai_responses;
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
