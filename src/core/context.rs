use crate::core::prompt_metadata::{PromptBuildMetadata, prompt_hash, stable_hash};
use crate::core::types::Message;

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
    pub auto_compaction_needed: bool,
    pub metadata: PromptBuildMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionMode {
    None,
    Automatic,
    Degraded,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionDecision {
    pub mode: CompactionMode,
    pub circuit_open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionPolicy {
    pub consecutive_failures: u32,
    pub failure_threshold: u32,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            consecutive_failures: 0,
            failure_threshold: 3,
        }
    }
}

impl CompactionPolicy {
    pub fn decide(&self, context: &ContextBuild, budget: ContextBudget) -> CompactionDecision {
        if self.consecutive_failures >= self.failure_threshold.max(1) {
            return CompactionDecision {
                mode: CompactionMode::Disabled,
                circuit_open: true,
            };
        }

        if context.over_hard_limit {
            return CompactionDecision {
                mode: CompactionMode::Degraded,
                circuit_open: false,
            };
        }

        if context.auto_compaction_needed
            || context.token_estimate
                >= budget
                    .soft_limit_tokens
                    .saturating_sub(budget.reserved_tokens)
        {
            return CompactionDecision {
                mode: CompactionMode::Automatic,
                circuit_open: false,
            };
        }

        CompactionDecision {
            mode: CompactionMode::None,
            circuit_open: false,
        }
    }
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
        let stable_prefix_hash = self.stable_prefix_hash(working_memory, compact_summary);
        match self.history_limit {
            HistoryLimit::MessageCount(max_history) => self.build_by_message_count(
                user_message,
                working_memory,
                compact_summary,
                history,
                max_history,
                &stable_prefix_hash,
            ),
            HistoryLimit::TokenBudget(budget) => self.build_by_token_budget(
                user_message,
                working_memory,
                compact_summary,
                history,
                budget,
                &stable_prefix_hash,
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
        stable_prefix_hash: &str,
    ) -> ContextBuild {
        let mut messages = Vec::new();

        messages.push(Message::system(self.system_prompt.clone()));
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
        messages.push(Message::user(user_message));

        let token_estimate = estimate_messages_tokens(&messages);
        let metadata = self.build_metadata(
            &messages,
            token_estimate,
            history.len().saturating_sub(start),
            start,
            stable_prefix_hash,
        );
        ContextBuild {
            messages,
            token_estimate,
            included_history_messages: history.len().saturating_sub(start),
            dropped_history_messages: start,
            over_hard_limit: false,
            auto_compaction_needed: start > 0,
            metadata,
        }
    }

    fn build_by_token_budget(
        &self,
        user_message: &str,
        working_memory: &[Message],
        compact_summary: Option<&str>,
        history: &[Message],
        budget: ContextBudget,
        stable_prefix_hash: &str,
    ) -> ContextBuild {
        let current_user = Message::user(user_message);
        let mut prefix = Vec::new();
        prefix.push(Message::system(self.system_prompt.clone()));
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
        let over_hard_limit = token_estimate
            > budget
                .hard_limit_tokens
                .saturating_sub(budget.reserved_tokens);
        let auto_compaction_needed = token_estimate
            >= budget
                .soft_limit_tokens
                .saturating_sub(budget.reserved_tokens)
            || history.len().saturating_sub(included_history_messages) > 0;

        let dropped_history_messages = history.len().saturating_sub(included_history_messages);
        let metadata = self.build_metadata(
            &messages,
            token_estimate,
            included_history_messages,
            dropped_history_messages,
            stable_prefix_hash,
        );
        ContextBuild {
            messages,
            token_estimate,
            included_history_messages,
            dropped_history_messages,
            over_hard_limit,
            auto_compaction_needed,
            metadata,
        }
    }

    /// Get the system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    fn build_metadata(
        &self,
        messages: &[Message],
        token_estimate: usize,
        included_history_messages: usize,
        dropped_history_messages: usize,
        stable_prefix_hash: &str,
    ) -> PromptBuildMetadata {
        PromptBuildMetadata {
            prompt_hash: prompt_hash(messages),
            stable_prefix_hash: stable_prefix_hash.to_string(),
            workspace_fingerprint: String::new(),
            tool_signature: String::new(),
            token_estimate,
            included_history_messages,
            dropped_history_messages,
            prompt_cache_key: None,
        }
    }

    fn stable_prefix_hash(
        &self,
        working_memory: &[Message],
        compact_summary: Option<&str>,
    ) -> String {
        if working_memory.is_empty() && compact_summary.is_none() {
            return stable_hash(&self.system_prompt);
        }
        stable_hash(
            &serde_json::json!({
                "system_prompt": self.system_prompt,
                "working_memory": working_memory,
                "compact_summary": compact_summary,
            })
            .to_string(),
        )
    }
}

