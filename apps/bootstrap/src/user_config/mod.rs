//! User-owned provider configuration.
//!
//! The user document is deliberately separate from workspace configuration:
//! workspace files may select an existing profile, but cannot define provider
//! endpoints or credential sources.

mod document;
mod loader;
mod paths;
mod writer;

pub use document::{
    ModelDefaults, USER_CONFIG_SCHEMA_VERSION, UserConfigDocument, UserConfigError,
};
pub use loader::UserConfigLoader;
pub use paths::{USER_CONFIG_ROOT_ENV, UserConfigPaths};
pub use writer::UserConfigWriter;

pub(crate) use writer::{harden_directory_permissions, harden_file_permissions};
