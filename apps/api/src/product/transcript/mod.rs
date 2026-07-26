//! Canonical-event transcript projection lane.
//!
//! C0 foundation intentionally leaves the implementation to the bounded
//! transcript worker. This module must project runtime facts rather than write
//! or persist a second chat history.

use std::sync::Arc;

use super::{
    ProductRuntimeStateResolver, ProductStore, ProductStoreError, ProductTranscriptReader,
};

pub(crate) fn open_product_transcript_reader(
    _store: Arc<dyn ProductStore>,
    _runtime_state_resolver: Arc<dyn ProductRuntimeStateResolver>,
) -> Result<Arc<dyn ProductTranscriptReader>, ProductStoreError> {
    Err(ProductStoreError::unavailable())
}
