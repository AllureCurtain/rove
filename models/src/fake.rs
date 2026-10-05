use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use futures::StreamExt;
use futures::stream::BoxStream;

use crate::traits::{ModelClient, ModelClientId, ModelEvent};
use crate::{Message, ModelError, ModelToolSchema, Role, StopReason, Usage};

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
    /// Fail before producing any output, so a caller can exercise a retry
    /// budget deterministically and offline.
    Fail(ModelError),
    /// Emit some text and then fail, so a caller can prove that a turn which
    /// already produced output is not retried.
    TextThenFail { text: String, error: ModelError },
    /// Emit `text` and then stay in flight until `release` is notified.
    ///
    /// This is the smallest deterministic way to hold a turn open while the
    /// caller cancels the run: the gate replaces a wall-clock delay, so the
    /// abort-salvage race is decided by the test rather than by scheduling.
    /// Releasing the gate proves a request that completes after the cancel
    /// signal; never releasing it proves one that outlives the salvage window.
    /// `Notify` keeps a permit, so a release that happens before the turn
    /// reaches the gate is not lost.
    Gate {
        text: String,
        release: Arc<tokio::sync::Notify>,
    },
    /// Emit `text` and then stay in flight until `release` is notified, then
    /// fail with `error` instead of completing.
    ///
    /// [`FakeTurn::Gate`] decides *when* a held-open request settles; this one
    /// decides that it settles by failing. Together they make every outcome of
    /// the abort-salvage window an explicit test choice: completion inside the
    /// window, expiry, and a provider error inside it.
    GateThenFail {
        text: String,
        release: Arc<tokio::sync::Notify>,
        error: ModelError,
    },
}

