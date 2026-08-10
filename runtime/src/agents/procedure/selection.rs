//! Deterministic procedure ranking and bounded selection (design §12.2).
//!
//! Two properties matter more than ranking quality:
//!
//! 1. **Determinism.** The same catalog and the same request always produce the
//!    same selection, including ties, so a run can be replayed and audited.
//!    Nothing here reads the clock, the filesystem, or a random source.
//! 2. **Ranking cannot override eligibility.** The input is
//!    [`ProcedureCatalog::eligible`], so an ineligible document has no score at
//!    all. There is deliberately no "boost" that can reach past that.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::catalog::ProcedureCatalog;
use super::document::{ProcedureDocument, ProcedureReference};
use super::metadata::RiskLevel;
use super::trust::ProcedureTrust;

/// Hard ceiling on selections, independent of any package request.
///
/// Prompt budget is finite and shared. A package asking for more than this is
/// clamped rather than honoured (design §8.7: a package cannot widen its own
/// bounds).
pub const MAX_SELECTED_PROCEDURES: usize = 8;

/// What the run is looking for. All fields are runtime- or user-supplied; none
/// come from a procedure document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionRequest {
    /// Intent tokens derived from the task, e.g. `rollback`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub intents: BTreeSet<String>,
    /// Tag tokens derived from the task or the planner.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub tags: BTreeSet<String>,
    /// Scope hint, e.g. a subdirectory the task is confined to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// Maximum selections requested. Clamped to [`MAX_SELECTED_PROCEDURES`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_selected: Option<usize>,
    /// Highest risk level the run will accept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_risk_level: Option<RiskLevel>,
}

impl SelectionRequest {
    /// Effective selection limit: the request bounded by the hard ceiling, and
    /// by the operator cap when one is supplied.
    pub fn effective_limit(&self, operator_cap: Option<usize>) -> usize {
        let requested = self.max_selected.unwrap_or(3);
        let capped = match operator_cap {
            Some(cap) => requested.min(cap),
            None => requested,
        };
        capped.min(MAX_SELECTED_PROCEDURES)
    }
}

/// The score components, kept separate so an audit log can explain a choice
/// rather than showing an opaque number.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionScore {
    /// Matching intent tokens.
    pub intent_matches: u32,
    /// Matching tag tokens.
    pub tag_matches: u32,
    /// 1 when the document scope prefixes the requested scope.
    pub scope_match: u32,
    /// Narrower targeting is preferred over universal when both match.
    pub specificity: u32,
    /// Trust bonus: builtin over workspace over user-installed.
    pub trust_rank: u32,
    /// Lower risk preferred when everything else ties.
    pub risk_rank: u32,
}

impl SelectionScore {
    /// Weighted total. Weights are fixed constants, not tunables, so the same
    /// build always ranks the same way.
    pub fn total(self) -> u32 {
        self.intent_matches * 100
            + self.tag_matches * 20
            + self.scope_match * 40
            + self.specificity * 5
            + self.trust_rank * 3
            + self.risk_rank
    }

    /// Whether the document matched the request at all.
    ///
    /// A document that matches nothing is not selected even when slots remain:
    /// filling the prompt with unrelated runbooks is worse than sending none.
    pub fn is_relevant(self) -> bool {
        self.intent_matches > 0 || self.tag_matches > 0 || self.scope_match > 0
    }
}

/// One selected procedure, with the score that justified it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectedProcedure {
    pub reference: ProcedureReference,
    pub score: SelectionScore,
    pub total: u32,
}

/// The outcome of a selection pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureSelection {
    pub selected: Vec<SelectedProcedure>,
    /// Eligible documents considered but not selected, in rank order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub considered: Vec<SelectedProcedure>,
    /// Effective limit actually applied.
    pub limit: usize,
    /// Eligible documents excluded for exceeding the accepted risk level.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risk_excluded: Vec<String>,
    /// Selections dropped because they conflict with a higher-ranked selection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflict_excluded: Vec<String>,
}

impl ProcedureSelection {
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// IDs of the selected procedures, in selection order.
    pub fn selected_ids(&self) -> Vec<&str> {
        self.selected
            .iter()
            .map(|selection| selection.reference.id.as_str())
            .collect()
    }

