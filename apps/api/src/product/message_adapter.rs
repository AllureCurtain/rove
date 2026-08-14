//! Adapter from the shared runtime message lifecycle to the API ProductStore.
//!
//! ProductStore remains the API's durable authority. This adapter only maps
//! the shared domain command/status vocabulary onto the existing transactional
//! ProductStore methods; it does not create a second event log or queue.

use std::sync::Arc;

use async_trait::async_trait;

use rove_runtime::conversation::{
    ConversationMessage, MessageDelivery, MessageDomainError, MessageErrorKind, MessageMutation,
    MessagePage, MessagePageQuery, MessageRepository, MessageStatus, SendMessageCommand,
};
use rove_runtime::events::StreamEvent;
use rove_runtime::types::RunId;

use super::{
    CreateProductMessageRequest, ProductControlId, ProductControlStatus, ProductMessage,
    ProductMessageDelivery, ProductMessagePageQuery, ProductMessageStatus, ProductSessionId,
    ProductStore, ProductStoreError,
};

#[derive(Clone)]
pub(crate) struct ProductMessageRepository {
    store: Arc<dyn ProductStore>,
}

impl ProductMessageRepository {
    pub(crate) fn new(store: Arc<dyn ProductStore>) -> Self {
        Self { store }
    }

    fn session_id(value: &str) -> Result<ProductSessionId, MessageDomainError> {
        value
            .parse()
            .map_err(|error: String| MessageDomainError::new(MessageErrorKind::Invalid, error))
    }

    fn message_id(value: &str) -> Result<ProductControlId, MessageDomainError> {
        value
            .parse()
            .map_err(|error: String| MessageDomainError::new(MessageErrorKind::Invalid, error))
    }
}

pub(crate) fn service(
    store: Arc<dyn ProductStore>,
) -> rove_runtime::conversation::MessageDomainService {
    rove_runtime::conversation::MessageDomainService::new(Arc::new(ProductMessageRepository::new(
        store,
    )))
}

#[async_trait]
impl MessageRepository for ProductMessageRepository {
    async fn send(
        &self,
        session_id: &str,
        command: SendMessageCommand,
    ) -> Result<MessageMutation, MessageDomainError> {
        let session_id = Self::session_id(session_id)?;
        let (message, replayed) = self
            .store
            .create_message(
                &session_id,
                CreateProductMessageRequest {
                    content: command.content,
                    idempotency_key: command.idempotency_key,
                },
            )
            .await
            .map_err(map_product_error)?;
        Ok(MessageMutation {
            message: to_domain(message),
            claimed_successor: None,
            replayed,
        })
    }

    async fn list(
        &self,
        session_id: &str,
        query: MessagePageQuery,
    ) -> Result<MessagePage, MessageDomainError> {
        let session_id = Self::session_id(session_id)?;
        self.store
            .list_messages(
                &session_id,
                ProductMessagePageQuery {
                    after_seq: query.after_sequence,
                    before_seq: query.before_sequence,
                    limit: query.limit,
                },
            )
            .await
            .map(|page| MessagePage {
                messages: page.messages.into_iter().map(to_domain).collect(),
                next_after_sequence: page.next_after_seq,
                next_before_sequence: page.next_before_seq,
            })
            .map_err(map_product_error)
    }

