//! Shared product-message domain contract.
//!
//! Product surfaces own adapters for their existing durable stores. This
//! module owns validation and lifecycle vocabulary only; canonical runtime
//! facts remain [`crate::events::StreamEvent`] values.

use std::sync::Arc;

use async_trait::async_trait;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::events::StreamEvent;
use crate::types::RunId;

pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;
pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
pub const MAX_PENDING_MESSAGES: i64 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDelivery {
    Successor,
    CurrentRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Queued,
    InterventionRequested,
    AppliedCurrentRun,
    ClaimedSuccessor,
    NeedsAttention,
    Revoked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionDeliveryState {
    Idle,
    Active,
    NeedsAttention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub id: String,
    pub session_id: String,
    pub content: String,
    pub requested_delivery: MessageDelivery,
    pub actual_delivery: Option<MessageDelivery>,
    pub status: MessageStatus,
    pub sequence: i64,
    pub target_run_id: Option<RunId>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessageCommand {
    pub content: String,
    pub idempotency_key: Option<String>,
    pub session_state: SessionDeliveryState,
    pub target_run_id: Option<RunId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageMutation {
    pub message: ConversationMessage,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageErrorKind {
    Invalid,
    NotFound,
    Conflict,
    Rejected,
    Storage,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct MessageDomainError {
    pub kind: MessageErrorKind,
    pub message: String,
}

impl MessageDomainError {
    pub fn new(kind: MessageErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait MessageRepository: Send + Sync {
    async fn send(
        &self,
        session_id: &str,
        command: SendMessageCommand,
    ) -> Result<MessageMutation, MessageDomainError>;
    async fn list(&self, session_id: &str) -> Result<Vec<ConversationMessage>, MessageDomainError>;
    async fn promote(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError>;
    async fn revoke(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError>;
    async fn observe_event(
        &self,
        session_id: &str,
        run_id: RunId,
        event: &StreamEvent,
    ) -> Result<(), MessageDomainError>;
    async fn claim_successor(
        &self,
        session_id: &str,
        run_id: RunId,
    ) -> Result<Option<ConversationMessage>, MessageDomainError>;
    async fn require_attention(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Result<Vec<ConversationMessage>, MessageDomainError>;
}

#[derive(Clone)]
pub struct MessageDomainService {
    repository: Arc<dyn MessageRepository>,
}

impl MessageDomainService {
    pub fn new(repository: Arc<dyn MessageRepository>) -> Self {
        Self { repository }
    }

    pub async fn send(
        &self,
        session_id: &str,
        mut command: SendMessageCommand,
    ) -> Result<MessageMutation, MessageDomainError> {
        command.content = command.content.trim().to_string();
        if session_id.is_empty()
            || command.content.is_empty()
            || command.content.len() > MAX_MESSAGE_BYTES
            || command
                .idempotency_key
                .as_ref()
                .is_some_and(|key| key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES)
        {
            return Err(MessageDomainError::new(
                MessageErrorKind::Invalid,
                "message content, session, or idempotency key is invalid",
            ));
        }
        self.repository.send(session_id, command).await
    }

    pub async fn list(
        &self,
        session_id: &str,
    ) -> Result<Vec<ConversationMessage>, MessageDomainError> {
        self.repository.list(session_id).await
    }

    pub async fn promote(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError> {
        self.repository.promote(session_id, message_id).await
    }

    pub async fn revoke(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError> {
        self.repository.revoke(session_id, message_id).await
    }

    pub async fn observe_event(
        &self,
        session_id: &str,
        run_id: RunId,
        event: &StreamEvent,
    ) -> Result<(), MessageDomainError> {
        self.repository
            .observe_event(session_id, run_id, event)
            .await
    }

    pub async fn claim_successor(
        &self,
        session_id: &str,
        run_id: RunId,
    ) -> Result<Option<ConversationMessage>, MessageDomainError> {
        self.repository.claim_successor(session_id, run_id).await
    }

    pub async fn require_attention(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Result<Vec<ConversationMessage>, MessageDomainError> {
        self.repository.require_attention(session_id, reason).await
    }
}

/// Runtime-state adapter used by the local CLI/TUI. It is deliberately owned
/// by Runtime rather than TUI state, and shares `state.sqlite` with the
/// existing session/run index.
#[derive(Debug, Clone)]
pub struct SqliteMessageRepository {
    path: std::path::PathBuf,
    busy_timeout_ms: u64,
}

impl SqliteMessageRepository {
    pub fn new(path: impl Into<std::path::PathBuf>, busy_timeout_ms: u64) -> Self {
        Self {
            path: path.into(),
            busy_timeout_ms,
        }
    }

    fn connect(&self) -> Result<Connection, MessageDomainError> {
        let connection = Connection::open(&self.path).map_err(storage)?;
        connection
            .busy_timeout(std::time::Duration::from_millis(self.busy_timeout_ms))
            .map_err(storage)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(storage)?;
        Ok(connection)
    }
}

#[async_trait]
impl MessageRepository for SqliteMessageRepository {
    async fn send(
        &self,
        session_id: &str,
        command: SendMessageCommand,
    ) -> Result<MessageMutation, MessageDomainError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        if let Some(key) = command.idempotency_key.as_deref()
            && let Some(existing) = transaction
                .query_row(
                    "SELECT message_id, session_id, content, requested_delivery, actual_delivery, status, sequence, target_run_id, reason FROM conversation_messages WHERE session_id = ?1 AND idempotency_key = ?2",
                    params![session_id, key],
                    message_from_row,
                )
                .optional()
                .map_err(storage)?
        {
            if existing.content != command.content {
                return Err(MessageDomainError::new(
                    MessageErrorKind::Conflict,
                    "idempotency key already belongs to different message content",
                ));
            }
            transaction.commit().map_err(storage)?;
            return Ok(MessageMutation {
                message: existing,
                replayed: true,
            });
        }
        let pending: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM conversation_messages WHERE session_id = ?1 AND status IN ('queued', 'intervention_requested')",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(storage)?;
        if pending >= MAX_PENDING_MESSAGES {
            return Err(MessageDomainError::new(
                MessageErrorKind::Rejected,
                "message queue is full",
            ));
        }
        let sequence: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM conversation_messages WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .map_err(storage)?;
        let id = crate::types::SessionId::new().to_string();
        let (status, actual, reason, target_run_id) = match command.session_state {
            SessionDeliveryState::Idle => (
                "claimed_successor",
                Some("successor"),
                None,
                command.target_run_id.map(|id| id.to_string()),
            ),
            SessionDeliveryState::Active => ("queued", None, None, None),
            SessionDeliveryState::NeedsAttention => (
                "needs_attention",
                None,
                Some("session requires an explicit recovery decision"),
                None,
            ),
        };
        transaction
            .execute(
                "INSERT INTO conversation_messages(message_id, session_id, idempotency_key, content, requested_delivery, actual_delivery, status, sequence, target_run_id, reason, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'successor', ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![id, session_id, command.idempotency_key, command.content, actual, status, sequence, target_run_id, reason, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(storage)?;
        let message = get_message(&transaction, session_id, &id)?;
        transaction.commit().map_err(storage)?;
        Ok(MessageMutation {
            message,
            replayed: false,
        })
    }

    async fn list(&self, session_id: &str) -> Result<Vec<ConversationMessage>, MessageDomainError> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT message_id, session_id, content, requested_delivery, actual_delivery, status, sequence, target_run_id, reason FROM conversation_messages WHERE session_id = ?1 ORDER BY sequence")
            .map_err(storage)?;
        statement
            .query_map(params![session_id], message_from_row)
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)
    }

    async fn promote(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError> {
        transition(
            self,
            session_id,
            message_id,
            &["queued"],
            "intervention_requested",
            Some("current_run"),
            None,
            None,
        )
    }

    async fn revoke(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let existing = get_message(&transaction, session_id, message_id)?;
        if existing.status == MessageStatus::Revoked {
            transaction.commit().map_err(storage)?;
            return Ok(existing);
        }
        if !matches!(
            existing.status,
            MessageStatus::Queued | MessageStatus::NeedsAttention
        ) {
            return Err(MessageDomainError::new(
                MessageErrorKind::Rejected,
                "message is no longer eligible for revocation",
            ));
        }
        transaction
            .execute(
                "UPDATE conversation_messages SET status = 'revoked', updated_at = ?3 WHERE session_id = ?1 AND message_id = ?2 AND status IN ('queued', 'needs_attention')",
                params![session_id, message_id, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(storage)?;
        let updated = get_message(&transaction, session_id, message_id)?;
        transaction.commit().map_err(storage)?;
        Ok(updated)
    }

    async fn observe_event(
        &self,
        session_id: &str,
        run_id: RunId,
        event: &StreamEvent,
    ) -> Result<(), MessageDomainError> {
        let (id, from, to, actual, reason) = match event {
            StreamEvent::MessageInterventionRequested { id } => (
                id,
                &["queued", "intervention_requested"][..],
                "intervention_requested",
                Some("current_run"),
                None,
            ),
            StreamEvent::MessageAppliedCurrentRun { id } => (
                id,
                &["intervention_requested"][..],
                "applied_current_run",
                Some("current_run"),
                None,
            ),
            StreamEvent::MessageClaimedSuccessor { id } => (
                id,
                &["queued", "claimed_successor"][..],
                "claimed_successor",
                Some("successor"),
                None,
            ),
            StreamEvent::MessageNeedsAttention { id, reason } => (
                id,
                &["queued", "intervention_requested", "needs_attention"][..],
                "needs_attention",
                None,
                Some(reason.as_str()),
            ),
            StreamEvent::MessageRevoked { id } => (
                id,
                &["queued", "needs_attention", "revoked"][..],
                "revoked",
                None,
                None,
            ),
            StreamEvent::MessageQueued { .. } => return Ok(()),
            _ => return Ok(()),
        };
        let _ = transition(self, session_id, id, from, to, actual, Some(run_id), reason)?;
        Ok(())
    }

    async fn claim_successor(
        &self,
        session_id: &str,
        run_id: RunId,
    ) -> Result<Option<ConversationMessage>, MessageDomainError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let id: Option<String> = transaction
            .query_row(
                "SELECT message_id FROM conversation_messages WHERE session_id = ?1 AND status = 'queued' ORDER BY sequence LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage)?;
        let Some(id) = id else {
            transaction.commit().map_err(storage)?;
            return Ok(None);
        };
        let changed = transaction
            .execute(
                "UPDATE conversation_messages SET status = 'claimed_successor', actual_delivery = 'successor', target_run_id = ?3, updated_at = ?4 WHERE session_id = ?1 AND message_id = ?2 AND status = 'queued'",
                params![session_id, id, run_id.to_string(), chrono::Utc::now().to_rfc3339()],
            )
            .map_err(storage)?;
        if changed != 1 {
            return Err(MessageDomainError::new(
                MessageErrorKind::Conflict,
                "successor claim lost its compare-and-set race",
            ));
        }
        let message = get_message(&transaction, session_id, &id)?;
        transaction.commit().map_err(storage)?;
        Ok(Some(message))
    }

    async fn require_attention(
        &self,
        session_id: &str,
        reason: &str,
    ) -> Result<Vec<ConversationMessage>, MessageDomainError> {
        let mut connection = self.connect()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let mut statement = transaction
            .prepare("SELECT message_id FROM conversation_messages WHERE session_id = ?1 AND status IN ('queued', 'intervention_requested') ORDER BY sequence")
            .map_err(storage)?;
        let ids = statement
            .query_map(params![session_id], |row| row.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        drop(statement);
        transaction
            .execute(
                "UPDATE conversation_messages SET status = 'needs_attention', reason = ?2, updated_at = ?3 WHERE session_id = ?1 AND status IN ('queued', 'intervention_requested')",
                params![session_id, reason, chrono::Utc::now().to_rfc3339()],
            )
            .map_err(storage)?;
        let messages = ids
            .iter()
            .map(|id| get_message(&transaction, session_id, id))
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().map_err(storage)?;
        Ok(messages)
    }
}

#[allow(clippy::too_many_arguments)]
fn transition(
    repository: &SqliteMessageRepository,
    session_id: &str,
    message_id: &str,
    from: &[&str],
    to: &str,
    actual: Option<&str>,
    run_id: Option<RunId>,
    reason: Option<&str>,
) -> Result<ConversationMessage, MessageDomainError> {
    let mut connection = repository.connect()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    let existing = get_message(&transaction, session_id, message_id)?;
    if status_to_db(existing.status) == to {
        transaction.commit().map_err(storage)?;
        return Ok(existing);
    }
    if !from.contains(&status_to_db(existing.status)) {
        return Err(MessageDomainError::new(
            MessageErrorKind::Rejected,
            "message transition is no longer eligible",
        ));
    }
    transaction
        .execute(
            "UPDATE conversation_messages SET status = ?3, actual_delivery = COALESCE(?4, actual_delivery), target_run_id = COALESCE(?5, target_run_id), reason = COALESCE(?6, reason), updated_at = ?7 WHERE session_id = ?1 AND message_id = ?2 AND status = ?8",
            params![session_id, message_id, to, actual, run_id.map(|id| id.to_string()), reason, chrono::Utc::now().to_rfc3339(), status_to_db(existing.status)],
        )
        .map_err(storage)?;
    let updated = get_message(&transaction, session_id, message_id)?;
    transaction.commit().map_err(storage)?;
    Ok(updated)
}

fn get_message(
    connection: &Connection,
    session_id: &str,
    message_id: &str,
) -> Result<ConversationMessage, MessageDomainError> {
    connection
        .query_row(
            "SELECT message_id, session_id, content, requested_delivery, actual_delivery, status, sequence, target_run_id, reason FROM conversation_messages WHERE session_id = ?1 AND message_id = ?2",
            params![session_id, message_id],
            message_from_row,
        )
        .optional()
        .map_err(storage)?
        .ok_or_else(|| MessageDomainError::new(MessageErrorKind::NotFound, "message not found"))
}

fn message_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationMessage> {
    let requested: String = row.get(3)?;
    let actual: Option<String> = row.get(4)?;
    let status: String = row.get(5)?;
    let target_run_id: Option<String> = row.get(7)?;
    Ok(ConversationMessage {
        id: row.get(0)?,
        session_id: row.get(1)?,
        content: row.get(2)?,
        requested_delivery: delivery_from_db(&requested)?,
        actual_delivery: actual.as_deref().map(delivery_from_db).transpose()?,
        status: status_from_db(&status)?,
        sequence: row.get(6)?,
        target_run_id: target_run_id
            .as_deref()
            .map(|value| serde_json::from_value(serde_json::Value::String(value.to_string())))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    7,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        reason: row.get(8)?,
    })
}

fn delivery_from_db(value: &str) -> rusqlite::Result<MessageDelivery> {
    match value {
        "successor" => Ok(MessageDelivery::Successor),
        "current_run" => Ok(MessageDelivery::CurrentRun),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn status_from_db(value: &str) -> rusqlite::Result<MessageStatus> {
    match value {
        "queued" => Ok(MessageStatus::Queued),
        "intervention_requested" => Ok(MessageStatus::InterventionRequested),
        "applied_current_run" => Ok(MessageStatus::AppliedCurrentRun),
        "claimed_successor" => Ok(MessageStatus::ClaimedSuccessor),
        "needs_attention" => Ok(MessageStatus::NeedsAttention),
        "revoked" => Ok(MessageStatus::Revoked),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}

fn status_to_db(value: MessageStatus) -> &'static str {
    match value {
        MessageStatus::Queued => "queued",
        MessageStatus::InterventionRequested => "intervention_requested",
        MessageStatus::AppliedCurrentRun => "applied_current_run",
        MessageStatus::ClaimedSuccessor => "claimed_successor",
        MessageStatus::NeedsAttention => "needs_attention",
        MessageStatus::Revoked => "revoked",
    }
}

fn storage(error: impl std::fmt::Display) -> MessageDomainError {
    tracing::debug!(%error, "conversation message storage operation failed");
    MessageDomainError::new(
        MessageErrorKind::Storage,
        "message storage operation failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::index::StateIndex;

    #[tokio::test]
    async fn sqlite_adapter_is_fifo_idempotent_and_cas_safe() {
        let temp = tempfile::TempDir::new().unwrap();
        let index = StateIndex::new(temp.path());
        index.initialize().unwrap();
        let service = MessageDomainService::new(Arc::new(SqliteMessageRepository::new(
            index.path(),
            index.busy_timeout_ms(),
        )));
        let first = service
            .send(
                "session",
                SendMessageCommand {
                    content: " first ".to_string(),
                    idempotency_key: Some("one".to_string()),
                    session_state: SessionDeliveryState::Active,
                    target_run_id: None,
                },
            )
            .await
            .unwrap();
        let replay = service
            .send(
                "session",
                SendMessageCommand {
                    content: "first".to_string(),
                    idempotency_key: Some("one".to_string()),
                    session_state: SessionDeliveryState::Active,
                    target_run_id: None,
                },
            )
            .await
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(first.message.id, replay.message.id);
        let second = service
            .send(
                "session",
                SendMessageCommand {
                    content: "second".to_string(),
                    idempotency_key: Some("two".to_string()),
                    session_state: SessionDeliveryState::Active,
                    target_run_id: None,
                },
            )
            .await
            .unwrap();
        assert!(first.message.sequence < second.message.sequence);
        let promoted = service.promote("session", &first.message.id).await.unwrap();
        assert_eq!(promoted.status, MessageStatus::InterventionRequested);
        assert_eq!(
            service
                .revoke("session", &first.message.id)
                .await
                .unwrap_err()
                .kind,
            MessageErrorKind::Rejected
        );
        let claimed = service
            .claim_successor("session", RunId::new())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, second.message.id);
    }
}
