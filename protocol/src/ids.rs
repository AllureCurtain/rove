//! ULID-backed identifiers shared by every Rove surface.
//!
//! These live in the protocol crate rather than in `rove-runtime` because they
//! appear in persisted artifacts, HTTP paths, and SSE payloads. A consumer that
//! only parses a run id should not have to link an async runtime to do it.
//!
//! `rove-runtime` and `rove-core` re-export these under their historic paths,
//! so call sites keep importing `rove_runtime::types::SessionId`.

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Declares a ULID newtype together with the conversions every Rove identifier
/// is expected to support: fresh generation, `Display`, and `FromStr`.
///
/// The wire form is the bare ULID string, because `Ulid`'s own `Serialize`
/// impl is transparent and these are `#[repr(transparent)]`-style newtypes.
macro_rules! protocol_id {
    ($(#[$meta:meta])* $id:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $id(pub Ulid);

        impl $id {
            /// Generates a new, monotonically sortable identifier.
            pub fn new() -> Self {
                Self(Ulid::new())
            }
        }

        impl Default for $id {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $id {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl std::str::FromStr for $id {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Ulid::from_string(value)
                    .map(Self)
                    .map_err(|error| error.to_string())
            }
        }
    };
}

protocol_id! {
    /// Unique identifier for a session (user-level, spans multiple jobs).
    SessionId
}

protocol_id! {
    /// Unique identifier for a job (one task submission).
    JobId
}

protocol_id! {
    /// Unique identifier for a single engine run (one main-loop execution).
    RunId
}

protocol_id! {
    /// Unique identity for one tool invocation.
    CallId
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{CallId, JobId, RunId, SessionId};

    #[test]
    fn an_identifier_serializes_as_a_bare_ulid_string() {
        let id = RunId::new();

        let encoded = serde_json::to_value(id).unwrap();

        assert_eq!(encoded, serde_json::Value::String(id.to_string()));
    }

    #[test]
    fn parsing_round_trips_the_displayed_form() {
        for rendered in [
            SessionId::new().to_string(),
            JobId::new().to_string(),
            RunId::new().to_string(),
            CallId::new().to_string(),
        ] {
            assert_eq!(
                SessionId::from_str(&rendered).unwrap().to_string(),
                rendered
            );
        }
    }

    #[test]
    fn parsing_a_non_ulid_reports_the_reason_instead_of_panicking() {
        let error = SessionId::from_str("not-a-ulid").unwrap_err();

        assert!(!error.is_empty(), "expected a described failure");
    }

    #[test]
    fn distinct_identifier_types_do_not_collide_when_freshly_generated() {
        assert_ne!(RunId::new(), RunId::new());
    }
}
