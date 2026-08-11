//! Agent source selectors (`builtin:legacy`, `workspace:ops-diagnostic`).
//!
//! A selector always names both a source namespace and an ID, so two packages
//! that share an ID can never shadow each other by search order (design §8.4).
//! An ambiguous or unknown selector produces a diagnostic; it never silently
//! resolves to "whatever was found first".

use std::fmt;

use serde::{Deserialize, Serialize};

/// Longest accepted selector text. Bounded so a hostile config value cannot
/// push an unbounded string into diagnostics and events.
pub const MAX_SELECTOR_LEN: usize = 128;
/// Longest accepted agent ID within a selector.
pub const MAX_AGENT_ID_LEN: usize = 64;

/// Where an Agent package came from. Source determines trust; a package
/// cannot declare its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    /// Compiled into the runtime. Highest trust, not author-editable.
    Builtin,
    /// Tracked in the workspace under `agents/<id>/`.
    Workspace,
}

impl AgentSource {
    pub fn code(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Workspace => "workspace",
        }
    }

    /// Namespaces accepted in this phase. `user`/`remote` registries and
    /// signature verification are deliberately out of scope (design §8.4),
    /// so they are rejected rather than half-supported.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "builtin" => Some(Self::Builtin),
            "workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

impl fmt::Display for AgentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// A fully qualified `<source>:<agent-id>` reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AgentSelector {
    pub source: AgentSource,
    pub agent_id: String,
}

/// The built-in package that reproduces pre-AgentDefinition behaviour.
pub const LEGACY_AGENT_ID: &str = "legacy";

impl AgentSelector {
    /// The compatibility selector used when no Agent is configured.
    pub fn legacy() -> Self {
        Self {
            source: AgentSource::Builtin,
            agent_id: LEGACY_AGENT_ID.to_string(),
        }
    }

    pub fn is_legacy(&self) -> bool {
        self.source == AgentSource::Builtin && self.agent_id == LEGACY_AGENT_ID
    }

    /// Parse `<source>:<agent-id>`.
    ///
    /// The namespace is mandatory. A bare `default` is rejected rather than
    /// guessed at, because guessing is what allows silent shadowing.
    pub fn parse(value: &str) -> Result<Self, SelectorError> {
        if value.len() > MAX_SELECTOR_LEN {
            return Err(SelectorError::TooLong {
                len: value.len(),
                max: MAX_SELECTOR_LEN,
            });
        }

        let mut parts = value.splitn(2, ':');
        let source_text = parts.next().unwrap_or_default();
        let Some(agent_id) = parts.next() else {
            return Err(SelectorError::MissingSourceNamespace);
        };

        let source = AgentSource::parse(source_text).ok_or_else(|| {
            SelectorError::UnknownSourceNamespace {
                namespace: sanitize(source_text),
            }
        })?;
        validate_agent_id(agent_id)?;

        Ok(Self {
            source,
            agent_id: agent_id.to_string(),
        })
    }
}

impl fmt::Display for AgentSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source.code(), self.agent_id)
    }
}

/// An agent ID must be a lowercase path-safe token.
///
/// This is a security boundary, not a style rule: the ID becomes a directory
/// name under `agents/`, so `..`, separators, and absolute-path prefixes must
/// never survive parsing.
pub fn validate_agent_id(agent_id: &str) -> Result<(), SelectorError> {
    if agent_id.is_empty() {
        return Err(SelectorError::EmptyAgentId);
    }
    if agent_id.len() > MAX_AGENT_ID_LEN {
        return Err(SelectorError::TooLong {
            len: agent_id.len(),
            max: MAX_AGENT_ID_LEN,
        });
    }

    let valid = agent_id.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '_' | '.')
    });
    if !valid {
        return Err(SelectorError::InvalidAgentId {
            agent_id: sanitize(agent_id),
        });
    }
    // `.` is allowed inside an ID for namespacing, but a component that is
    // only dots is a traversal attempt.
    if agent_id.chars().all(|character| character == '.') {
        return Err(SelectorError::InvalidAgentId {
            agent_id: sanitize(agent_id),
        });
    }
    if agent_id.starts_with('.') || agent_id.ends_with('.') {
        return Err(SelectorError::InvalidAgentId {
            agent_id: sanitize(agent_id),
        });
    }
    Ok(())
}

/// Bound and strip untrusted text before it appears in an error or event.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(rename_all = "snake_case", tag = "error")]
pub enum SelectorError {
    #[error("agent selector must be '<source>:<agent-id>'")]
    MissingSourceNamespace,
    #[error("unknown agent source namespace '{namespace}'")]
    UnknownSourceNamespace { namespace: String },
    #[error("agent id must not be empty")]
    EmptyAgentId,
    #[error("agent id '{agent_id}' must be lowercase alphanumeric with '-', '_' or '.'")]
    InvalidAgentId { agent_id: String },
    #[error("agent selector is {len} bytes, over the {max} byte limit")]
    TooLong { len: usize, max: usize },
}

