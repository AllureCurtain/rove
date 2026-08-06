use rove_models::{
    AssistantTurn, ContentBlock, HistoryProjectionError, HistoryProjector, InternalCallId, Message,
    ModelMessage, Role, ToolCall, ToolResult, ToolResultStatus,
};
use serde::{Deserialize, Serialize};

use crate::types::SessionId;

pub const SESSION_SCHEMA_VERSION: u16 = 1;
pub const MAX_SESSION_ENTRIES: usize = 4096;

/// Provenance retained on application/session entries.  It identifies where
/// an entry came from without copying canonical provider payloads into the
/// session history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EntryProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Typed conversation/application material.  Controls, compaction notes, and
/// capability hydration remain entries even when they are omitted from the
/// provider model projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEntry {
    User {
        id: String,
        content: Vec<ContentBlock>,
        #[serde(default)]
        provenance: EntryProvenance,
    },
    Assistant {
        id: String,
        turn: AssistantTurn,
        #[serde(default)]
        provenance: EntryProvenance,
    },
    ToolResult {
        id: String,
        result: ToolResult,
        #[serde(default)]
        provenance: EntryProvenance,
    },
    Control {
        id: String,
        action: String,
        content: String,
        #[serde(default)]
        provenance: EntryProvenance,
    },
    Compaction {
        id: String,
        summary: String,
        #[serde(default)]
        provenance: EntryProvenance,
    },
    Capability {
        id: String,
        capability: String,
        content: Vec<ContentBlock>,
        #[serde(default)]
        provenance: EntryProvenance,
    },
    /// Old artifacts that cannot be converted losslessly remain readable and
    /// are projected through the deterministic legacy compatibility path.
    Legacy {
        id: String,
        message: Message,
        #[serde(default)]
        provenance: EntryProvenance,
    },
}

impl SessionEntry {
    pub fn user(id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::User {
            id: id.into(),
            content: vec![ContentBlock::text(content)],
            provenance: EntryProvenance::default(),
        }
    }

    pub fn assistant(id: impl Into<String>, turn: AssistantTurn) -> Self {
        Self::Assistant {
            id: id.into(),
            turn,
            provenance: EntryProvenance::default(),
        }
    }

    pub fn tool_result(id: impl Into<String>, result: ToolResult) -> Self {
        Self::ToolResult {
            id: id.into(),
            result,
            provenance: EntryProvenance::default(),
        }
    }

    fn model_message(&self) -> Option<ModelMessage> {
        match self {
            Self::User { content, .. } => Some(ModelMessage {
                schema_version: SESSION_SCHEMA_VERSION,
                role: Role::User,
                content: content.clone(),
                tool_calls: Vec::new(),
                tool_result: None,
            }),
            Self::Assistant { turn, .. } => Some(ModelMessage::assistant(turn.clone())),
            Self::ToolResult { result, .. } => Some(ModelMessage::tool(result.clone())),
            Self::Compaction { summary, .. } => Some(ModelMessage::text(
                Role::System,
                format!("Session summary: {summary}"),
            )),
            Self::Control { .. } | Self::Capability { .. } | Self::Legacy { .. } => None,
        }
    }
}

/// Session tracks a user's interaction across multiple jobs.
///
/// M0: Minimal — just an ID. M1+: working memory, history, resume.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Session {
    pub id: SessionId,
    #[serde(default = "default_session_schema_version")]
    pub schema_version: u16,
    #[serde(default)]
    pub entries: Vec<SessionEntry>,
}

