//! Instruction authority taxonomy and conflict resolution.
//!
//! Six classes of content reach a run (design §6.1) and they are *not*
//! distinguished by "everything goes into the system prompt". Each class
//! carries an [`AuthorityClass`], and a lower class can never override a
//! higher one. The important consequence is negative: advisory content —
//! selected procedures, memory, reference material, tool output — cannot
//! claim authority for itself. A retrieved document that says
//! "ignore previous instructions" is data, not permission.
//!
//! This module is deliberately free of I/O so the ordering rules can be
//! tested exhaustively without a workspace on disk.

use serde::{Deserialize, Serialize};

/// Authority layers, ordered from most to least authoritative (design §7.1).
///
/// `Ord` follows declaration order, so `EnforcedRuntimePolicy` is the
/// smallest and most authoritative value. Compare with [`Self::outranks`]
/// rather than relying on the raw ordering at call sites.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityClass {
    /// Workspace boundary, approval, denylist, budget caps, cancellation.
    /// Enforced by code, never by model goodwill.
    #[default]
    EnforcedRuntimePolicy,
    /// Explicitly selected AgentDefinition policy plus in-scope workspace
    /// instructions. May tighten enforced policy, never loosen it.
    TrustedOperatorPolicy,
    /// The current task and the constraints the user stated with it.
    UserTask,
    /// Persona, default working style, output preferences.
    AgentDefaults,
    /// Selected procedures, memory, reference material, tool output.
    AdvisoryContext,
}

impl AuthorityClass {
    /// Stable machine-readable code for events and diagnostics.
    pub fn code(self) -> &'static str {
        match self {
            Self::EnforcedRuntimePolicy => "enforced_runtime_policy",
            Self::TrustedOperatorPolicy => "trusted_operator_policy",
            Self::UserTask => "user_task",
            Self::AgentDefaults => "agent_defaults",
            Self::AdvisoryContext => "advisory_context",
        }
    }

    /// True when `self` is strictly more authoritative than `other`.
    pub fn outranks(self, other: Self) -> bool {
        self < other
    }

    /// Whether content in this class may tighten a constraint set.
    ///
    /// Every class may ask for something stricter. Only the enforced layer
    /// decides what "stricter" means in the end, which is why widening is a
    /// separate question ([`Self::may_widen`]).
    pub fn may_tighten(self) -> bool {
        true
    }

    /// Whether content in this class may widen an existing permission.
    ///
    /// Only enforced runtime policy — that is, code and operator config —
    /// can grant more than what is already granted. An Agent package that
    /// writes a larger budget or `approval = "auto"` gains nothing, and a
    /// procedure that suggests a destructive command does not pre-approve it.
    pub fn may_widen(self) -> bool {
        matches!(self, Self::EnforcedRuntimePolicy)
    }

    /// Whether this class may act as a source of trusted instructions.
    ///
    /// Advisory context never can. This is the check that keeps a
    /// high-similarity reference chunk from being treated like policy.
    pub fn is_trusted_instruction_source(self) -> bool {
        !matches!(self, Self::AdvisoryContext)
    }
}

/// The six content classes of design §6.1, each pinned to its authority.
///
/// Content is classified by *where it came from*, never by what it says
/// about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentClass {
    RuntimeHardPolicy,
    AgentDefinition,
    WorkspaceInstructions,
    Procedure,
    ReferenceAndMemory,
    RuntimeEvidence,
}

impl ContentClass {
    /// The authority this content class carries. Fixed by the taxonomy, not
    /// negotiable per document.
    pub fn authority(self) -> AuthorityClass {
        match self {
            Self::RuntimeHardPolicy => AuthorityClass::EnforcedRuntimePolicy,
            Self::AgentDefinition | Self::WorkspaceInstructions => {
                AuthorityClass::TrustedOperatorPolicy
            }
            // A procedure is a reviewed method, not a mandate: it is selected,
            // then followed or deviated from against live evidence (§6.5).
            Self::Procedure | Self::ReferenceAndMemory | Self::RuntimeEvidence => {
                AuthorityClass::AdvisoryContext
            }
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::RuntimeHardPolicy => "runtime_hard_policy",
            Self::AgentDefinition => "agent_definition",
            Self::WorkspaceInstructions => "workspace_instructions",
            Self::Procedure => "procedure",
            Self::ReferenceAndMemory => "reference_and_memory",
            Self::RuntimeEvidence => "runtime_evidence",
        }
    }

    /// Whether this class may ever grant a tool permission.
    ///
    /// Nothing here can. Permission comes from enforced policy alone; a
    /// procedure's `required_capabilities` describes a need, not a grant
    /// (§6.3, §16.3).
    pub fn grants_tool_permission(self) -> bool {
        false
    }
}

