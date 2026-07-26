//! SQLite ProductStore implementation lane.
//!
//! C0 foundation intentionally leaves the implementation to the bounded store
//! worker. Central construction and route wiring remain coordinator-owned.

use std::path::PathBuf;
use std::sync::Arc;

use super::{ProductStore, ProductStoreError};

pub(crate) fn open_product_store(
    _path: PathBuf,
    _busy_timeout_ms: u64,
) -> Result<Arc<dyn ProductStore>, ProductStoreError> {
    Err(ProductStoreError::unavailable())
}
