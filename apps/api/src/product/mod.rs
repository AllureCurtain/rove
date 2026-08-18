//! API-global product control plane.
//!
//! This module owns public contracts and routes, the SQLite ProductStore,
//! browser migration coordination, and canonical-event transcript projection.

pub(crate) mod artifacts;
mod contracts;
pub(crate) mod diff;
pub(crate) mod export;
pub(crate) mod files;
pub(crate) mod mcp;
pub(crate) mod message_adapter;
pub(crate) mod migration;
pub(crate) mod platform;
pub(crate) mod provider_catalog;
pub(crate) mod review;
pub(crate) mod routes;
pub(crate) mod store;
pub(crate) mod transcript;
pub(crate) mod trust;
pub(crate) mod usage;

pub use artifacts::{
    ProductArtifactAvailability, ProductArtifactContentEnvelope, ProductArtifactPreviewKind,
    ProductArtifactSourceKind, ProductArtifactView, ProductArtifactsResponse,
};
pub use contracts::*;
pub use diff::{ProductDiffEntry, ProductDiffOp, ProductDiffSource, ProductSessionDiffResponse};
pub use export::{
    ProductExportChild, ProductExportFormat, ProductExportLineage, ProductExportPartialReasons,
    ProductExportQuery, ProductExportRedactionSummary, ProductExportSafety, ProductExportSession,
    ProductExportWorkspace, ProductSessionExport,
};
pub use files::{
    ProductFileContentEnvelope, ProductFileEntry, ProductFileKind, ProductFilesResponse,
    ProductImageMetadata,
};