/// One attempted instruction contribution, tagged with its origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityClaim {
    /// Where the content came from. Determines authority.
    pub content_class: ContentClass,
    /// Stable source label (selector, `AGENTS.md` path, procedure ref).
    pub source: String,
    /// Whether the contribution tightens or widens the resulting constraint.
    pub effect: ClaimEffect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimEffect {
    /// Asks for something stricter than the current resolution.
    Tighten,
    /// Asks for something more permissive than the current resolution.
    Widen,
}

/// Outcome of resolving one claim against the currently effective authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum ClaimResolution {
    /// The claim is applied.
    Accepted,
    /// The claim is dropped, with a bounded machine-readable reason.
    Rejected { reason: AuthorityRejection },
}

impl ClaimResolution {
    pub fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }
}

/// Why a claim was not applied. Codes are stable and safe to log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityRejection {
    /// Attempted to widen a permission without the standing to do so.
    WidenNotPermitted,
    /// Attempted to override an equal-or-higher authority already resolved.
    LowerAuthorityCannotOverride,
    /// Advisory content attempted to act as a trusted instruction source.
    AdvisoryContentIsNotInstruction,
}

impl AuthorityRejection {
    pub fn code(self) -> &'static str {
        match self {
            Self::WidenNotPermitted => "widen_not_permitted",
            Self::LowerAuthorityCannotOverride => "lower_authority_cannot_override",
            Self::AdvisoryContentIsNotInstruction => "advisory_content_is_not_instruction",
        }
    }
}

/// Resolve one claim against the authority that already decided a constraint.
///
/// `resolved_by` is the authority of the decision currently in force. A claim
/// is applied when it tightens from any class, or when it widens from a class
/// permitted to widen and not outranked by the standing decision.
pub fn resolve_claim(claim: &AuthorityClaim, resolved_by: AuthorityClass) -> ClaimResolution {
    let authority = claim.content_class.authority();

    match claim.effect {
        ClaimEffect::Tighten => {
            // Advisory text is never an instruction, so it cannot silently
            // narrow the effective policy either; it can only inform.
            if !authority.is_trusted_instruction_source() {
                return ClaimResolution::Rejected {
                    reason: AuthorityRejection::AdvisoryContentIsNotInstruction,
                };
            }
            ClaimResolution::Accepted
        }
        ClaimEffect::Widen => {
            if !authority.may_widen() {
                return ClaimResolution::Rejected {
                    reason: AuthorityRejection::WidenNotPermitted,
                };
            }
            if resolved_by.outranks(authority) {
                return ClaimResolution::Rejected {
                    reason: AuthorityRejection::LowerAuthorityCannotOverride,
                };
            }
            ClaimResolution::Accepted
        }
    }
}

