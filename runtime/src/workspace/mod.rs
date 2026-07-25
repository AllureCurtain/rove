//! Workspace detection and path-boundary enforcement.

pub mod boundary;
pub mod root;

pub use boundary::*;
pub use root::{Workspace, WorkspaceKind};
