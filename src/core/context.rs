use crate::core::types::Message;

/// Manages prompt construction and context budget.
///
/// Responsible for assembling the full prompt from:
/// - System prompt (from file)
/// - Memory context
/// - Conversation history
/// - Current user message
///
/// M0: Simple concatenation. M1+: budget-aware truncation/summarization.
pub struct ContextManager {
    system_prompt: String,
    max_history: usize,
}

impl ContextManager {
    pub fn new(system_prompt: String) -> Self {
        Self {
            system_prompt,
            max_history: 20, // Keep last N messages
        }
    }

    pub fn with_max_history(system_prompt: String, max_history: usize) -> Self {
        Self {
            system_prompt,
            max_history,
        }
    }

    /// Build the full message list to send to the LLM.
    pub fn build(
        &self,
        user_message: &str,
        working_memory: &[Message],
        history: &[Message],
    ) -> Vec<Message> {
        let mut messages = Vec::new();

        // System prompt
        messages.push(Message {
            role: crate::core::types::Role::System,
            content: self.system_prompt.clone(),
        });

        // Memory sections always precede trimmed history.
        messages.extend_from_slice(working_memory);

        // History (truncated to budget)
        let start = if history.len() > self.max_history {
            history.len() - self.max_history
        } else {
            0
        };
        messages.extend_from_slice(&history[start..]);

        // Current user message
        messages.push(Message {
            role: crate::core::types::Role::User,
            content: user_message.to_string(),
        });

        messages
    }

    /// Get the system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }
}

pub fn session_summary_message(summary: &str) -> Message {
    Message {
        role: crate::core::types::Role::System,
        content: format!("Session summary: {summary}"),
    }
}

pub fn durable_memory_message(index: &str) -> Message {
    Message {
        role: crate::core::types::Role::System,
        content: format!("Durable memory:\n{}", index.trim_end()),
    }
}