    /// Stable hash input describing exactly what was selected, for the run
    /// identity. Content hashes are included so an edited procedure changes it.
    pub fn identity_components(&self) -> Vec<String> {
        self.selected
            .iter()
            .map(|selection| {
                format!(
                    "{}@{}#{}",
                    selection.reference.id,
                    selection.reference.version,
                    selection.reference.content_hash
                )
            })
            .collect()
    }
}

/// Trust bonus. Small, so it breaks ties without overriding relevance.
fn trust_rank(trust: ProcedureTrust) -> u32 {
    match trust {
        ProcedureTrust::BuiltinTrusted => 3,
        ProcedureTrust::WorkspaceTrusted => 2,
        ProcedureTrust::UserInstalled => 1,
        // Unreachable through the catalog, which excludes it. Scored zero
        // rather than panicking so a future caller cannot be surprised.
        ProcedureTrust::ExternalUntrusted => 0,
    }
}

fn risk_rank(risk: RiskLevel) -> u32 {
    match risk {
        RiskLevel::Low => 2,
        RiskLevel::Medium => 1,
        RiskLevel::High => 0,
    }
}

/// Score one document against a request.
fn score(document: &ProcedureDocument, request: &SelectionRequest) -> SelectionScore {
    let metadata = &document.metadata;

    let intent_matches = metadata.intents.intersection(&request.intents).count() as u32;
    let tag_matches = metadata.tags.intersection(&request.tags).count() as u32;

    let scope_match = match (metadata.scope.as_deref(), request.scope.as_deref()) {
        (Some(declared), Some(requested)) if requested.starts_with(declared) => 1,
        _ => 0,
    };

    // Narrower targeting wins a tie: a Windows-specific Rust procedure is a
    // better answer than a universal one when both match.
    let specificity = [
        !metadata.agents.is_empty(),
        !metadata.platforms.is_empty(),
        !metadata.workspace_kinds.is_empty(),
        metadata.scope.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count() as u32;

    SelectionScore {
        intent_matches,
        tag_matches,
        scope_match,
        specificity,
        trust_rank: trust_rank(document.trust()),
        risk_rank: risk_rank(metadata.risk_level),
    }
}

/// Rank and select from the eligible portion of a catalog.
///
/// `operator_cap` is the operator's ceiling on selections; `None` means the
/// operator set none, in which case the request is still bounded by
/// [`MAX_SELECTED_PROCEDURES`].
pub fn select_procedures(
    catalog: &ProcedureCatalog,
    request: &SelectionRequest,
    operator_cap: Option<usize>,
) -> ProcedureSelection {
    let limit = request.effective_limit(operator_cap);

    let mut risk_excluded = Vec::new();
    let mut ranked: Vec<SelectedProcedure> = Vec::new();

    for entry in catalog.eligible() {
        let document = &entry.document;
        if let Some(max_risk) = request.max_risk_level
            && document.metadata.risk_level > max_risk
        {
            risk_excluded.push(document.metadata.id.clone());
            continue;
        }
        let score = score(document, request);
        if !score.is_relevant() {
            continue;
        }
        ranked.push(SelectedProcedure {
            reference: document.reference(),
            score,
            total: score.total(),
        });
    }

    // Ties break on ID, which is stable across runs and across filesystem
    // enumeration order. Without this, two equally good procedures could swap
    // between runs and make a replay diverge.
    ranked.sort_by(|left, right| {
        right
            .total
            .cmp(&left.total)
            .then_with(|| left.reference.id.cmp(&right.reference.id))
    });

    let mut selected: Vec<SelectedProcedure> = Vec::new();
    let mut conflict_excluded = Vec::new();
    let mut considered = Vec::new();

    for candidate in ranked {
        if selected.len() >= limit {
            considered.push(candidate);
            continue;
        }
        // A conflict declared by either side excludes the lower-ranked
        // document, so two contradictory runbooks never enter the same prompt.
        let conflicts = selected.iter().any(|chosen| {
            let chosen_document = catalog
                .get(&chosen.reference.id)
                .map(|entry| &entry.document);
            let candidate_document = catalog
                .get(&candidate.reference.id)
                .map(|entry| &entry.document);
            let chosen_declares = chosen_document.is_some_and(|document| {
                document
                    .metadata
                    .conflicts_with
                    .contains(&candidate.reference.id)
            });
            let candidate_declares = candidate_document.is_some_and(|document| {
                document
                    .metadata
                    .conflicts_with
                    .contains(&chosen.reference.id)
            });
            chosen_declares || candidate_declares
        });
        if conflicts {
            conflict_excluded.push(candidate.reference.id.clone());
            continue;
        }
        selected.push(candidate);
    }

    risk_excluded.sort();
    conflict_excluded.sort();

    ProcedureSelection {
        selected,
        considered,
        limit,
        risk_excluded,
        conflict_excluded,
    }
}

#[cfg(test)]
mod tests {
    use super::super::catalog::EligibilityContext;
    use super::super::trust::ProcedureOrigin;
    use super::*;

    fn document(id: &str, extra: &str, origin: ProcedureOrigin) -> ProcedureDocument {
        let text = format!(
            "---\nschema_version: 1\nid: {id}\nversion: 1.0.0\nstatus: active\ntitle: T\nmode: diagnose\nrisk_level: low\n{extra}---\n\nBody for {id}.\n"
        );
        ProcedureDocument::parse(&text, origin, format!("procedures/{id}.md"))
            .expect("test document parses")
    }

    fn workspace(id: &str, extra: &str) -> ProcedureDocument {
        document(id, extra, ProcedureOrigin::WorkspacePackage)
    }

    fn catalog(documents: Vec<ProcedureDocument>) -> ProcedureCatalog {
        ProcedureCatalog::build(
            documents,
            &EligibilityContext::new("coder", "windows", "2026-08-09"),
        )
    }

    fn request(intents: &[&str]) -> SelectionRequest {
        SelectionRequest {
            intents: intents.iter().map(|value| value.to_string()).collect(),
            ..SelectionRequest::default()
        }
    }

    #[test]
    fn an_intent_match_is_selected() {
        let catalog = catalog(vec![workspace("a.b", "intents: [rollback]\n")]);
        let selection = select_procedures(&catalog, &request(&["rollback"]), None);
        assert_eq!(selection.selected_ids(), vec!["a.b"]);
        assert_eq!(selection.selected[0].score.intent_matches, 1);
    }

    /// Filling remaining slots with unrelated runbooks costs prompt budget and
    /// adds noise, so irrelevant documents are simply not selected.
    #[test]
    fn an_irrelevant_document_is_not_selected_even_with_slots_free() {
        let catalog = catalog(vec![workspace("a.b", "intents: [deploy]\n")]);
        let selection = select_procedures(&catalog, &request(&["rollback"]), None);
        assert!(selection.is_empty());
    }

    #[test]
    fn intent_outweighs_tag_and_trust() {
        let catalog = catalog(vec![
            document(
                "a.tagonly",
                "tags: [rollback]\n",
                ProcedureOrigin::BuiltinPackage,
            ),
            workspace("a.intent", "intents: [rollback]\n"),
        ]);
        let mut request = request(&["rollback"]);
        request.tags = BTreeSet::from(["rollback".to_string()]);
        let selection = select_procedures(&catalog, &request, None);
        assert_eq!(selection.selected_ids().first(), Some(&"a.intent"));
    }

    #[test]
    fn trust_breaks_a_tie_in_favour_of_builtin() {
        let catalog = catalog(vec![
            workspace("a.workspace", "intents: [rollback]\n"),
            document(
                "a.builtin",
                "intents: [rollback]\n",
                ProcedureOrigin::BuiltinPackage,
            ),
        ]);
        let selection = select_procedures(&catalog, &request(&["rollback"]), None);
        assert_eq!(selection.selected_ids().first(), Some(&"a.builtin"));
    }

    #[test]
    fn narrower_targeting_wins_a_tie_over_universal() {
        let catalog = catalog(vec![
            workspace("a.universal", "intents: [rollback]\n"),
            workspace(
                "a.specific",
                "intents: [rollback]\nplatforms: [windows]\nagents: [coder]\n",
            ),
        ]);
        let selection = select_procedures(&catalog, &request(&["rollback"]), None);
        assert_eq!(selection.selected_ids().first(), Some(&"a.specific"));
    }

    #[test]
    fn a_scope_prefix_match_scores_and_a_mismatch_does_not() {
        let catalog = catalog(vec![workspace("a.b", "scope: services/api\n")]);

        let matching = SelectionRequest {
            scope: Some("services/api/handlers".to_string()),
            ..SelectionRequest::default()
        };
        assert_eq!(
            select_procedures(&catalog, &matching, None).selected_ids(),
            vec!["a.b"]
        );

        let mismatched = SelectionRequest {
            scope: Some("apps/web".to_string()),
            ..SelectionRequest::default()
        };
        assert!(select_procedures(&catalog, &mismatched, None).is_empty());
    }

    /// Replay depends on this: identical inputs must give an identical order,
    /// including when scores tie exactly.
    #[test]
    fn ties_break_deterministically_on_id_regardless_of_input_order() {
        let forward = catalog(vec![
            workspace("a.first", "intents: [x]\n"),
            workspace("b.second", "intents: [x]\n"),
            workspace("c.third", "intents: [x]\n"),
        ]);
        let reverse = catalog(vec![
            workspace("c.third", "intents: [x]\n"),
            workspace("b.second", "intents: [x]\n"),
            workspace("a.first", "intents: [x]\n"),
        ]);
        let request = request(&["x"]);
        assert_eq!(
            select_procedures(&forward, &request, None).selected_ids(),
            select_procedures(&reverse, &request, None).selected_ids()
        );
        assert_eq!(
            select_procedures(&forward, &request, None).selected_ids(),
            vec!["a.first", "b.second", "c.third"]
        );
    }

    #[test]
    fn selection_is_bounded_by_the_request_limit() {
        let documents: Vec<ProcedureDocument> = (0..6)
            .map(|index| workspace(&format!("a.p{index}"), "intents: [x]\n"))
            .collect();
        let catalog = catalog(documents);
        let mut request = request(&["x"]);
        request.max_selected = Some(2);
        let selection = select_procedures(&catalog, &request, None);
        assert_eq!(selection.selected.len(), 2);
        assert_eq!(selection.limit, 2);
        assert_eq!(
            selection.considered.len(),
            4,
            "the rest are recorded, not lost"
        );
    }

    /// §8.7: a package cannot widen its own bounds.
    #[test]
    fn an_operator_cap_bounds_a_larger_request() {
        let documents: Vec<ProcedureDocument> = (0..6)
            .map(|index| workspace(&format!("a.p{index}"), "intents: [x]\n"))
            .collect();
        let catalog = catalog(documents);
        let mut request = request(&["x"]);
        request.max_selected = Some(6);
        let selection = select_procedures(&catalog, &request, Some(2));
        assert_eq!(selection.limit, 2);
        assert_eq!(selection.selected.len(), 2);
    }

    #[test]
    fn the_hard_ceiling_bounds_even_an_uncapped_request() {
        let request = SelectionRequest {
            max_selected: Some(1000),
            ..SelectionRequest::default()
        };
        assert_eq!(request.effective_limit(None), MAX_SELECTED_PROCEDURES);
        assert_eq!(
            request.effective_limit(Some(10_000)),
            MAX_SELECTED_PROCEDURES
        );
    }

    #[test]
    fn the_default_limit_is_three() {
        assert_eq!(SelectionRequest::default().effective_limit(None), 3);
    }

    #[test]
    fn a_document_over_the_accepted_risk_level_is_excluded_and_recorded() {
        let mut request = request(&["x"]);
        request.max_risk_level = Some(RiskLevel::Low);

        let low_only = catalog(vec![workspace("a.low", "intents: [x]\n")]);
        let selection = select_procedures(&low_only, &request, None);
        assert_eq!(selection.selected.len(), 1);
        assert!(selection.risk_excluded.is_empty());

        // `risk_level` must replace the default, so this document is built
        // directly rather than through the helper.
        let risky = ProcedureDocument::parse(
            "---\nschema_version: 1\nid: a.risky\nversion: 1\nstatus: active\ntitle: T\nmode: diagnose\nrisk_level: high\nintents: [x]\n---\n\nBody.\n",
            ProcedureOrigin::WorkspacePackage,
            "procedures/risky.md",
        )
        .expect("parses");
        let with_risky = catalog(vec![risky]);
        let selection = select_procedures(&with_risky, &request, None);
        assert!(selection.is_empty());
        assert_eq!(selection.risk_excluded, vec!["a.risky".to_string()]);
    }

    /// Two contradictory runbooks in one prompt is worse than one, so the
    /// lower-ranked side is dropped and recorded.
    #[test]
    fn a_declared_conflict_excludes_the_lower_ranked_document() {
        let catalog = catalog(vec![
            document(
                "a.preferred",
                "intents: [x]\nconflicts_with: [a.other]\n",
                ProcedureOrigin::BuiltinPackage,
            ),
            workspace("a.other", "intents: [x]\n"),
        ]);
        let selection = select_procedures(&catalog, &request(&["x"]), None);
        assert_eq!(selection.selected_ids(), vec!["a.preferred"]);
        assert_eq!(selection.conflict_excluded, vec!["a.other".to_string()]);
    }

    /// The conflict holds whichever side declared it.
    #[test]
    fn a_conflict_declared_by_the_lower_ranked_side_still_applies() {
        let catalog = catalog(vec![
            document(
                "a.preferred",
                "intents: [x]\n",
                ProcedureOrigin::BuiltinPackage,
            ),
            workspace("a.other", "intents: [x]\nconflicts_with: [a.preferred]\n"),
        ]);
        let selection = select_procedures(&catalog, &request(&["x"]), None);
        assert_eq!(selection.selected_ids(), vec!["a.preferred"]);
        assert_eq!(selection.conflict_excluded, vec!["a.other".to_string()]);
    }

    /// The structural guarantee: ranking has no path to an ineligible document.
    #[test]
    fn an_ineligible_document_is_never_selected_however_well_it_matches() {
        let catalog = catalog(vec![document(
            "a.untrusted",
            "intents: [x]\ntags: [x]\nscope: .\n",
            ProcedureOrigin::RuntimeSupplied,
        )]);
        let mut request = request(&["x"]);
        request.tags = BTreeSet::from(["x".to_string()]);
        request.scope = Some(".".to_string());
        let selection = select_procedures(&catalog, &request, None);
        assert!(
            selection.is_empty(),
            "a perfect-scoring untrusted document must still not be selected"
        );
        assert!(selection.considered.is_empty());
    }

    #[test]
    fn identity_components_include_id_version_and_content_hash() {
        let catalog = catalog(vec![workspace("a.b", "intents: [x]\n")]);
        let selection = select_procedures(&catalog, &request(&["x"]), None);
        let components = selection.identity_components();
        assert_eq!(components.len(), 1);
        assert!(components[0].starts_with("a.b@1.0.0#sha256:"));
    }

    /// An edited procedure must change the run identity even at the same version.
    #[test]
    fn editing_a_body_changes_the_identity_components() {
        let before = catalog(vec![workspace("a.b", "intents: [x]\n")]);
        let edited = ProcedureDocument::parse(
            "---\nschema_version: 1\nid: a.b\nversion: 1.0.0\nstatus: active\ntitle: T\nmode: diagnose\nrisk_level: low\nintents: [x]\n---\n\nDifferent body.\n",
            ProcedureOrigin::WorkspacePackage,
            "procedures/a.b.md",
        )
        .expect("parses");
        let after = catalog(vec![edited]);
        let request = request(&["x"]);
        assert_ne!(
            select_procedures(&before, &request, None).identity_components(),
            select_procedures(&after, &request, None).identity_components()
        );
    }

    #[test]
    fn an_empty_catalog_selects_nothing_without_error() {
        let catalog = catalog(vec![]);
        let selection = select_procedures(&catalog, &request(&["x"]), None);
        assert!(selection.is_empty());
        assert_eq!(selection.limit, 3);
    }

    #[test]
    fn a_zero_limit_selects_nothing() {
        let catalog = catalog(vec![workspace("a.b", "intents: [x]\n")]);
        let mut request = request(&["x"]);
        request.max_selected = Some(0);
        let selection = select_procedures(&catalog, &request, None);
        assert!(selection.is_empty());
        assert_eq!(selection.considered.len(), 1);
    }

    #[test]
    fn selection_serialization_round_trips() {
        let catalog = catalog(vec![workspace("a.b", "intents: [x]\n")]);
        let selection = select_procedures(&catalog, &request(&["x"]), None);
        let json = serde_json::to_string(&selection).expect("serializes");
        let restored: ProcedureSelection = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(selection, restored);
    }
}
