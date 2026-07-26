//! Canonical-event transcript projection lane.
//!
//! C0 foundation intentionally leaves the implementation to the bounded
//! transcript worker. This module must project runtime facts rather than write
//! or persist a second chat history.

use std::sync::Arc;

use super::{
    ProductRuntimeStateResolver, ProductStore, ProductStoreError, ProductTranscriptReader,
};

mod reader;
mod validation;

pub(crate) fn open_product_transcript_reader(
    store: Arc<dyn ProductStore>,
    runtime_state_resolver: Arc<dyn ProductRuntimeStateResolver>,
) -> Result<Arc<dyn ProductTranscriptReader>, ProductStoreError> {
    Ok(Arc::new(reader::CanonicalProductTranscriptReader::new(
        store,
        runtime_state_resolver,
    )))
}