pub fn session_summary_message(summary: &str) -> Message {
    Message::system(format!("Session summary: {summary}"))
}

pub fn compact_summary_message(summary: &str) -> Message {
    Message::system(format!("Compact summary: {summary}"))
}

pub fn durable_memory_message(index: &str) -> Message {
    Message::system(format!("Durable memory:\n{}", index.trim_end()))
}

pub fn estimate_messages_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_message_tokens).sum()
}

pub fn estimate_message_tokens(message: &Message) -> usize {
    let tool_call_tokens: usize = message
        .tool_calls
        .iter()
        .map(|tool_call| {
            estimate_text_tokens(&tool_call.id)
                + estimate_text_tokens(&tool_call.name)
                + estimate_text_tokens(&tool_call.args.to_string())
        })
        .sum();
    let tool_call_id_tokens = message
        .tool_call_id
        .as_deref()
        .map(estimate_text_tokens)
        .unwrap_or(0);

    MESSAGE_OVERHEAD_TOKENS
        + estimate_text_tokens(&message.content)
        + tool_call_tokens
        + tool_call_id_tokens
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::ToolCallRef;

    #[test]
    fn estimate_message_tokens_counts_structured_tool_calls() {
        let plain = Message::assistant("");
        let with_tool_call = Message::assistant_with_tool_calls(
            "",
            vec![ToolCallRef {
                id: "toolu_1".to_string(),
                name: "echo".to_string(),
                args: serde_json::json!({
                    "message": "this argument text must count against context budget"
                }),
            }],
        );

        assert!(estimate_message_tokens(&with_tool_call) > estimate_message_tokens(&plain));
    }

    #[test]
    fn build_metadata_matches_prompt_and_history_selection() {
        let context = ContextManager::with_max_history("system".to_string(), 1);
        let history = vec![Message::user("old"), Message::assistant("new")];

        let built = context.build_with_checkpoint("current", &[], None, &history);

        assert_eq!(
            built.metadata.prompt_hash,
            crate::core::prompt_metadata::prompt_hash(&built.messages)
        );
        assert_eq!(
            built.metadata.stable_prefix_hash,
            crate::core::prompt_metadata::stable_hash("system")
        );
        assert_eq!(built.metadata.token_estimate, built.token_estimate);
        assert_eq!(built.metadata.included_history_messages, 1);
        assert_eq!(built.metadata.dropped_history_messages, 1);
        assert!(built.metadata.workspace_fingerprint.is_empty());
        assert!(built.metadata.tool_signature.is_empty());
        assert!(built.metadata.prompt_cache_key.is_none());
    }

    #[test]
    fn stable_prefix_hash_changes_with_memory_and_compact_summary() {
        let context = ContextManager::with_max_history("system".to_string(), 4);
        let first_memory = vec![Message::system("Durable memory:\none")];
        let second_memory = vec![Message::system("Durable memory:\ntwo")];

        let first = context.build_with_checkpoint("first", &first_memory, Some("summary"), &[]);
        let second = context.build_with_checkpoint("second", &second_memory, Some("summary"), &[]);
        let third =
            context.build_with_checkpoint("third", &second_memory, Some("new summary"), &[]);

        assert_ne!(
            first.metadata.stable_prefix_hash,
            second.metadata.stable_prefix_hash
        );
        assert_ne!(
            second.metadata.stable_prefix_hash,
            third.metadata.stable_prefix_hash
        );
    }

    #[test]
    fn context_manager_keeps_stable_prefix_on_repeated_builds() {
        let context = ContextManager::with_max_history("system".to_string(), 4);
        let working_memory = vec![Message::system("Durable memory:\nproject facts")];

        let first =
            context.build_with_checkpoint("first", &working_memory, Some("stable summary"), &[]);
        let second =
            context.build_with_checkpoint("second", &working_memory, Some("stable summary"), &[]);

        assert!(first.messages.iter().any(|msg| msg.content == "system"));
        assert!(
            first
                .messages
                .iter()
                .any(|msg| msg.content == "Durable memory:\nproject facts")
        );
        assert!(
            first
                .messages
                .iter()
                .any(|msg| msg.content == "Compact summary: stable summary")
        );
        assert!(second.messages.iter().any(|msg| msg.content == "system"));
        assert!(
            second
                .messages
                .iter()
                .any(|msg| msg.content == "Durable memory:\nproject facts")
        );
        assert!(
            second
                .messages
                .iter()
                .any(|msg| msg.content == "Compact summary: stable summary")
        );
        assert_eq!(
            first.metadata.stable_prefix_hash,
            second.metadata.stable_prefix_hash
        );
    }
}
