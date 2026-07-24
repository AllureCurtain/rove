//! First-party CLI/REPL/TUI surfaces for Rove.

pub mod cli;
pub mod product_registry;
pub mod terminal;
pub mod tui;

pub use product_registry::{
    default_tool_registry, default_tool_registry_with_shell_policy, runtime_tool_registry,
};
