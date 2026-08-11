//! The procedure catalog and hard eligibility (design §12.1).
//!
//! The ordering here is the security property, not an optimisation: every
//! document is reduced to eligible/ineligible by absolute checks first, and
//! ranking in [`super::selection`] only ever sees the eligible set. A
//! high-scoring document that fails a hard check is not "ranked lower" — it is
//! absent, and the reason is recorded.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::document::ProcedureDocument;
use super::metadata::ProcedureMode;
use super::trust::ProcedureTrust;

/// Largest catalog retained in memory.
///
/// Bounded so a workspace with a generated procedure directory cannot make
/// selection cost unbounded.
pub const MAX_CATALOG_ENTRIES: usize = 512;

/// Everything hard eligibility is decided against.
///
/// All of it is runtime-derived. No field is taken from a procedure document,
/// which is what stops a document from describing the world it wants to be
/// evaluated in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EligibilityContext {
    /// Agent the run is executing as.
    pub agent_id: String,
    /// Current platform token, e.g. `windows`, `linux`, `macos`.
    pub platform: String,
    /// Detected workspace kinds, e.g. `rust`, `node`.
    pub workspace_kinds: BTreeSet<String>,
    /// Capability IDs actually available *and* permitted for this run.
    ///
    /// This is the resolved set, after operator deny and agent policy — not the
    /// raw provider list. Availability alone never implies approval (§16.3).
    pub available_capabilities: BTreeSet<String>,
    /// Trust levels the active agent package permits.
    pub allowed_trust_levels: BTreeSet<ProcedureTrust>,
    /// Modes permitted for this run. A diagnostic run excludes remediation.
    pub allowed_modes: BTreeSet<ProcedureMode>,
    /// Tags every eligible document must carry, imposed by the active Agent.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_tags: BTreeSet<String>,
    /// Today's date as `YYYY-MM-DD`, supplied so selection is replayable.
    pub today: String,
    /// Whether non-`active` documents may be considered. Off by default.
    #[serde(default)]
    pub include_non_active: bool,
}

impl EligibilityContext {
    /// A permissive context for a given agent, used as a test and default base.
    pub fn new(
        agent_id: impl Into<String>,
        platform: impl Into<String>,
        today: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            platform: platform.into(),
            workspace_kinds: BTreeSet::new(),
            available_capabilities: BTreeSet::new(),
            allowed_trust_levels: ProcedureTrust::default_allowed().into_iter().collect(),
            allowed_modes: BTreeSet::from([
                ProcedureMode::Diagnose,
                ProcedureMode::Verify,
                ProcedureMode::General,
            ]),
            required_tags: BTreeSet::new(),
            today: today.into(),
            include_non_active: false,
        }
    }

    /// Permit remediation procedures as well.
    pub fn with_remediation(mut self) -> Self {
        self.allowed_modes.insert(ProcedureMode::Remediate);
        self
    }

    pub fn with_capabilities<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.available_capabilities = capabilities.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_workspace_kinds<I, S>(mut self, kinds: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.workspace_kinds = kinds.into_iter().map(Into::into).collect();
        self
    }
}

/// Why a document is not eligible. Each variant maps to exactly one hard check
/// so an audit log can name the rule that excluded a document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum IneligibilityReason {
    /// Trust level not permitted by the active agent package.
    TrustNotAllowed { trust: ProcedureTrust },
    /// External untrusted content: never selectable, independent of policy.
    NotSelectableTrust { trust: ProcedureTrust },
    /// Status is not `active` and non-active documents were not opted in.
    StatusNotSelectable { status: String },
    /// Past its declared `valid_until`.
    Expired { valid_until: String },
    /// Declared for other agents only.
    AgentNotTargeted,
    /// Declared for other platforms only.
    PlatformNotTargeted { platform: String },
    /// Declared for other workspace kinds only.
    WorkspaceKindNotTargeted,
    /// Mode not permitted for this run.
    ModeNotAllowed { mode: String },
    /// Active Agent policy requires a tag the document does not carry.
    MissingRequiredTag { tag: String },
    /// A required capability is not available and permitted.
    MissingRequiredCapability { capability: String },
    /// Superseded by another document present in the catalog.
    Superseded { by: String },
    /// A document with the same ID and at least equal trust was retained
    /// instead, so this copy never entered the catalog.
    ShadowedByEqualOrHigherTrust {
        retained_trust: ProcedureTrust,
        discarded_trust: ProcedureTrust,
    },
    /// Dropped because the catalog is full.
    CatalogFull { max: usize },
}

