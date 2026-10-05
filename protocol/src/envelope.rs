//! Versioned envelope for streamed events.
//!
//! Every SSE frame Rove writes carries the protocol version as its first field,
//! so a client can decide whether it understands the payload before it tries to
//! interpret the body:
//!
//! ```text
//! data: {"v":1,"type":"run_started","run_id":"01J…","job_id":"01J…", …}
//! ```
//!
//! The body is flattened rather than nested, which keeps the wire shape
//! backward compatible: a client written before versioning still finds `type`
//! and every payload field exactly where they were, and simply ignores `v`.

use serde::{Deserialize, Serialize};

use crate::version::{PROTOCOL_VERSION, protocol_version};

/// Wraps a payload with the protocol version.
///
/// `v` is declared first so serde emits it first; the flattened payload
/// follows. On the way in, `v` defaults to [`PROTOCOL_VERSION`] so a frame
/// recorded before the field existed still deserializes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Versioned<T> {
    #[serde(rename = "v", default = "protocol_version")]
    pub version: u32,
    #[serde(flatten)]
    pub payload: T,
}

impl<T> Versioned<T> {
    /// Stamps a payload with the current protocol version.
    pub fn now(payload: T) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            payload,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::Versioned;
    use crate::version::PROTOCOL_VERSION;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Fixture {
        RunStarted { run_id: String },
    }

    #[test]
    fn the_version_is_the_first_field_on_the_wire() {
        let frame = serde_json::to_string(&Versioned::now(Fixture::RunStarted {
            run_id: "01J".to_string(),
        }))
        .unwrap();

        assert!(
            frame.starts_with(&format!("{{\"v\":{PROTOCOL_VERSION},")),
            "expected the version to lead the frame, got {frame}"
        );
    }

    #[test]
    fn flattening_leaves_the_payload_fields_where_an_older_client_expects_them() {
        let frame = serde_json::to_value(Versioned::now(Fixture::RunStarted {
            run_id: "01J".to_string(),
        }))
        .unwrap();

        assert_eq!(frame["type"], "run_started");
        assert_eq!(frame["run_id"], "01J");
        assert!(
            frame.get("payload").is_none(),
            "the payload must be flattened, not nested"
        );
    }

    #[test]
    fn a_frame_recorded_before_versioning_still_deserializes() {
        let legacy = r#"{"type":"run_started","run_id":"01J"}"#;

        let decoded: Versioned<Fixture> = serde_json::from_str(legacy).unwrap();

        assert_eq!(decoded.version, PROTOCOL_VERSION);
        assert_eq!(
            decoded.payload,
            Fixture::RunStarted {
                run_id: "01J".to_string()
            }
        );
    }

    #[test]
    fn a_versioned_frame_round_trips() {
        let original = Versioned::now(Fixture::RunStarted {
            run_id: "01J".to_string(),
        });

        let decoded: Versioned<Fixture> =
            serde_json::from_str(&serde_json::to_string(&original).unwrap()).unwrap();

        assert_eq!(decoded.version, original.version);
        assert_eq!(decoded.payload, original.payload);
    }
}
