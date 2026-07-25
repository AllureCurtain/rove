//! First-party CLI/REPL/TUI surfaces for Rove.

pub mod cli;
pub mod product_registry;
pub mod terminal;
pub mod tui;

pub use product_registry::{
    tool_registry, tool_registry_with_mcp, tool_registry_with_shell_policy,
};
