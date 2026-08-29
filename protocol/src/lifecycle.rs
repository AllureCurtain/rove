//! Lifecycle vocabulary that crosses the wire: run status, approval policy and
//! decision, and the host-selected execution mode.
//!
//! Every variant here is serialized as `snake_case` and is part of the public
//! protocol. Renaming one is a breaking change; see [`crate::version`].

use serde::{Deserialize, Serialize};

/// Current status of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Init,
    Running,
    Done,
    Error,
    Cancelled,
    Interrupted,
}

/// Tool approval policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Ask,
    Auto,
    Never,
}

/// Execution profile selected by the host before a run starts.
///
/// Review is deliberately a runtime-owned mode rather than a prompt hint. It
/// is carried into every tool invocation and checked again at dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    #[default]
    Normal,
    Review,
}

/// A concrete approval decision supplied by an interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Reject,
}

#[cfg(test)]
mod tests {
    use super::{ApprovalDecision, ApprovalPolicy, RunMode, RunStatus};

    /// The wire spellings are pinned here rather than left to the derive, so a
    /// rename that would silently break a persisted artifact or a live client
    /// fails in this crate first.
    #[test]
    fn lifecycle_variants_keep_their_published_wire_spellings() {
        assert_eq!(json(&RunStatus::Init), "\"init\"");
        assert_eq!(json(&RunStatus::Running), "\"running\"");
        assert_eq!(json(&RunStatus::Done), "\"done\"");
        assert_eq!(json(&RunStatus::Error), "\"error\"");
        assert_eq!(json(&RunStatus::Cancelled), "\"cancelled\"");
        assert_eq!(json(&RunStatus::Interrupted), "\"interrupted\"");

        assert_eq!(json(&ApprovalPolicy::Ask), "\"ask\"");
        assert_eq!(json(&ApprovalPolicy::Auto), "\"auto\"");
        assert_eq!(json(&ApprovalPolicy::Never), "\"never\"");

        assert_eq!(json(&RunMode::Normal), "\"normal\"");
        assert_eq!(json(&RunMode::Review), "\"review\"");

        assert_eq!(json(&ApprovalDecision::Approve), "\"approve\"");
        assert_eq!(json(&ApprovalDecision::Reject), "\"reject\"");
    }

    #[test]
    fn run_mode_defaults_to_normal_so_an_absent_field_never_grants_review() {
        assert_eq!(RunMode::default(), RunMode::Normal);
    }

    fn json<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_string(value).unwrap()
    }
}
