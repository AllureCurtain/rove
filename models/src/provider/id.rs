use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

pub const OPENAI_CHAT_PROTOCOL: &str = "openai-chat";
pub const OPENAI_RESPONSES_PROTOCOL: &str = "openai-responses";
pub const ANTHROPIC_MESSAGES_PROTOCOL: &str = "anthropic-messages";
pub const OLLAMA_CHAT_PROTOCOL: &str = "ollama-chat";
pub const FAKE_PROTOCOL: &str = "fake";
pub const EXTERNAL_ADAPTER_V1_PROTOCOL: &str = "external-adapter-v1";

const MAX_PROTOCOL_ID_BYTES: usize = 128;

/// Stable, open identifier for one model wire protocol.
///
/// IDs are deliberately not represented by a closed enum. Rove-owned built-in
/// IDs are exported as constants, while applications may register additional
/// canonical IDs.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WireProtocolId(String);

impl WireProtocolId {
    pub fn new(value: impl Into<String>) -> Result<Self, WireProtocolIdError> {
        let value = value.into();
        validate_protocol_id(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for WireProtocolId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("WireProtocolId")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for WireProtocolId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for WireProtocolId {
    type Err = WireProtocolIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value.to_owned())
    }
}

impl TryFrom<String> for WireProtocolId {
    type Error = WireProtocolIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for WireProtocolId {
    type Error = WireProtocolIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl Serialize for WireProtocolId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for WireProtocolId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum WireProtocolIdError {
    #[error("wire protocol id must not be empty")]
    Empty,
    #[error("wire protocol id exceeds {max} bytes")]
    TooLong { max: usize },
    #[error(
        "wire protocol id must start with a lowercase ASCII letter or digit and contain only lowercase ASCII letters, digits, '-', '.', '_', or '/'"
    )]
    InvalidSyntax,
}

fn validate_protocol_id(value: &str) -> Result<(), WireProtocolIdError> {
    if value.is_empty() {
        return Err(WireProtocolIdError::Empty);
    }
    if value.len() > MAX_PROTOCOL_ID_BYTES {
        return Err(WireProtocolIdError::TooLong {
            max: MAX_PROTOCOL_ID_BYTES,
        });
    }

    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err(WireProtocolIdError::Empty);
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(WireProtocolIdError::InvalidSyntax);
    }
    if !bytes.all(|byte| {
        byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(byte, b'-' | b'.' | b'_' | b'/')
    }) {
        return Err(WireProtocolIdError::InvalidSyntax);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_builtin_and_namespaced_protocol_ids() {
        for value in [
            OPENAI_CHAT_PROTOCOL,
            OPENAI_RESPONSES_PROTOCOL,
            ANTHROPIC_MESSAGES_PROTOCOL,
            OLLAMA_CHAT_PROTOCOL,
            FAKE_PROTOCOL,
            EXTERNAL_ADAPTER_V1_PROTOCOL,
            "acme/custom_v2",
        ] {
            assert_eq!(WireProtocolId::new(value).unwrap().as_str(), value);
        }
    }

    #[test]
    fn rejects_noncanonical_protocol_ids() {
        assert_eq!(
            WireProtocolId::new("").unwrap_err(),
            WireProtocolIdError::Empty
        );
        assert_eq!(
            WireProtocolId::new(" OpenAI").unwrap_err(),
            WireProtocolIdError::InvalidSyntax
        );
        assert_eq!(
            WireProtocolId::new("openai:chat").unwrap_err(),
            WireProtocolIdError::InvalidSyntax
        );
        assert!(matches!(
            WireProtocolId::new("a".repeat(MAX_PROTOCOL_ID_BYTES + 1)),
            Err(WireProtocolIdError::TooLong { .. })
        ));
    }

    #[test]
    fn serde_deserialization_runs_validation() {
        let id: WireProtocolId = serde_json::from_str("\"vendor/chat\"").unwrap();
        assert_eq!(id.as_str(), "vendor/chat");
        assert!(serde_json::from_str::<WireProtocolId>("\"Vendor Chat\"").is_err());
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"vendor/chat\"");
    }
}
