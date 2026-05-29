use tokio_util::sync::CancellationToken;

use crate::core::types::{Message, PromptCompactionMode, PromptCompactionState};
use crate::errors::ModelError;
use crate::models::traits::{ModelClient, ModelEvent};

pub(crate) const COMPACTION_PROMPT_VERSION: &str = "rove.compaction.v1";

#[derive(Debug, Clone)]
pub(crate) struct CompactionRuntime {
    pub enabled: bool,
    pub failure_threshold: u32,
    pub consecutive_failures: u32,
    pub last_error: Option<String>,
}

impl CompactionRuntime {
    pub(crate) fn new(enabled: bool, failure_threshold: u32) -> Self {
        Self {
            enabled,
            failure_threshold: failure_threshold.max(1),
            consecutive_failures: 0,
            last_error: None,
        }
    }

    pub(crate) fn circuit_open(&self) -> bool {
        self.enabled && self.consecutive_failures >= self.failure_threshold
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompactionUpdate {
    pub summary: Option<String>,
    pub state: PromptCompactionState,
}

pub(crate) async fn maybe_compact_history(
    runtime: &mut CompactionRuntime,
    model: &dyn ModelClient,
    compacted: &[Message],
    cancel_token: CancellationToken,
) -> Option<CompactionUpdate> {
    if compacted.is_empty() || !runtime.enabled || runtime.circuit_open() {
        return None;
    }

    match generate_summary(model, compacted, cancel_token).await {
        Ok(summary) => {
            runtime.consecutive_failures = 0;
            runtime.last_error = None;
            Some(CompactionUpdate {
                summary: Some(summary),
                state: PromptCompactionState {
                    mode: PromptCompactionMode::ModelGenerated,
                    auto_triggered: true,
                    degraded: false,
                    consecutive_failures: 0,
                    circuit_open: false,
                    model: Some(model.model_id().to_string()),
                    prompt_version: Some(COMPACTION_PROMPT_VERSION.to_string()),
                    source_message_count: compacted.len(),
                    last_error: None,
                },
            })
        }
        Err(err) => {
            runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
            let last_error = err.to_string();
            runtime.last_error = Some(last_error.clone());
            let fallback_summary = deterministic_summary(compacted);
            Some(CompactionUpdate {
                summary: fallback_summary,
                state: PromptCompactionState {
                    mode: PromptCompactionMode::Degraded,
                    auto_triggered: true,
                    degraded: true,
                    consecutive_failures: runtime.consecutive_failures,
                    circuit_open: runtime.circuit_open(),
                    model: Some(model.model_id().to_string()),
                    prompt_version: Some(COMPACTION_PROMPT_VERSION.to_string()),
                    source_message_count: compacted.len(),
                    last_error: Some(last_error),
                },
            })
        }
    }
}

fn deterministic_summary(compacted: &[Message]) -> Option<String> {
    let last = compacted.last()?;
    let content = compact(last.content.trim(), 180);
    Some(format!(
        "{} earlier message(s) compacted; latest compacted {} message: {}",
        compacted.len(),
        role_label(&last.role),
        content
    ))
}

fn compact(value: &str, max_chars: usize) -> String {
    let truncated: String = value.chars().take(max_chars).collect();
    if value.chars().count() > max_chars {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn role_label(role: &crate::core::types::Role) -> &'static str {
    match role {
        crate::core::types::Role::System => "system",
        crate::core::types::Role::User => "user",
        crate::core::types::Role::Assistant => "assistant",
        crate::core::types::Role::Tool => "tool",
    }
}

async fn generate_summary(
    model: &dyn ModelClient,
    compacted: &[Message],
    cancel_token: CancellationToken,
) -> Result<String, ModelError> {
    let mut prompt_messages = Vec::with_capacity(compacted.len() + 1);
    prompt_messages.push(Message::system(
        "Summarize the following prior agent conversation for future task resume. Preserve user intent, decisions, tool results, files changed, open risks, and causal state. Return only the summary text.".to_string(),
    ));
    prompt_messages.extend_from_slice(compacted);

    let mut stream = model.stream(&prompt_messages, &[]);
    let mut summary = String::new();
    loop {
        let item = tokio::select! {
            biased;
            _ = cancel_token.cancelled() => {
                return Err(ModelError::StreamInterrupted("compaction cancelled".to_string()));
            }
            item = futures::StreamExt::next(&mut stream) => item,
        };
        let Some(item) = item else {
            break;
        };
        match item? {
            ModelEvent::TextDelta { text } => summary.push_str(&text),
            ModelEvent::Done => break,
            ModelEvent::ThinkingDelta { .. }
            | ModelEvent::ToolUseStart { .. }
            | ModelEvent::ToolUseDelta { .. }
            | ModelEvent::ToolUseDone { .. }
            | ModelEvent::Usage { .. } => {}
        }
    }

    let summary = summary.trim().to_string();
    if summary.is_empty() {
        Err(ModelError::RequestFailed(
            "compaction returned an empty summary".to_string(),
        ))
    } else {
        Ok(summary)
    }
}