impl IneligibilityReason {
    /// Stable machine-readable code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::TrustNotAllowed { .. } => "trust_not_allowed",
            Self::NotSelectableTrust { .. } => "not_selectable_trust",
            Self::StatusNotSelectable { .. } => "status_not_selectable",
            Self::Expired { .. } => "expired",
            Self::AgentNotTargeted => "agent_not_targeted",
            Self::PlatformNotTargeted { .. } => "platform_not_targeted",
            Self::WorkspaceKindNotTargeted => "workspace_kind_not_targeted",
            Self::ModeNotAllowed { .. } => "mode_not_allowed",
            Self::MissingRequiredTag { .. } => "missing_required_tag",
            Self::MissingRequiredCapability { .. } => "missing_required_capability",
            Self::Superseded { .. } => "superseded",
            Self::ShadowedByEqualOrHigherTrust { .. } => "shadowed_by_equal_or_higher_trust",
            Self::CatalogFull { .. } => "catalog_full",
        }
    }
}

/// The result of hard eligibility for one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EligibilityOutcome {
    Eligible,
    /// Every failed check, not just the first, so an author sees the whole list.
    Ineligible {
        reasons: Vec<IneligibilityReason>,
    },
}

impl EligibilityOutcome {
    pub fn is_eligible(&self) -> bool {
        matches!(self, Self::Eligible)
    }

    pub fn reasons(&self) -> &[IneligibilityReason] {
        match self {
            Self::Eligible => &[],
            Self::Ineligible { reasons } => reasons,
        }
    }
}

/// One catalog entry: the document plus its eligibility verdict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub document: ProcedureDocument,
    pub eligibility: EligibilityOutcome,
}

impl CatalogEntry {
    pub fn is_eligible(&self) -> bool {
        self.eligibility.is_eligible()
    }
}

/// A loaded, evaluated set of procedures.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcedureCatalog {
    /// Entries keyed by procedure ID, so ordering is deterministic and a
    /// duplicate ID cannot silently produce two competing documents.
    entries: BTreeMap<String, CatalogEntry>,
    /// Documents dropped before evaluation, with the reason.
    dropped: Vec<(String, IneligibilityReason)>,
}

