use std::sync::Arc;

use crate::terminal::interaction::{TerminalInteractionProviders, bounded_interaction_channel};
use rove_runtime::types::{ToolApprovalProvider, UserInputProvider};

pub use crate::terminal::interaction::{
    TerminalInteractionKind as TuiInteractionKind,
    TerminalInteractionProviders as TuiInteractionProviders,
    TerminalInteractionReceiver as TuiInteractionReceiver,
    TerminalInteractionRequest as TuiInteractionRequest,
};

pub const DEFAULT_TUI_INTERACTION_CAPACITY: usize = 8;

pub struct TuiInteractionBroker {
    pub approval_provider: Arc<dyn ToolApprovalProvider>,
    pub input_provider: Arc<dyn UserInputProvider>,
    pub receiver: TuiInteractionReceiver,
}

impl TuiInteractionBroker {
    pub fn new(capacity: usize) -> Self {
        let (
            TerminalInteractionProviders {
                approval_provider,
                input_provider,
            },
            receiver,
        ) = bounded_interaction_channel(capacity);

        Self {
            approval_provider,
            input_provider,
            receiver,
        }
    }

    /// Splits the provider handles used to build the runtime from the unique
    /// receiver owned by the TUI application loop.
    pub fn into_parts(self) -> (TuiInteractionProviders, TuiInteractionReceiver) {
        (
            TuiInteractionProviders {
                approval_provider: self.approval_provider,
                input_provider: self.input_provider,
            },
            self.receiver,
        )
    }
}

impl Default for TuiInteractionBroker {
    fn default() -> Self {
        Self::new(DEFAULT_TUI_INTERACTION_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_TUI_INTERACTION_CAPACITY, TuiInteractionBroker};

    #[test]
    fn default_broker_exposes_bounded_receiver_and_providers() {
        let broker = TuiInteractionBroker::default();

        assert_eq!(
            broker.receiver.max_capacity(),
            DEFAULT_TUI_INTERACTION_CAPACITY
        );
        assert_eq!(broker.receiver.capacity(), DEFAULT_TUI_INTERACTION_CAPACITY);
        assert_eq!(std::sync::Arc::strong_count(&broker.approval_provider), 1);
        assert_eq!(std::sync::Arc::strong_count(&broker.input_provider), 1);
    }

    #[test]
    fn broker_splits_runtime_providers_from_the_unique_receiver() {
        let broker = TuiInteractionBroker::new(3);

        let (providers, receiver) = broker.into_parts();

        assert_eq!(receiver.max_capacity(), 3);
        assert_eq!(receiver.capacity(), 3);
        assert_eq!(
            std::sync::Arc::strong_count(&providers.approval_provider),
            1
        );
        assert_eq!(std::sync::Arc::strong_count(&providers.input_provider), 1);
    }
}