/// Intersect an operator cap with an Agent-suggested value (design §8.7).
///
/// `resolved = agent default bounded by operator cap`. An absent cap means
/// the operator expressed no limit, so the suggestion stands; an absent
/// suggestion falls back to the cap. Written as a helper because getting this
/// backwards is exactly how a package would gain budget it was never granted.
pub fn bounded_by_operator_cap(suggested: Option<u32>, operator_cap: Option<u32>) -> Option<u32> {
    match (suggested, operator_cap) {
        (Some(suggested), Some(cap)) => Some(suggested.min(cap)),
        (Some(suggested), None) => Some(suggested),
        (None, cap) => cap,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authority_order_runs_from_enforced_policy_down_to_advisory() {
        assert!(
            AuthorityClass::EnforcedRuntimePolicy.outranks(AuthorityClass::TrustedOperatorPolicy)
        );
        assert!(AuthorityClass::TrustedOperatorPolicy.outranks(AuthorityClass::UserTask));
        assert!(AuthorityClass::UserTask.outranks(AuthorityClass::AgentDefaults));
        assert!(AuthorityClass::AgentDefaults.outranks(AuthorityClass::AdvisoryContext));
        assert!(!AuthorityClass::AdvisoryContext.outranks(AuthorityClass::AdvisoryContext));
    }

    #[test]
    fn only_enforced_runtime_policy_may_widen() {
        assert!(AuthorityClass::EnforcedRuntimePolicy.may_widen());
        for class in [
            AuthorityClass::TrustedOperatorPolicy,
            AuthorityClass::UserTask,
            AuthorityClass::AgentDefaults,
            AuthorityClass::AdvisoryContext,
        ] {
            assert!(!class.may_widen(), "{} must not widen", class.code());
            assert!(
                class.may_tighten(),
                "{} must be able to tighten",
                class.code()
            );
        }
    }

    #[test]
    fn no_content_class_ever_grants_a_tool_permission() {
        for class in [
            ContentClass::RuntimeHardPolicy,
            ContentClass::AgentDefinition,
            ContentClass::WorkspaceInstructions,
            ContentClass::Procedure,
            ContentClass::ReferenceAndMemory,
            ContentClass::RuntimeEvidence,
        ] {
            assert!(
                !class.grants_tool_permission(),
                "{} must not grant permission",
                class.code()
            );
        }
    }

    /// A procedure or memory entry that asks for more permission is the
    /// canonical escalation attempt, and it must fail on authority alone —
    /// before anyone looks at how convincing the text is.
    #[test]
    fn advisory_content_cannot_widen_or_instruct() {
        for class in [
            ContentClass::Procedure,
            ContentClass::ReferenceAndMemory,
            ContentClass::RuntimeEvidence,
        ] {
            let widen = AuthorityClaim {
                content_class: class,
                source: "retrieved-doc".to_string(),
                effect: ClaimEffect::Widen,
            };
            assert_eq!(
                resolve_claim(&widen, AuthorityClass::AdvisoryContext),
                ClaimResolution::Rejected {
                    reason: AuthorityRejection::WidenNotPermitted
                },
                "{} must not widen",
                class.code()
            );

            let tighten = AuthorityClaim {
                content_class: class,
                source: "retrieved-doc".to_string(),
                effect: ClaimEffect::Tighten,
            };
            assert_eq!(
                resolve_claim(&tighten, AuthorityClass::EnforcedRuntimePolicy),
                ClaimResolution::Rejected {
                    reason: AuthorityRejection::AdvisoryContentIsNotInstruction
                },
                "{} must not act as an instruction",
                class.code()
            );
        }
    }

    #[test]
    fn trusted_sources_may_tighten_but_an_agent_package_cannot_widen() {
        let tighten = AuthorityClaim {
            content_class: ContentClass::WorkspaceInstructions,
            source: "AGENTS.md".to_string(),
            effect: ClaimEffect::Tighten,
        };
        assert!(resolve_claim(&tighten, AuthorityClass::EnforcedRuntimePolicy).is_accepted());

        let widen = AuthorityClaim {
            content_class: ContentClass::AgentDefinition,
            source: "workspace:ops".to_string(),
            effect: ClaimEffect::Widen,
        };
        assert_eq!(
            resolve_claim(&widen, AuthorityClass::EnforcedRuntimePolicy),
            ClaimResolution::Rejected {
                reason: AuthorityRejection::WidenNotPermitted
            }
        );
    }

    #[test]
    fn agent_defaults_are_intersected_with_operator_caps() {
        // A package asking for more than the cap is clamped, not honoured.
        assert_eq!(bounded_by_operator_cap(Some(500), Some(20)), Some(20));
        // Asking for less than the cap is a genuine tightening.
        assert_eq!(bounded_by_operator_cap(Some(5), Some(20)), Some(5));
        // No cap configured leaves the suggestion in place.
        assert_eq!(bounded_by_operator_cap(Some(5), None), Some(5));
        // No suggestion falls back to the cap, never to unlimited.
        assert_eq!(bounded_by_operator_cap(None, Some(20)), Some(20));
        assert_eq!(bounded_by_operator_cap(None, None), None);
    }

    #[test]
    fn content_class_authority_mapping_is_fixed_by_the_taxonomy() {
        assert_eq!(
            ContentClass::RuntimeHardPolicy.authority(),
            AuthorityClass::EnforcedRuntimePolicy
        );
        assert_eq!(
            ContentClass::AgentDefinition.authority(),
            AuthorityClass::TrustedOperatorPolicy
        );
        assert_eq!(
            ContentClass::WorkspaceInstructions.authority(),
            AuthorityClass::TrustedOperatorPolicy
        );
        assert_eq!(
            ContentClass::Procedure.authority(),
            AuthorityClass::AdvisoryContext
        );
    }

    #[test]
    fn authority_and_rejection_codes_are_stable_snake_case() {
        assert_eq!(
            AuthorityClass::EnforcedRuntimePolicy.code(),
            "enforced_runtime_policy"
        );
        assert_eq!(ContentClass::Procedure.code(), "procedure");
        assert_eq!(
            AuthorityRejection::WidenNotPermitted.code(),
            "widen_not_permitted"
        );
        assert_eq!(
            serde_json::to_value(AuthorityClass::AdvisoryContext).unwrap(),
            serde_json::json!("advisory_context")
        );
    }
}