impl SelectorError {
    /// Stable machine-readable code for diagnostics.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingSourceNamespace => "missing_source_namespace",
            Self::UnknownSourceNamespace { .. } => "unknown_source_namespace",
            Self::EmptyAgentId => "empty_agent_id",
            Self::InvalidAgentId { .. } => "invalid_agent_id",
            Self::TooLong { .. } => "selector_too_long",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_qualified_selector_round_trips() {
        let selector = AgentSelector::parse("workspace:ops-diagnostic").unwrap();

        assert_eq!(selector.source, AgentSource::Workspace);
        assert_eq!(selector.agent_id, "ops-diagnostic");
        assert_eq!(selector.to_string(), "workspace:ops-diagnostic");
        assert!(!selector.is_legacy());
    }

    #[test]
    fn the_legacy_selector_is_builtin() {
        let legacy = AgentSelector::legacy();

        assert_eq!(legacy.to_string(), "builtin:legacy");
        assert!(legacy.is_legacy());
        assert_eq!(AgentSelector::parse("builtin:legacy").unwrap(), legacy);
    }

    /// A bare ID is exactly the shape that lets one package shadow another by
    /// search order, so it must not resolve at all.
    #[test]
    fn an_unqualified_selector_is_rejected_rather_than_guessed() {
        assert_eq!(
            AgentSelector::parse("default"),
            Err(SelectorError::MissingSourceNamespace)
        );
    }

    #[test]
    fn unimplemented_namespaces_are_rejected_not_silently_accepted() {
        // These are named in the design as later scope. Accepting them now
        // would claim support that does not exist.
        for value in ["future-user:research", "remote:downloaded", "user:mine"] {
            let error = AgentSelector::parse(value).unwrap_err();
            assert_eq!(error.code(), "unknown_source_namespace", "{value}");
        }
    }

    /// The agent ID becomes a directory name, so traversal must die in the
    /// parser rather than at the filesystem.
    #[test]
    fn traversal_and_separator_ids_are_rejected() {
        for value in [
            "workspace:..",
            "workspace:../../etc",
            "workspace:a/b",
            "workspace:a\\b",
            "workspace:/abs",
            "workspace:C:",
            "workspace:.hidden",
            "workspace:trailing.",
            "workspace:UPPER",
            "workspace:has space",
            "workspace:nul\0byte",
        ] {
            let Err(error) = AgentSelector::parse(value) else {
                panic!("{value} must not parse");
            };
            assert!(
                matches!(
                    error,
                    SelectorError::InvalidAgentId { .. } | SelectorError::EmptyAgentId
                ),
                "{value} produced {error:?}"
            );
        }
    }

    #[test]
    fn an_empty_agent_id_is_rejected() {
        assert_eq!(
            AgentSelector::parse("workspace:"),
            Err(SelectorError::EmptyAgentId)
        );
    }

    #[test]
    fn oversized_selectors_and_ids_are_bounded() {
        let long_id = "a".repeat(MAX_AGENT_ID_LEN + 1);
        assert!(matches!(
            AgentSelector::parse(&format!("workspace:{long_id}")),
            Err(SelectorError::TooLong { .. })
        ));

        let very_long = "b".repeat(MAX_SELECTOR_LEN + 1);
        assert!(matches!(
            AgentSelector::parse(&very_long),
            Err(SelectorError::TooLong { .. })
        ));
    }

    /// Error text is rendered into diagnostics, so untrusted input must not
    /// carry control characters or unbounded length into it.
    #[test]
    fn error_text_from_untrusted_input_is_sanitized_and_bounded() {
        let error = AgentSelector::parse(&format!("bad\u{7}ns:{}", "x")).unwrap_err();
        let rendered = error.to_string();

        assert!(
            !rendered.contains('\u{7}'),
            "control char survived: {rendered:?}"
        );

        let long_namespace = "n".repeat(100);
        let error = AgentSelector::parse(&format!("{long_namespace}:id")).unwrap_err();
        if let SelectorError::UnknownSourceNamespace { namespace } = error {
            assert!(
                namespace.len() <= 64,
                "namespace not bounded: {}",
                namespace.len()
            );
        } else {
            panic!("expected unknown namespace, got {error:?}");
        }
    }

    #[test]
    fn dotted_ids_are_allowed_for_namespacing() {
        let selector = AgentSelector::parse("workspace:ops.disk.triage").unwrap();
        assert_eq!(selector.agent_id, "ops.disk.triage");
    }
}