fn default_session_schema_version() -> u16 {
    SESSION_SCHEMA_VERSION
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: SessionId::new(),
            schema_version: SESSION_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }

    pub fn with_id(id: SessionId) -> Self {
        Self { id, ..Self::new() }
    }

    pub fn append(&mut self, entry: SessionEntry) -> Result<(), SessionError> {
        if self.entries.len() >= MAX_SESSION_ENTRIES {
            return Err(SessionError::TooManyEntries {
                max: MAX_SESSION_ENTRIES,
            });
        }
        if entry_id(&entry).trim().is_empty() {
            return Err(SessionError::EmptyEntryId);
        }
        if self
            .entries
            .iter()
            .any(|existing| entry_id(existing) == entry_id(&entry))
        {
            return Err(SessionError::DuplicateEntryId {
                id: entry_id(&entry).to_string(),
            });
        }
        self.validate_entry_append(&entry)?;
        self.entries.push(entry);
        Ok(())
    }

    pub fn model_messages(&self) -> Result<Vec<ModelMessage>, SessionError> {
        let mut messages = Vec::new();
        let mut legacy = Vec::new();
        for entry in &self.entries {
            match entry {
                SessionEntry::Legacy { message, .. } => legacy.push(message.clone()),
                _ => {
                    if !legacy.is_empty() {
                        messages.extend(legacy_history_to_model(&legacy));
                        legacy.clear();
                    }
                    if let Some(message) = entry.model_message() {
                        messages.push(message);
                    }
                }
            }
        }
        if !legacy.is_empty() {
            messages.extend(legacy_history_to_model(&legacy));
        }
        Ok(messages)
    }

    pub fn project_for_provider(
        &self,
        protocol: &str,
    ) -> Result<rove_models::ProjectedHistory, SessionError> {
        self.validate_projection()?;
        let messages = self.model_messages()?;
        HistoryProjector::new(protocol)
            .project(&messages)
            .map_err(SessionError::Projection)
    }

    pub fn from_legacy_history(id: SessionId, history: &[Message]) -> Self {
        let mut session = Self::with_id(id);
        session.entries = history
            .iter()
            .enumerate()
            .map(|(index, message)| SessionEntry::Legacy {
                id: format!("legacy-{index}"),
                message: message.clone(),
                provenance: EntryProvenance {
                    event_id: None,
                    source: Some("legacy_message".to_string()),
                },
            })
            .collect();
        session
    }

    fn validate_projection(&self) -> Result<(), SessionError> {
        let messages = self.model_messages()?;
        for message in &messages {
            message
                .validate()
                .map_err(|error| SessionError::InvalidEntry(error.to_string()))?;
        }
        // Validate ordering/correlation without making a provider request.
        HistoryProjector::new("session")
            .project(&messages)
            .map_err(SessionError::Projection)?;
        Ok(())
    }

    fn validate_entry_append(&self, entry: &SessionEntry) -> Result<(), SessionError> {
        match entry {
            SessionEntry::Assistant { turn, .. } => turn
                .validate()
                .map_err(|error| SessionError::InvalidEntry(error.to_string())),
            SessionEntry::ToolResult { result, .. } => {
                result
                    .validate()
                    .map_err(|error| SessionError::InvalidEntry(error.to_string()))?;
                let mut found_call = None;
                let mut already_completed = false;
                for existing in &self.entries {
                    match existing {
                        SessionEntry::Assistant { turn, .. } => {
                            if let Some(call) = turn
                                .tool_calls
                                .iter()
                                .find(|call| call.internal_call_id == result.internal_call_id)
                            {
                                found_call = Some(call.name.clone());
                            }
                        }
                        SessionEntry::ToolResult {
                            result: previous, ..
                        } if previous.internal_call_id == result.internal_call_id => {
                            already_completed = true;
                        }
                        _ => {}
                    }
                }
                let Some(expected_name) = found_call else {
                    return Err(SessionError::Projection(
                        HistoryProjectionError::OrphanResult {
                            index: self.entries.len(),
                            id: result.internal_call_id.to_string(),
                        },
                    ));
                };
                if already_completed {
                    return Err(SessionError::Projection(
                        HistoryProjectionError::DuplicateResult {
                            index: self.entries.len(),
                            id: result.internal_call_id.to_string(),
                        },
                    ));
                }
                if expected_name != result.tool_name {
                    return Err(SessionError::Projection(
                        HistoryProjectionError::ResultNameMismatch {
                            index: self.entries.len(),
                            expected: expected_name,
                            actual: result.tool_name.clone(),
                        },
                    ));
                }
                Ok(())
            }
            SessionEntry::User { .. }
            | SessionEntry::Control { .. }
            | SessionEntry::Compaction { .. }
            | SessionEntry::Capability { .. }
            | SessionEntry::Legacy { .. } => {
                if let Some(id) = first_unresolved_call(&self.entries) {
                    return Err(SessionError::Projection(
                        HistoryProjectionError::MissingResult { id },
                    ));
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum SessionError {
    #[error("session contains too many entries (maximum {max})")]
    TooManyEntries { max: usize },
    #[error("session entry id must not be empty")]
    EmptyEntryId,
    #[error("session entry id `{id}` is duplicated")]
    DuplicateEntryId { id: String },
    #[error("invalid session entry: {0}")]
    InvalidEntry(String),
    #[error("session projection failed: {0}")]
    Projection(#[from] HistoryProjectionError),
}

fn entry_id(entry: &SessionEntry) -> &str {
    match entry {
        SessionEntry::User { id, .. }
        | SessionEntry::Assistant { id, .. }
        | SessionEntry::ToolResult { id, .. }
        | SessionEntry::Control { id, .. }
        | SessionEntry::Compaction { id, .. }
        | SessionEntry::Capability { id, .. }
        | SessionEntry::Legacy { id, .. } => id,
    }
}

fn legacy_history_to_model(history: &[Message]) -> Vec<ModelMessage> {
    let mut messages = Vec::with_capacity(history.len());
    let mut pending = std::collections::BTreeMap::<String, InternalCallId>::new();
    for (index, message) in history.iter().enumerate() {
        let content = if message.content_blocks.is_empty() {
            vec![ContentBlock::text(message.content.clone())]
        } else {
            message.content_blocks.clone()
        };
        if message.role == Role::Assistant && !message.tool_calls.is_empty() {
            let tool_calls = message
                .tool_calls
                .iter()
                .enumerate()
                .map(|(call_index, call)| {
                    let id = InternalCallId::new(format!("legacy-call-{index}-{call_index}"))
                        .expect("deterministic legacy identity is valid");
                    pending.insert(call.id.clone(), id.clone());
                    ToolCall {
                        internal_call_id: id,
                        name: call.name.clone(),
                        arguments: call.args.clone(),
                        wire_reference: None,
                    }
                })
                .collect();
            messages.push(ModelMessage {
                schema_version: SESSION_SCHEMA_VERSION,
                role: Role::Assistant,
                content,
                tool_calls,
                tool_result: None,
            });
        } else if message.role == Role::Tool {
            let id = message
                .internal_call_id
                .clone()
                .or_else(|| {
                    message
                        .tool_call_id
                        .as_ref()
                        .and_then(|id| pending.get(id).cloned())
                })
                .or_else(|| pending.values().next().cloned())
                .unwrap_or_else(|| {
                    InternalCallId::new(format!("legacy-result-{index}"))
                        .expect("deterministic legacy identity is valid")
                });
            messages.push(ModelMessage::tool(ToolResult {
                internal_call_id: id.clone(),
                tool_name: message
                    .tool_name
                    .clone()
                    .unwrap_or_else(|| "legacy_tool".to_string()),
                content,
                status: message
                    .tool_result_status
                    .clone()
                    .unwrap_or(ToolResultStatus::Ok),
                error_code: None,
            }));
            pending.retain(|_, candidate| candidate != &id);
        } else {
            messages.push(ModelMessage {
                schema_version: SESSION_SCHEMA_VERSION,
                role: message.role.clone(),
                content,
                tool_calls: Vec::new(),
                tool_result: None,
            });
        }
    }
    messages
}

fn first_unresolved_call(entries: &[SessionEntry]) -> Option<String> {
    let mut calls = std::collections::BTreeSet::new();
    let mut results = std::collections::BTreeSet::new();
    for entry in entries {
        match entry {
            SessionEntry::Assistant { turn, .. } => {
                calls.extend(
                    turn.tool_calls
                        .iter()
                        .map(|call| call.internal_call_id.clone()),
                );
            }
            SessionEntry::ToolResult { result, .. } => {
                results.insert(result.internal_call_id.clone());
            }
            _ => {}
        }
    }
    calls.difference(&results).next().map(ToString::to_string)
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_session_round_trips_and_projects_call_result_atomically() {
        let id = InternalCallId::new("run-call-1").unwrap();
        let mut session = Session::new();
        session
            .append(SessionEntry::user("u-1", "inspect"))
            .unwrap();
        session
            .append(SessionEntry::assistant(
                "a-1",
                AssistantTurn {
                    tool_calls: vec![rove_models::ToolCall::new(
                        id.clone(),
                        "echo",
                        serde_json::json!({"message":"ok"}),
                    )],
                    stop_reason: rove_models::StopReason::ToolUse,
                    ..AssistantTurn::default()
                },
            ))
            .unwrap();
        session
            .append(SessionEntry::tool_result(
                "t-1",
                ToolResult::text(id, "echo", "ok"),
            ))
            .unwrap();
        let decoded: Session =
            serde_json::from_str(&serde_json::to_string(&session).unwrap()).unwrap();
        assert_eq!(decoded, session);
        let projected = session.project_for_provider("openai-completions").unwrap();
        assert_eq!(projected.messages.len(), 3);
        assert_eq!(
            projected.messages[1].tool_calls[0].id,
            projected.messages[2].tool_call_id.clone().unwrap()
        );
    }

    #[test]
    fn invalid_or_duplicate_entry_correlations_fail_before_append() {
        let id = InternalCallId::new("call-1").unwrap();
        let mut session = Session::new();
        session
            .append(SessionEntry::tool_result(
                "t-1",
                ToolResult::text(id, "echo", "orphan"),
            ))
            .unwrap_err();
        assert!(session.entries.is_empty());

        session.append(SessionEntry::user("same", "one")).unwrap();
        assert!(matches!(
            session.append(SessionEntry::user("same", "two")),
            Err(SessionError::DuplicateEntryId { .. })
        ));
    }

    #[test]
    fn old_message_history_is_readable_through_legacy_entries() {
        let history = vec![Message::user("hello"), Message::assistant("world")];
        let session = Session::from_legacy_history(SessionId::new(), &history);
        let messages = session.model_messages().unwrap();
        assert_eq!(messages.len(), history.len());
        assert_eq!(messages[0].role, Role::User);
    }
}