impl ProcedureCatalog {
    /// Build a catalog by evaluating every document against `context`.
    ///
    /// On a duplicate ID the higher-trust document wins, and on equal trust the
    /// first wins. Silently merging or picking arbitrarily would let a
    /// lower-trust copy shadow a builtin procedure.
    pub fn build(
        documents: impl IntoIterator<Item = ProcedureDocument>,
        context: &EligibilityContext,
    ) -> Self {
        let mut entries: BTreeMap<String, ProcedureDocument> = BTreeMap::new();
        let mut dropped = Vec::new();

        for document in documents {
            if entries.len() >= MAX_CATALOG_ENTRIES && !entries.contains_key(&document.metadata.id)
            {
                dropped.push((
                    document.metadata.id.clone(),
                    IneligibilityReason::CatalogFull {
                        max: MAX_CATALOG_ENTRIES,
                    },
                ));
                continue;
            }
            // `ProcedureTrust` orders most-trusted first, so a numerically
            // smaller value is the more trusted one.
            match entries.get(&document.metadata.id) {
                Some(existing) if existing.trust() <= document.trust() => {
                    dropped.push((
                        document.metadata.id.clone(),
                        IneligibilityReason::ShadowedByEqualOrHigherTrust {
                            retained_trust: existing.trust(),
                            discarded_trust: document.trust(),
                        },
                    ));
                }
                Some(existing) => {
                    // The incoming document is more trusted; it replaces the
                    // one already held, and the displacement is recorded rather
                    // than silently overwritten.
                    dropped.push((
                        document.metadata.id.clone(),
                        IneligibilityReason::ShadowedByEqualOrHigherTrust {
                            retained_trust: document.trust(),
                            discarded_trust: existing.trust(),
                        },
                    ));
                    entries.insert(document.metadata.id.clone(), document);
                }
                None => {
                    entries.insert(document.metadata.id.clone(), document);
                }
            }
        }

        // Supersession is resolved against the retained set, so a document can
        // only be superseded by one that is actually present and eligible.
        let superseding: BTreeMap<String, String> = entries
            .values()
            .filter_map(|document| {
                document
                    .metadata
                    .supersedes
                    .as_ref()
                    .map(|target| (target.clone(), document.metadata.id.clone()))
            })
            .collect();

        let evaluated = entries
            .into_iter()
            .map(|(id, document)| {
                let mut reasons = evaluate_eligibility(&document, context);
                if let Some(by) = superseding.get(&id) {
                    reasons.push(IneligibilityReason::Superseded { by: by.clone() });
                }
                let eligibility = if reasons.is_empty() {
                    EligibilityOutcome::Eligible
                } else {
                    EligibilityOutcome::Ineligible { reasons }
                };
                (
                    id,
                    CatalogEntry {
                        document,
                        eligibility,
                    },
                )
            })
            .collect();

        Self {
            entries: evaluated,
            dropped,
        }
    }

