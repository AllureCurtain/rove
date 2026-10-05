//! API-global ProductStore.
//!
//! Owns the product control-plane contracts (workspaces, sessions, messages,
//! controls, preferences, provider selections, reviews, attachments metadata)
//! and the SQLite catalog behind them. It holds no canonical runtime events:
//! transcripts are projected from each workspace's StateStore by `rove-api`.
//! `rove-api` re-exports everything here under `rove_api::product`.

pub mod attachment_paths;
mod contracts;
pub mod cursor;
pub mod ownership;
pub mod pricing;
pub mod secret_patterns;
pub mod store;
pub mod text;

pub use contracts::*;
pub use cursor::{
    ProductCursorError, ProductMessageSearchCursor, ProductSearchCursor, ProductSessionCursor,
    ProductTraceCursorPosition, SESSION_RANK_ARCHIVED, SESSION_RANK_LIVE,
};
