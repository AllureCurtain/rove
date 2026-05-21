use async_stream::stream;
use futures::stream::{BoxStream, Stream, StreamExt};

use crate::core::context::ContextManager;
use crate::core::events::StreamEvent;
use crate::core::executor::Executor;
use crate::core::parser::parse_action;
use crate::core::types::{
    Action, JobId, Message, Role, RunId, RunRequest, TerminationReason, Usage,
};
use crate::models::traits::ModelClient;
use crate::state::trace::TraceWriter;
use crate::tools::registry::ToolRegistry;

/// Configuration for the engine's execution limits.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub max_steps: u32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { max_steps: 20 }
    }
}

/// The core engine that drives the agent loop.
///
/// Owns the model client, tool registry, context manager, and config.
/// Produces a `Stream<Item = StreamEvent>` that any interface can consume.
pub struct Engine {
    model: Box<dyn ModelClient>,
    registry: ToolRegistry,
    context_manager: ContextManager,
    config: EngineConfig,
}

impl Engine {
    pub fn new(
        model: Box<dyn ModelClient>,
        registry: ToolRegistry,
        context_manager: ContextManager,
        config: EngineConfig,
    ) -> Self {
        Self {
            model,
            registry,
            context_manager,
            config,
        }
    }

    /// Run the agent loop for a user message.
    ///
    /// Returns a stream of events. The stream completes when the run terminates.
    pub fn ask(
        &self,
        user_message: String,
        trace_writer: Option<TraceWriter>,
    ) -> impl Stream<Item = StreamEvent> + '_ {
        let req = RunRequest {
            session_id: crate::core::types::SessionId::new(),
            job_id: JobId::new(),
            run_id: RunId::new(),
            user_message,
        };

        self.run(req, trace_writer)
    }

    /// Run the agent loop for an explicit request.
    ///
    /// The caller owns run identity so persisted artifacts and streamed events stay aligned.
    pub fn run(
        &self,
        req: RunRequest,
        trace_writer: Option<TraceWriter>,
    ) -> impl Stream<Item = StreamEvent> + '_ {
        let job_id = req.job_id;
        let run_id = req.run_id;
        let user_message = req.user_message;

        stream! {
            let start_event = StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: user_message.clone(),
            };
            if let Some(ref tw) = trace_writer {
                let _ = tw.append(&start_event);
            }
            yield start_event;

            let mut history: Vec<Message> = Vec::new();
            let mut step: u32 = 0;

            loop {
                if step >= self.config.max_steps {
                    let event = StreamEvent::RunCompleted {
                        reason: TerminationReason::StepLimit,
                        output: None,
                    };
                    if let Some(ref tw) = trace_writer {
                        let _ = tw.append(&event);
                    }
                    yield event;
                    return;
                }
                step += 1;

                // 1. Build prompt
                let messages = self.context_manager.build(&user_message, &history);

                // 2. Call model (streaming)
                let mut full_response = String::new();
                let mut usage = Usage::default();
                let mut model_stream: BoxStream<'_, _> = self.model.stream(
                    &messages,
                    &self.registry.schemas(),
                );

                while let Some(chunk_result) = model_stream.next().await {
                    match chunk_result {
                        Ok(chunk) => {
                            if !chunk.delta.is_empty() {
                                full_response.push_str(&chunk.delta);
                                let event = StreamEvent::LlmChunk {
                                    delta: chunk.delta,
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&event);
                                }
                                yield event;
                            }
                            if let Some(u) = chunk.usage {
                                usage = u;
                            }
                        }
                        Err(e) => {
                            let event = StreamEvent::RunCompleted {
                                reason: TerminationReason::Error,
                                output: Some(format!("Model error: {}", e)),
                            };
                            if let Some(ref tw) = trace_writer {
                                let _ = tw.append(&event);
                            }
                            yield event;
                            return;
                        }
                    }
                }

                // Emit full message event
                let msg_event = StreamEvent::LlmMessage {
                    full: full_response.clone(),
                    usage: usage.clone(),
                };
                if let Some(ref tw) = trace_writer {
                    let _ = tw.append(&msg_event);
                }
                yield msg_event;

                // 3. Parse action
                let action = parse_action(&full_response);

                // 4. Handle action
                match action {
                    Action::Final { text } => {
                        let event = StreamEvent::RunCompleted {
                            reason: TerminationReason::Final,
                            output: Some(text),
                        };
                        if let Some(ref tw) = trace_writer {
                            let _ = tw.append(&event);
                        }
                        yield event;
                        return;
                    }
                    Action::ToolCall { call_id, name, args } => {
                        let start_event = StreamEvent::ToolCallStarted {
                            call_id,
                            name: name.clone(),
                            args: args.clone(),
                        };
                        if let Some(ref tw) = trace_writer {
                            let _ = tw.append(&start_event);
                        }
                        yield start_event;

                        let executor = Executor::new(&self.registry);
                        match executor.run(&name, args, call_id).await {
                            Ok(result) => {
                                // Add assistant message + tool result to history
                                history.push(Message {
                                    role: Role::Assistant,
                                    content: full_response.clone(),
                                });
                                history.push(Message {
                                    role: Role::Tool,
                                    content: result.output.clone(),
                                });

                                let event = StreamEvent::ToolCallCompleted {
                                    call_id,
                                    result,
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&event);
                                }
                                yield event;
                            }
                            Err(e) => {
                                // Feed error back to LLM
                                history.push(Message {
                                    role: Role::Assistant,
                                    content: full_response.clone(),
                                });
                                history.push(Message {
                                    role: Role::Tool,
                                    content: format!("Error: {}", e),
                                });

                                let event = StreamEvent::ToolCallFailed {
                                    call_id,
                                    error: e,
                                };
                                if let Some(ref tw) = trace_writer {
                                    let _ = tw.append(&event);
                                }
                                yield event;
                            }
                        }
                    }
                    Action::Malformed { reason } => {
                        // Feed parse failure back to LLM for self-correction
                        history.push(Message {
                            role: Role::Assistant,
                            content: full_response.clone(),
                        });
                        history.push(Message {
                            role: Role::User,
                            content: format!(
                                "Your previous output could not be parsed: {}. Please try again.",
                                reason
                            ),
                        });
                    }
                }
            }
        }
    }
}