    async fn promote(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError> {
        let session_id = Self::session_id(session_id)?;
        let message_id = Self::message_id(message_id)?;
        self.store
            .promote_message(&session_id, &message_id)
            .await
            .map(to_domain)
            .map_err(map_product_error)
    }

    async fn revoke(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> Result<ConversationMessage, MessageDomainError> {
        let session_id = Self::session_id(session_id)?;
        let message_id = Self::message_id(message_id)?;
        self.store
            .revoke_message(&session_id, &message_id)
            .await
            .map(to_domain)
            .map_err(map_product_error)
    }

    async fn observe_event(
        &self,
        session_id: &str,
        run_id: RunId,
        event: &StreamEvent,
    ) -> Result<(), MessageDomainError> {
        let session_id = Self::session_id(session_id)?;
        let Some((message_id, from, to)) = event_transition(event) else {
            return Ok(());
        };
        let message_id = Self::message_id(message_id)?;
        let transitioned = self
            .store
            .transition_control(&session_id, &message_id, from, to, Some(&run_id))
            .await;
        // Canonical event replay is idempotent. A stale CAS is therefore a
        // successful observation when the row already reflects the outcome.
        match transitioned {
            Ok(_)
            | Err(ProductStoreError {
                code: super::ProductErrorCode::ProductControlRejected,
                ..
            }) => Ok(()),
            Err(error) => Err(map_product_error(error)),
        }
    }

    async fn claim_successor(
        &self,
        _session_id: &str,
        _run_id: RunId,
    ) -> Result<Option<ConversationMessage>, MessageDomainError> {
        Err(MessageDomainError::new(
            MessageErrorKind::Rejected,
            "successor claims are coordinator-owned",
        ))
    }

    async fn require_attention(
        &self,
        _session_id: &str,
        _reason: &str,
    ) -> Result<Vec<ConversationMessage>, MessageDomainError> {
        Err(MessageDomainError::new(
            MessageErrorKind::Rejected,
            "attention transitions are coordinator-owned",
        ))
    }
}

fn event_transition(
    event: &StreamEvent,
) -> Option<(&str, ProductControlStatus, ProductControlStatus)> {
    match event {
        StreamEvent::MessageInterventionRequested { id } => Some((
            id,
            ProductControlStatus::Pending,
            ProductControlStatus::Accepted,
        )),
        StreamEvent::MessageAppliedCurrentRun { id } => Some((
            id,
            ProductControlStatus::Accepted,
            ProductControlStatus::Applied,
        )),
        StreamEvent::MessageClaimedSuccessor { id } => Some((
            id,
            ProductControlStatus::Accepted,
            ProductControlStatus::Applied,
        )),
        StreamEvent::MessageNeedsAttention { id, .. } => Some((
            id,
            ProductControlStatus::Pending,
            ProductControlStatus::Abandoned,
        )),
        StreamEvent::MessageRevoked { id } => Some((
            id,
            ProductControlStatus::Pending,
            ProductControlStatus::Revoked,
        )),
        _ => None,
    }
}

pub(crate) fn to_domain(message: ProductMessage) -> ConversationMessage {
    ConversationMessage {
        id: message.id.to_string(),
        session_id: message.product_session_id.to_string(),
        content: message.content,
        requested_delivery: match message.requested_delivery {
            ProductMessageDelivery::Successor => MessageDelivery::Successor,
            ProductMessageDelivery::CurrentRun => MessageDelivery::CurrentRun,
        },
        actual_delivery: message.actual_delivery.map(|delivery| match delivery {
            ProductMessageDelivery::Successor => MessageDelivery::Successor,
            ProductMessageDelivery::CurrentRun => MessageDelivery::CurrentRun,
        }),
        status: match message.status {
            ProductMessageStatus::Queued => MessageStatus::Queued,
            ProductMessageStatus::InterventionRequested => MessageStatus::InterventionRequested,
            ProductMessageStatus::AppliedCurrentRun => MessageStatus::AppliedCurrentRun,
            ProductMessageStatus::ClaimedSuccessor => MessageStatus::ClaimedSuccessor,
            ProductMessageStatus::NeedsAttention => MessageStatus::NeedsAttention,
            ProductMessageStatus::Revoked => MessageStatus::Revoked,
        },
        sequence: message.seq,
        target_run_id: message.run_id.or(message.successor_run_id),
        reason: message.reason,
    }
}

fn map_product_error(error: ProductStoreError) -> MessageDomainError {
    let kind = match error.code {
        super::ProductErrorCode::ProductNotFound => MessageErrorKind::NotFound,
        super::ProductErrorCode::ProductControlConflict => MessageErrorKind::Conflict,
        super::ProductErrorCode::ProductControlRejected => MessageErrorKind::Rejected,
        super::ProductErrorCode::ProductInvalidInput => MessageErrorKind::Invalid,
        _ => MessageErrorKind::Storage,
    };
    MessageDomainError::new(kind, error.message)
}

pub(crate) fn map_domain_error(error: MessageDomainError) -> ProductStoreError {
    let code = match error.kind {
        MessageErrorKind::Invalid => super::ProductErrorCode::ProductInvalidInput,
        MessageErrorKind::NotFound => super::ProductErrorCode::ProductNotFound,
        MessageErrorKind::Conflict => super::ProductErrorCode::ProductControlConflict,
        MessageErrorKind::Rejected => super::ProductErrorCode::ProductControlRejected,
        MessageErrorKind::Storage => super::ProductErrorCode::ProductStorageFailure,
    };
    ProductStoreError::new(code, error.message)
}
