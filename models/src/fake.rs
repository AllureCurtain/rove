use std::sync::Mutex;

use async_trait::async_trait;
use futures::stream::BoxStream;

use crate::traits::{ModelClient, ModelClientId, ModelEvent};
use crate::{Message, ModelError, ModelToolSchema, Role, Usage};

/// Scripted turn for a `FakeModelClient` — one entry per LLM call.
#[derive(Debug, Clone)]
pub enum FakeTurn {
    /// Emit plain text as the assistant message.
    Text(String),
    /// Emit a single native tool-use call.
    ToolUse {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    /// Emit a batch of native tool-use calls in one assistant turn.
    ToolBatch(Vec<(String, String, serde_json::Value)>),
}

/// Deterministic local model for smoke tests and demos.
pub struct FakeModelClient {
    response: String,
    turns: Mutex<Vec<FakeTurn>>,
}

impl FakeModelClient {
    pub fn new(response: String) -> Self {
        Self {
            response,
            turns: Mutex::new(Vec::new()),
        }
    }

    /// Build a client that plays back a scripted sequence of turns. After the
    /// scripted turns are exhausted, the client falls back to emitting `response`.
    pub fn with_turns(response: String, turns: Vec<FakeTurn>) -> Self {
        let mut reversed = turns;
        reversed.reverse();
        Self {
            response,
            turns: Mutex::new(reversed),
        }
    }
}

fn turn_events(turn: FakeTurn) -> Vec<Result<ModelEvent, ModelError>> {
    match turn {
        FakeTurn::Text(text) => vec![
            Ok(ModelEvent::TextDelta { text }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
        ],
        FakeTurn::ToolUse { id, name, args } => vec![
            Ok(ModelEvent::ToolUseStart {
                id: id.clone(),
                name: name.clone(),
            }),
            Ok(ModelEvent::ToolUseDone { id, name, args }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
        ],
        FakeTurn::ToolBatch(calls) => {
            let mut events = Vec::with_capacity(calls.len() * 2 + 1);
            for (id, name, args) in calls {
                events.push(Ok(ModelEvent::ToolUseStart {
                    id: id.clone(),
                    name: name.clone(),
                }));
                events.push(Ok(ModelEvent::ToolUseDone { id, name, args }));
            }
            events.push(Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }));
            events
        }
    }
}

#[async_trait]
impl ModelClient for FakeModelClient {
    fn stream(
        &self,
        messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        if let Some(turn) = self.turns.lock().expect("turns mutex poisoned").pop() {
            return Box::pin(futures::stream::iter(turn_events(turn)));
        }

        let response = if messages
            .first()
            .map(|message| message.content.contains("You are the planner for rove."))
            .unwrap_or(false)
        {
            serde_json::json!({
                "goal": messages
                    .get(1)
                    .map(|message| message.content.trim_start_matches("Goal: ").to_string())
                    .unwrap_or_else(|| "fake goal".to_string()),
                "steps": [
                    { "id": "1", "title": "answer the request" }
                ]
            })
            .to_string()
        } else if let Some(tool_result) = latest_unconcluded_tool_result(messages) {
            tool_result.to_string()
        } else if self.response == "fake response" {
            format!(
                "fake response: {}",
                current_user_goal(messages).unwrap_or("message")
            )
        } else {
            self.response.clone()
        };
        Box::pin(futures::stream::iter([
            Ok(ModelEvent::TextDelta { text: response }),
            Ok(ModelEvent::Usage {
                usage: Usage {
                    prompt_tokens: 0,
                    completion_tokens: 0,
                    total_tokens: 0,
                    cached_tokens: 0,
                },
            }),
        ]))
    }

    fn model_id(&self) -> &str {
        "fake"
    }

    fn client_id(&self) -> ModelClientId {
        ModelClientId::new("fake", "local", self.model_id())
    }
}

fn latest_unconcluded_tool_result(messages: &[Message]) -> Option<&str> {
    let tool_index = messages
        .iter()
        .rposition(|message| message.role == Role::Tool)?;
    let tool_message = &messages[tool_index];
    if tool_message.content.trim_start().starts_with("Error:")
        || messages[tool_index + 1..]
            .iter()
            .any(|message| message.role == Role::Assistant)
    {
        return None;
    }
    Some(tool_message.content.as_str())
}

fn current_user_goal(messages: &[Message]) -> Option<&str> {
    let content = messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)?
        .content
        .trim();
    content
        .strip_prefix("Goal: ")
        .and_then(|rest| rest.lines().next())
        .map(str::trim)
        .filter(|goal| !goal.is_empty())
        .or_else(|| (!content.is_empty()).then_some(content))
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::*;

    #[tokio::test]
    async fn static_fake_response_concludes_an_unanswered_successful_tool_result() {
        let model =
            FakeModelClient::new(r#"{"tool":"echo","args":{"message":"ping"}}"#.to_string());
        let messages = vec![
            Message::assistant(r#"{"tool":"echo","args":{"message":"ping"}}"#),
            Message::tool("ping", None),
            Message::user("continue the current step"),
        ];

        let events = model.stream(&messages, &[]).collect::<Vec<_>>().await;

        assert!(matches!(
            &events[0],
            Ok(ModelEvent::TextDelta { text }) if text == "ping"
        ));
    }

    #[tokio::test]
    async fn static_fake_response_does_not_treat_failed_or_concluded_tools_as_new_results() {
        let raw = r#"{"tool":"echo","args":{"message":"ping"}}"#;
        let model = FakeModelClient::new(raw.to_string());
        let failed = vec![Message::tool("Error: invalid arguments", None)];
        let concluded = vec![
            Message::tool("ping", None),
            Message::assistant("already concluded"),
        ];

        let failed_events = model.stream(&failed, &[]).collect::<Vec<_>>().await;
        let concluded_events = model.stream(&concluded, &[]).collect::<Vec<_>>().await;

        assert!(matches!(
            &failed_events[0],
            Ok(ModelEvent::TextDelta { text }) if text == raw
        ));
        assert!(matches!(
            &concluded_events[0],
            Ok(ModelEvent::TextDelta { text }) if text == raw
        ));
    }
}