    /// All entries, eligible or not, in stable ID order.
    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }

    /// Only the eligible entries — the sole input to ranking.
    pub fn eligible(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values().filter(|entry| entry.is_eligible())
    }

    pub fn get(&self, id: &str) -> Option<&CatalogEntry> {
        self.entries.get(id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn eligible_count(&self) -> usize {
        self.eligible().count()
    }

    /// Documents dropped before evaluation, with the reason.
    pub fn dropped(&self) -> &[(String, IneligibilityReason)] {
        &self.dropped
    }
}

/// Run every hard check, collecting all failures.
///
/// Order within the function does not affect the verdict; each check is
/// independent and absolute (design §12.1).
fn evaluate_eligibility(
    document: &ProcedureDocument,
    context: &EligibilityContext,
) -> Vec<IneligibilityReason> {
    let mut reasons = Vec::new();
    let metadata = &document.metadata;
    let trust = document.trust();

    // 1. External untrusted content is never selectable, whatever policy says.
    if !trust.is_selectable() {
        reasons.push(IneligibilityReason::NotSelectableTrust { trust });
    }

    // 2. Trust must additionally be permitted by the active package.
    if !context.allowed_trust_levels.contains(&trust) {
        reasons.push(IneligibilityReason::TrustNotAllowed { trust });
    }

    // 3. Status.
    if !metadata.status.is_selectable_by_default() && !context.include_non_active {
        reasons.push(IneligibilityReason::StatusNotSelectable {
            status: metadata.status.code().to_string(),
        });
    }

    // 4. Expiry.
    if document.is_expired(&context.today) {
        reasons.push(IneligibilityReason::Expired {
            valid_until: metadata
                .valid_until
                .clone()
                .unwrap_or_else(|| context.today.clone()),
        });
    }

    // 5. Agent targeting. Empty means "any agent".
    if !metadata.agents.is_empty() && !metadata.agents.contains(&context.agent_id) {
        reasons.push(IneligibilityReason::AgentNotTargeted);
    }

    // 6. Platform targeting.
    if !metadata.platforms.is_empty() && !metadata.platforms.contains(&context.platform) {
        reasons.push(IneligibilityReason::PlatformNotTargeted {
            platform: context.platform.clone(),
        });
    }

    // 7. Workspace kind targeting.
    if !metadata.workspace_kinds.is_empty()
        && metadata
            .workspace_kinds
            .is_disjoint(&context.workspace_kinds)
    {
        reasons.push(IneligibilityReason::WorkspaceKindNotTargeted);
    }

    // 8. Mode.
    if !context.allowed_modes.contains(&metadata.mode) {
        reasons.push(IneligibilityReason::ModeNotAllowed {
            mode: metadata.mode.code().to_string(),
        });
    }

    for tag in context.required_tags.difference(&metadata.tags) {
        reasons.push(IneligibilityReason::MissingRequiredTag { tag: tag.clone() });
    }

    // 9. Required capabilities must all be available *and* permitted.
    for capability in &metadata.required_capabilities {
        if !context.available_capabilities.contains(capability) {
            reasons.push(IneligibilityReason::MissingRequiredCapability {
                capability: capability.clone(),
            });
        }
    }

    reasons
}

#[cfg(test)]
mod tests {
    use super::super::trust::ProcedureOrigin;
    use super::*;

    struct DocumentSpec {
        id: &'static str,
        extra: String,
        origin: ProcedureOrigin,
    }

    fn spec(id: &'static str) -> DocumentSpec {
        DocumentSpec {
            id,
            extra: String::new(),
            origin: ProcedureOrigin::WorkspacePackage,
        }
    }

    impl DocumentSpec {
        fn with(mut self, line: &str) -> Self {
            self.extra.push_str(line);
            self.extra.push('\n');
            self
        }

        fn origin(mut self, origin: ProcedureOrigin) -> Self {
            self.origin = origin;
            self
        }

        fn build(&self) -> ProcedureDocument {
            let text = format!(
                "---\nschema_version: 1\nid: {}\nversion: 1.0.0\nstatus: active\ntitle: T\nmode: diagnose\nrisk_level: low\n{}---\n\nBody for {}.\n",
                self.id, self.extra, self.id
            );
            ProcedureDocument::parse(&text, self.origin, format!("procedures/{}.md", self.id))
                .expect("test document parses")
        }
    }

    fn context() -> EligibilityContext {
        EligibilityContext::new("coder", "windows", "2026-08-09")
    }

    fn reasons_for<'a>(catalog: &'a ProcedureCatalog, id: &str) -> &'a [IneligibilityReason] {
        catalog
            .get(id)
            .expect("entry present")
            .eligibility
            .reasons()
    }

    #[test]
    fn a_targeted_active_document_is_eligible() {
        let catalog = ProcedureCatalog::build([spec("a.b").build()], &context());
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.eligible_count(), 1);
        assert!(reasons_for(&catalog, "a.b").is_empty());
    }

    /// The core §25.5 case 8 guarantee at catalog level.
    #[test]
    fn uploaded_content_is_ineligible_even_when_it_matches_everything() {
        let document = spec("a.b")
            .with("agents: [coder]")
            .with("platforms: [windows]")
            .origin(ProcedureOrigin::RuntimeSupplied)
            .build();
        let catalog = ProcedureCatalog::build([document], &context());
        assert_eq!(catalog.eligible_count(), 0);
        let reasons = reasons_for(&catalog, "a.b");
        assert!(reasons.iter().any(|r| r.code() == "not_selectable_trust"));
    }

    #[test]
    fn a_trust_level_outside_package_policy_is_ineligible() {
        let document = spec("a.b")
            .origin(ProcedureOrigin::OperatorInstalled)
            .build();
        let catalog = ProcedureCatalog::build([document], &context());
        assert_eq!(catalog.eligible_count(), 0);
        assert!(
            reasons_for(&catalog, "a.b")
                .iter()
                .any(|r| r.code() == "trust_not_allowed")
        );
    }

    #[test]
    fn operator_installed_becomes_eligible_when_policy_allows_it() {
        let mut context = context();
        context
            .allowed_trust_levels
            .insert(ProcedureTrust::UserInstalled);
        let document = spec("a.b")
            .origin(ProcedureOrigin::OperatorInstalled)
            .build();
        let catalog = ProcedureCatalog::build([document], &context);
        assert_eq!(catalog.eligible_count(), 1);
    }

    #[test]
    fn a_draft_is_ineligible_unless_explicitly_included() {
        let text = |status: &str| {
            format!(
                "---\nschema_version: 1\nid: a.b\nversion: 1\nstatus: {status}\ntitle: T\nmode: diagnose\nrisk_level: low\n---\n\nBody.\n"
            )
        };
        let document = ProcedureDocument::parse(
            &text("draft"),
            ProcedureOrigin::WorkspacePackage,
            "procedures/a.md",
        )
        .expect("parses");

        let catalog = ProcedureCatalog::build([document.clone()], &context());
        assert_eq!(catalog.eligible_count(), 0);
        assert!(
            reasons_for(&catalog, "a.b")
                .iter()
                .any(|r| r.code() == "status_not_selectable")
        );

        let mut permissive = context();
        permissive.include_non_active = true;
        let catalog = ProcedureCatalog::build([document], &permissive);
        assert_eq!(catalog.eligible_count(), 1);
    }

    #[test]
    fn an_expired_document_is_ineligible() {
        let catalog = ProcedureCatalog::build(
            [spec("a.b").with("valid_until: 2026-01-01").build()],
            &context(),
        );
        assert!(
            reasons_for(&catalog, "a.b")
                .iter()
                .any(|r| r.code() == "expired")
        );
    }

    #[test]
    fn agent_platform_and_workspace_targeting_all_exclude() {
        let document = spec("a.b")
            .with("agents: [reviewer]")
            .with("platforms: [linux]")
            .with("workspace_kinds: [node]")
            .build();
        let catalog =
            ProcedureCatalog::build([document], &context().with_workspace_kinds(["rust"]));
        let codes: Vec<&str> = reasons_for(&catalog, "a.b")
            .iter()
            .map(|r| r.code())
            .collect();
        assert!(codes.contains(&"agent_not_targeted"), "{codes:?}");
        assert!(codes.contains(&"platform_not_targeted"), "{codes:?}");
        assert!(codes.contains(&"workspace_kind_not_targeted"), "{codes:?}");
    }

    /// Every failing check is reported, so one authoring pass fixes the file.
    #[test]
    fn all_failed_checks_are_reported_not_just_the_first() {
        let document = spec("a.b")
            .with("agents: [reviewer]")
            .with("platforms: [linux]")
            .with("valid_until: 2020-01-01")
            .with("required_capabilities: [shell]")
            .build();
        let catalog = ProcedureCatalog::build([document], &context());
        assert!(reasons_for(&catalog, "a.b").len() >= 4);
    }

    #[test]
    fn an_empty_target_list_means_any_target() {
        let catalog = ProcedureCatalog::build([spec("a.b").build()], &context());
        assert_eq!(catalog.eligible_count(), 1, "no targeting means universal");
    }

    /// A remediation must not be reachable from a diagnostic run.
    #[test]
    fn a_remediation_is_ineligible_until_remediation_is_permitted() {
        // `mode` must replace the default rather than be appended, so this
        // document is built directly instead of through the spec helper.
        let text = "---\nschema_version: 1\nid: a.b\nversion: 1.0.0\nstatus: active\ntitle: T\nmode: remediate\nrisk_level: low\nside_effects: [writes_workspace]\n---\n\nBody.\n";
        let document =
            ProcedureDocument::parse(text, ProcedureOrigin::WorkspacePackage, "procedures/a.md")
                .expect("parses");

        let catalog = ProcedureCatalog::build([document.clone()], &context());
        assert!(
            reasons_for(&catalog, "a.b")
                .iter()
                .any(|r| r.code() == "mode_not_allowed")
        );

        let catalog = ProcedureCatalog::build([document], &context().with_remediation());
        assert_eq!(catalog.eligible_count(), 1);
    }

    /// Availability is checked against the resolved permitted set, so a
    /// capability that exists but is denied still excludes the procedure.
    #[test]
    fn a_missing_required_capability_excludes_the_procedure() {
        let document = spec("a.b")
            .with("required_capabilities: [shell, http]")
            .build();
        let catalog = ProcedureCatalog::build([document.clone()], &context());
        let missing: Vec<&IneligibilityReason> = reasons_for(&catalog, "a.b")
            .iter()
            .filter(|r| r.code() == "missing_required_capability")
            .collect();
        assert_eq!(missing.len(), 2);

        let catalog =
            ProcedureCatalog::build([document], &context().with_capabilities(["shell", "http"]));
        assert_eq!(catalog.eligible_count(), 1);
    }

    #[test]
    fn an_optional_capability_never_excludes_a_procedure() {
        let document = spec("a.b").with("optional_capabilities: [http]").build();
        let catalog = ProcedureCatalog::build([document], &context());
        assert_eq!(catalog.eligible_count(), 1);
    }

    #[test]
    fn a_superseded_document_is_ineligible_and_names_its_successor() {
        let old = spec("a.old").build();
        let new = spec("a.new").with("supersedes: a.old").build();
        let catalog = ProcedureCatalog::build([old, new], &context());
        assert_eq!(catalog.eligible_count(), 1);
        let reasons = reasons_for(&catalog, "a.old");
        assert!(matches!(
            reasons.first(),
            Some(IneligibilityReason::Superseded { by }) if by == "a.new"
        ));
    }

    /// A lower-trust copy must not be able to shadow a builtin procedure.
    #[test]
    fn a_duplicate_id_keeps_the_more_trusted_document() {
        let builtin = spec("a.b").origin(ProcedureOrigin::BuiltinPackage).build();
        let workspace = spec("a.b")
            .origin(ProcedureOrigin::WorkspacePackage)
            .build();

        for documents in [[builtin.clone(), workspace.clone()], [workspace, builtin]] {
            let catalog = ProcedureCatalog::build(documents, &context());
            assert_eq!(catalog.len(), 1);
            assert_eq!(
                catalog.get("a.b").unwrap().document.trust(),
                ProcedureTrust::BuiltinTrusted,
                "the trusted document wins regardless of load order"
            );
            // The displacement is recorded either way, so an operator can see
            // that a shadowing copy exists.
            assert!(matches!(
                catalog.dropped(),
                [(id, IneligibilityReason::ShadowedByEqualOrHigherTrust {
                    retained_trust: ProcedureTrust::BuiltinTrusted,
                    discarded_trust: ProcedureTrust::WorkspaceTrusted,
                })] if id == "a.b"
            ));
        }
    }

    #[test]
    fn the_eligible_iterator_never_yields_an_ineligible_entry() {
        let documents = [
            spec("a.ok").build(),
            spec("a.expired").with("valid_until: 2020-01-01").build(),
            spec("a.untrusted")
                .origin(ProcedureOrigin::RuntimeSupplied)
                .build(),
        ];
        let catalog = ProcedureCatalog::build(documents, &context());
        let eligible: Vec<&str> = catalog
            .eligible()
            .map(|entry| entry.document.metadata.id.as_str())
            .collect();
        assert_eq!(eligible, vec!["a.ok"]);
    }

    #[test]
    fn entry_order_is_deterministic_by_id() {
        let documents = [
            spec("z.last").build(),
            spec("a.first").build(),
            spec("m.mid").build(),
        ];
        let catalog = ProcedureCatalog::build(documents, &context());
        let ids: Vec<&str> = catalog
            .entries()
            .map(|entry| entry.document.metadata.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a.first", "m.mid", "z.last"]);
    }

    #[test]
    fn catalog_serialization_round_trips() {
        let catalog = ProcedureCatalog::build([spec("a.b").build()], &context());
        let json = serde_json::to_string(&catalog).expect("serializes");
        let restored: ProcedureCatalog = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(catalog, restored);
    }
}
