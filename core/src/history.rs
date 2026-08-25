//! Model-visible conversation history items.
//!
//! This module answers the question "what enters the model context on the
//! next request?" — everything here is replayable verbatim, while pure
//! presentation/audit events (`StreamEvent`) never reach a model request.
//!
//! Mirrors codex's `ResponseItem` vs `EventMsg` separation:
//!
//! - [`HistoryItem`] — model-visible content (codex `ResponseItem`)
//! - UI/status notifications stay in `rove_runtime::events::StreamEvent`
//!   (codex `EventMsg`)
//!
//! Rove reuses the normalized protocol types ([`Message`], [`ToolCallRef`],
//! [`Usage`]) instead of inventing parallel shapes: an assistant message
//! already carries its tool calls, and a role-`Tool` message already carries
//! its tool result.

use serde::{Deserialize, Serialize};

use rove_models::Message;

/// A model-visible, replayable item of conversation history.
///
/// Replaying every item of one run through [`history_to_messages`] must
/// reproduce exactly the `Vec<Message>` the kernel held when the run ended.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryItem {
    /// One provider-neutral conversation message. User input, assistant
    /// output (with any requested tool calls), and tool results are all
    /// messages under rove's normalized protocol.
    Message(Message),
    /// Compaction marker: `summary` replaces the covered prefix in future
    /// model requests. The original covered messages remain in the trace,
    /// so audits never lose them (Phase 8 wires the runtime behavior).
    Compacted(CompactedItem),
    /// Per-turn model/provider metadata recorded for provenance. It is not
    /// projected into model requests.
    TurnContext(TurnContextItem),
}

/// Summary produced by context compaction that replaces a covered history
/// range for subsequent model turns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactedItem {
    pub summary: String,
    /// Number of history messages the summary replaces.
    pub covered_messages: u32,
}

/// Metadata about the model turn configuration active for a stretch of
/// history. Provenance only; never projected into a request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TurnContextItem {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub provider: String,
}

/// Project replayable history into the provider-neutral message list a model
/// request consumes. `TurnContext` items contribute nothing.
pub fn history_to_messages(items: &[HistoryItem]) -> Vec<Message> {
    let mut messages = Vec::new();
    for item in items {
        match item {
            HistoryItem::Message(message) => messages.push(message.clone()),
            // Compaction replaces the *covered* messages rather than adding
            // to them; the runtime compactor performs the actual replacement
            // before persisting, so a stored Compacted item only contributes
            // its summary as a user-visible system-style note here.
            HistoryItem::Compacted(compacted) => {
                messages.push(Message::system(format!(
                    "[conversation compacted] {}",
                    compacted.summary
                )));
            }
            HistoryItem::TurnContext(_) => {}
        }
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use rove_models::{Role, ToolCallRef};

    #[test]
    fn messages_round_trip_through_history_items() {
        let items = vec![
            HistoryItem::Message(Message::user("fix the bug")),
            HistoryItem::Message(Message::assistant_with_tool_calls(
                "on it",
                vec![ToolCallRef {
                    id: "call_1".to_string(),
                    name: "fs_read".to_string(),
                    args: serde_json::json!({"path": "a.rs"}),
                }],
            )),
            HistoryItem::Message(Message::tool("file body", Some("call_1".to_string()))),
            HistoryItem::Message(Message::assistant("done")),
        ];

        let messages = history_to_messages(&items);

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].role, Role::User);
        assert_eq!(messages[1].tool_calls.len(), 1);
        assert_eq!(messages[2].role, Role::Tool);
        assert_eq!(messages[3].content, "done");
    }

    #[test]
    fn serialized_items_deserialize_without_kind_ambiguity() {
        let item = HistoryItem::Message(Message::user("hi"));
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"kind\":\"message\""));
        let round: HistoryItem = serde_json::from_str(&json).unwrap();
        assert_eq!(round, item);
    }

    #[test]
    fn turn_context_and_compacted_project_deterministically() {
        let items = vec![
            HistoryItem::TurnContext(TurnContextItem {
                model: "fake".to_string(),
                provider: "fake-provider".to_string(),
            }),
            HistoryItem::Compacted(CompactedItem {
                summary: "earlier work summarized".to_string(),
                covered_messages: 6,
            }),
        ];
        let messages = history_to_messages(&items);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.contains("earlier work summarized"));
    }
}
