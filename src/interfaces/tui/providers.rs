use std::sync::Arc;

use crate::core::types::{ToolApprovalProvider, UserInputProvider};
use crate::interfaces::terminal::interaction::{
    TerminalInteractionProviders, bounded_interaction_channel,
};

pub use crate::interfaces::terminal::interaction::{
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
}