impl FakeTurn {
    /// Emit `text` and then stay in flight for the rest of the process.
    ///
    /// This is [`FakeTurn::Gate`] with a release nobody signals: a scripted turn
    /// cannot carry a live `Notify` through configuration, and a caller that only
    /// needs the request to outlive one abort-salvage window has nothing to
    /// release. The outcome is still decided by the caller's stop rather than by
    /// scheduling, because the turn never completes on its own.
    pub fn hold(text: impl Into<String>) -> Self {
        Self::Gate {
            text: text.into(),
            release: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

/// Deterministic local model for smoke tests and demos.
pub struct FakeModelClient {
    response: String,
    turns: Mutex<Vec<FakeTurn>>,
    compatibility_text_tool_calls: bool,
    /// Number of calls, and the message view of the most recent one.
    ///
    /// Only the last request is retained: this client also backs long-running
    /// demo sessions, and keeping every prompt would retain the whole
    /// conversation. Tests that need to see what a recovery turn was told read
    /// the last prompt and count the calls.
    last_call: Mutex<FakeCallObservation>,
}

#[derive(Debug, Default)]
struct FakeCallObservation {
    calls: usize,
    messages: Option<Vec<Message>>,
}

impl FakeModelClient {
    pub fn new(response: String) -> Self {
        Self {
            response,
            turns: Mutex::new(Vec::new()),
            compatibility_text_tool_calls: false,
            last_call: Mutex::new(FakeCallObservation::default()),
        }
    }

    /// Explicit legacy fixture for product surfaces which intentionally feed
    /// JSON tool envelopes as assistant text (for example `fake-raw`).
    pub fn with_compatibility_text(response: String) -> Self {
        Self {
            response,
            turns: Mutex::new(Vec::new()),
            compatibility_text_tool_calls: true,
            last_call: Mutex::new(FakeCallObservation::default()),
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
            compatibility_text_tool_calls: false,
            last_call: Mutex::new(FakeCallObservation::default()),
        }
    }

    /// How many model calls this client has been asked to complete.
    pub fn call_count(&self) -> usize {
        self.last_call
            .lock()
            .expect("call observation mutex poisoned")
            .calls
    }

    /// The message view of the most recent call, if the client was called.
    pub fn last_messages(&self) -> Option<Vec<Message>> {
        self.last_call
            .lock()
            .expect("call observation mutex poisoned")
            .messages
            .clone()
    }
}

/// The events every scripted turn ends with.
fn turn_completion_events() -> Vec<Result<ModelEvent, ModelError>> {
    vec![
        Ok(ModelEvent::Usage {
            usage: Usage::default(),
        }),
        Ok(ModelEvent::StopReason {
            reason: StopReason::EndTurn,
        }),
        Ok(ModelEvent::Done),
    ]
}

fn turn_events(turn: FakeTurn) -> Vec<Result<ModelEvent, ModelError>> {
    match turn {
        FakeTurn::Text(text) => {
            let mut events = vec![Ok(ModelEvent::TextDelta { text })];
            events.extend(turn_completion_events());
            events
        }
        FakeTurn::ToolUse { id, name, args } => vec![
            Ok(ModelEvent::ToolUseStart {
                id: id.clone(),
                name: name.clone(),
            }),
            Ok(ModelEvent::ToolUseDone { id, name, args }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
            Ok(ModelEvent::StopReason {
                reason: StopReason::ToolUse,
            }),
            Ok(ModelEvent::Done),
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
            events.push(Ok(ModelEvent::StopReason {
                reason: StopReason::ToolUse,
            }));
            events.push(Ok(ModelEvent::Done));
            events
        }
        FakeTurn::Fail(error) => vec![Err(error)],
        FakeTurn::TextThenFail { text, error } => vec![
            Ok(ModelEvent::TextDelta { text }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
            Err(error),
        ],
        // `stream` intercepts a gate so it can suspend mid-turn; this arm keeps
        // the expansion total and describes exactly the events the same turn
        // emits once its gate is released.
        FakeTurn::Gate { text, .. } => {
            let mut events = vec![Ok(ModelEvent::TextDelta { text })];
            events.extend(turn_completion_events());
            events
        }
        // Same expansion as the text-then-fail turn, minus the wait: the gate
        // decides when it runs, not what it emits.
        FakeTurn::GateThenFail { text, error, .. } => vec![
            Ok(ModelEvent::TextDelta { text }),
            Ok(ModelEvent::Usage {
                usage: Usage::default(),
            }),
            Err(error),
        ],
    }
}

#[async_trait]
impl ModelClient for FakeModelClient {
    fn stream(
        &self,
        messages: &[Message],
        _tools: &[ModelToolSchema],
    ) -> BoxStream<'_, Result<ModelEvent, ModelError>> {
        {
            let mut observation = self
                .last_call
                .lock()
                .expect("call observation mutex poisoned");
            observation.calls = observation.calls.saturating_add(1);
            observation.messages = Some(messages.to_vec());
        }
        if let Some(turn) = self.turns.lock().expect("turns mutex poisoned").pop() {
            // The delta is emitted before the gate so the caller can observe
            // text on screen and only then cancel the run; the completion
            // events follow whichever signal wins.
            return match turn {
                FakeTurn::Gate { text, release } => Box::pin(
                    futures::stream::iter([Ok(ModelEvent::TextDelta { text })]).chain(
                        futures::stream::once(async move {
                            release.notified().await;
                        })
                        .flat_map(|()| futures::stream::iter(turn_completion_events())),
                    ),
                ),
                FakeTurn::GateThenFail {
                    text,
                    release,
                    error,
                } => Box::pin(
                    futures::stream::iter([Ok(ModelEvent::TextDelta { text })]).chain(
                        futures::stream::once(async move {
                            release.notified().await;
                        })
                        .flat_map(move |()| {
                            futures::stream::iter([
                                Ok(ModelEvent::Usage {
                                    usage: Usage::default(),
                                }),
                                Err(error.clone()),
                            ])
                        }),
                    ),
                ),
                other => Box::pin(futures::stream::iter(turn_events(other))),
            };
        }

        let response = if messages
            .first()
            .map(|message| message.content.contains("You are the planner for rove."))
            .unwrap_or(false)
        {
            serde_json::json!({
                "goal": current_user_goal(messages).unwrap_or("fake goal"),
                "steps": [
                    { "id": "1", "title": "answer the request" }
                ]
            })
            .to_string()
        } else if let Some(tool_result) = latest_unconcluded_tool_result(messages) {
            tool_result.to_string()
        } else {
            // Echo a deterministic back-reference for any image content the
            // request carried, so an offline gate can prove an image block
            // reached the client without a network call.
            let image_blocks = messages
                .iter()
                .filter(|message| message.role == Role::User)
                .flat_map(|message| message.content_blocks.iter())
                .filter(|block| matches!(block, crate::ContentBlock::Image { .. }))
                .count();
            let mut response = if self.response == "fake response" {
                let goal = current_user_goal(messages).unwrap_or("message");
                format!("fake response: {goal}")
            } else {
                self.response.clone()
            };
            if image_blocks > 0 {
                response.push_str(&format!(" [image blocks received: {image_blocks}]"));
            }
            response
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
            Ok(ModelEvent::StopReason {
                reason: StopReason::EndTurn,
            }),
            Ok(ModelEvent::Done),
        ]))
    }

    fn model_id(&self) -> &str {
        "fake"
    }

    fn capabilities(&self) -> crate::ProviderCapabilities {
        crate::ProviderCapabilities {
            streaming: true,
            tool_calls: true,
            parallel_tool_calls: true,
            images: true,
        }
    }

    fn requires_terminal_event(&self) -> bool {
        true
    }

    fn history_protocol(&self) -> String {
        "fake".to_string()
    }

    fn compatibility_text_tool_calls(&self) -> bool {
        self.compatibility_text_tool_calls
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
    if !matches!(
        tool_message.tool_result_status,
        None | Some(crate::ToolResultStatus::Ok)
    ) || tool_message.content.trim_start().starts_with("Error:")
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

    #[test]
    fn scripted_fake_turns_emit_real_normalized_stop_reasons() {
        let text = turn_events(FakeTurn::Text("done".to_string()));
        let tool = turn_events(FakeTurn::ToolUse {
            id: "call-1".to_string(),
            name: "echo".to_string(),
            args: serde_json::json!({}),
        });
        assert!(text.iter().any(|event| matches!(
            event,
            Ok(ModelEvent::StopReason {
                reason: StopReason::EndTurn
            })
        )));
        assert!(tool.iter().any(|event| matches!(
            event,
            Ok(ModelEvent::StopReason {
                reason: StopReason::ToolUse
            })
        )));
    }

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

    #[tokio::test]
    async fn the_client_reports_its_call_count_and_most_recent_prompt() {
        let model = FakeModelClient::with_turns(
            "unscripted".to_string(),
            vec![
                FakeTurn::Text(String::new()),
                FakeTurn::Text("recovered".to_string()),
            ],
        );

        assert_eq!(model.call_count(), 0);
        assert!(model.last_messages().is_none());

        let first = model
            .stream(&[Message::user("ask")], &[])
            .collect::<Vec<_>>()
            .await;
        assert_eq!(model.call_count(), 1);
        assert_eq!(
            model.last_messages().expect("a recorded prompt"),
            vec![Message::user("ask")]
        );

        let nudge = Message::system("nudge");
        let second = model
            .stream(&[Message::user("ask"), nudge.clone()], &[])
            .collect::<Vec<_>>()
            .await;
        assert_eq!(model.call_count(), 2);
        assert_eq!(
            model.last_messages().expect("a recorded prompt"),
            vec![Message::user("ask"), nudge],
            "only the most recent prompt is retained"
        );

        assert!(matches!(
            &first[0],
            Ok(ModelEvent::TextDelta { text }) if text.is_empty()
        ));
        assert!(matches!(
            &second[0],
            Ok(ModelEvent::TextDelta { text }) if text == "recovered"
        ));
    }
}
