use crate::core::types::{Message, Role};

const MESSAGE_OVERHEAD_TOKENS: usize = 4;
const CHARS_PER_TOKEN: usize = 4;

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
    history_limit: HistoryLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub soft_limit_tokens: usize,
    pub hard_limit_tokens: usize,
    pub reserved_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBuild {
    pub messages: Vec<Message>,
    pub token_estimate: usize,
    pub included_history_messages: usize,
    pub dropped_history_messages: usize,
    pub over_hard_limit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HistoryLimit {
    MessageCount(usize),
    TokenBudget(ContextBudget),
}

impl ContextManager {
    pub fn new(system_prompt: String) -> Self {
        Self {
            system_prompt,
            history_limit: HistoryLimit::MessageCount(20),
        }
    }

    pub fn with_max_history(system_prompt: String, max_history: usize) -> Self {
        Self {
            system_prompt,
            history_limit: HistoryLimit::MessageCount(max_history),
        }
    }

    pub fn with_token_budget(system_prompt: String, budget: ContextBudget) -> Self {
        Self {
            system_prompt,
            history_limit: HistoryLimit::TokenBudget(budget),
        }
    }

    /// Build the full message list to send to the LLM.
    pub fn build(
        &self,
        user_message: &str,
        working_memory: &[Message],
        history: &[Message],
    ) -> Vec<Message> {
        self.build_with_checkpoint(user_message, working_memory, None, history)
            .messages
    }

    pub fn build_with_checkpoint(
        &self,
        user_message: &str,
        working_memory: &[Message],
        compact_summary: Option<&str>,
        history: &[Message],
    ) -> ContextBuild {
        match self.history_limit {
            HistoryLimit::MessageCount(max_history) => self.build_by_message_count(
                user_message,
                working_memory,
                compact_summary,
                history,
                max_history,
            ),
            HistoryLimit::TokenBudget(budget) => self.build_by_token_budget(
                user_message,
                working_memory,
                compact_summary,
                history,
                budget,
            ),
        }
    }

    fn build_by_message_count(
        &self,
        user_message: &str,
        working_memory: &[Message],
        compact_summary: Option<&str>,
        history: &[Message],
        max_history: usize,
    ) -> ContextBuild {
        let mut messages = Vec::new();

        messages.push(Message {
            role: Role::System,
            content: self.system_prompt.clone(),
        });
        messages.extend_from_slice(working_memory);
        if let Some(summary) = compact_summary {
            messages.push(compact_summary_message(summary));
        }

        let start = if history.len() > max_history {
            history.len() - max_history
        } else {
            0
        };
        messages.extend_from_slice(&history[start..]);
        messages.push(Message {
            role: Role::User,
            content: user_message.to_string(),
        });

        let token_estimate = estimate_messages_tokens(&messages);
        ContextBuild {
            messages,
            token_estimate,
            included_history_messages: history.len().saturating_sub(start),
            dropped_history_messages: start,
            over_hard_limit: false,
        }
    }

    fn build_by_token_budget(
        &self,
        user_message: &str,
        working_memory: &[Message],
        compact_summary: Option<&str>,
        history: &[Message],
        budget: ContextBudget,
    ) -> ContextBuild {
        let current_user = Message {
            role: Role::User,
            content: user_message.to_string(),
        };
        let mut prefix = Vec::new();
        prefix.push(Message {
            role: Role::System,
            content: self.system_prompt.clone(),
        });
        prefix.extend_from_slice(working_memory);
        if let Some(summary) = compact_summary {
            prefix.push(compact_summary_message(summary));
        }
        let required_tokens =
            estimate_messages_tokens(&prefix) + estimate_message_tokens(&current_user);
        let target_limit = prompt_target_limit(budget, required_tokens);

        let mut selected_history = Vec::new();
        let mut selected_tokens = 0;
        let mut included_history_messages = 0;
        for message in history.iter().rev() {
            let message_tokens = estimate_message_tokens(message);
            if required_tokens + selected_tokens + message_tokens > target_limit {
                break;
            }
            selected_tokens += message_tokens;
            included_history_messages += 1;
            selected_history.push(message.clone());
        }
        selected_history.reverse();

        let mut messages = prefix;
        messages.extend(selected_history);
        messages.push(current_user);

        let token_estimate = estimate_messages_tokens(&messages);
        ContextBuild {
            messages,
            token_estimate,
            included_history_messages,
            dropped_history_messages: history.len().saturating_sub(included_history_messages),
            over_hard_limit: token_estimate
                > budget
                    .hard_limit_tokens
                    .saturating_sub(budget.reserved_tokens),
        }
    }

    /// Get the system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }
}

pub fn session_summary_message(summary: &str) -> Message {
    Message {
        role: Role::System,
        content: format!("Session summary: {summary}"),
    }
}

pub fn compact_summary_message(summary: &str) -> Message {
    Message {
        role: Role::System,
        content: format!("Compact summary: {summary}"),
    }
}

pub fn durable_memory_message(index: &str) -> Message {
    Message {
        role: Role::System,
        content: format!("Durable memory:\n{}", index.trim_end()),
    }
}

pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn estimate_message_tokens(message: &Message) -> usize {
    MESSAGE_OVERHEAD_TOKENS + estimate_text_tokens(&message.content)
}

fn estimate_text_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(CHARS_PER_TOKEN).max(1)
}

fn prompt_target_limit(budget: ContextBudget, required_tokens: usize) -> usize {
    let hard_available = budget
        .hard_limit_tokens
        .saturating_sub(budget.reserved_tokens);
    let soft_available = budget
        .soft_limit_tokens
        .saturating_sub(budget.reserved_tokens);
    hard_available.min(soft_available.max(required_tokens))
}
