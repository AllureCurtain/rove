//! Procedural knowledge: documents, trust, catalog, and selection (design §11–§13).
//!
//! Layering, innermost first:
//!
//! * [`frontmatter`] — a deliberately restricted metadata parser over untrusted text.
//! * [`metadata`] — typed [`ProcedureMetadata`] with stable field codes.
//! * [`trust`] — trust derived from source location, never self-declared.
//! * [`document`] — a parsed document bound to its provenance.
//! * [`catalog`] — the loaded set, with hard eligibility filtering.
//! * [`selection`] — deterministic ranking and bounded selection.
//! * [`hydration`] — the four disclosure levels.
//!
//! The invariant that shapes the whole module: eligibility is decided before
//! ranking, and ranking never resurrects an ineligible document.

pub mod catalog;
pub mod document;
pub mod frontmatter;
pub mod hydration;
pub mod metadata;
pub mod selection;
pub mod trust;

pub use catalog::{
    CatalogEntry, EligibilityContext, EligibilityOutcome, IneligibilityReason, ProcedureCatalog,
};
pub use document::{DocumentError, ProcedureDocument, ProcedureReference};
pub use metadata::{
    PROCEDURE_SCHEMA_VERSION, ProcedureMetadata, ProcedureMode, ProcedureStatus, RiskLevel,
    SideEffect,
};
pub use selection::{ProcedureSelection, SelectedProcedure, SelectionScore, select_procedures};
pub use trust::{ProcedureOrigin, ProcedureProvenance, ProcedureTrust};
