use thiserror::Error;

pub use rove_core::ToolError;
pub use rove_models::ModelError;

/// Top-level application errors (used at binary/interface boundary).
#[derive(Debug, Error)]
pub enum RoveError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Workspace error: {0}")]
    Workspace(String),

    #[error("Model error: {0}")]
    Model(#[from] ModelError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("State error: {0}")]
    State(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}
