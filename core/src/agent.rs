use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_stream::stream;
use futures::StreamExt;
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use rove_models::{Message, ModelClient, Role, Usage};

use crate::model_turn::{ModelTurnItem, run_model_turn};
use crate::{
    Action, AgentError, AgentEvent, AgentOutcome, AgentStopReason, AllowAllToolPolicy,
    ToolCallAction, ToolContext, ToolInvocation, ToolPolicy, ToolRegistry,
};

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub system_prompt: Option<String>,
    pub max_model_turns: u32,
    pub max_tool_calls: u32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            system_prompt: None,
            max_model_turns: 32,
            max_tool_calls: 128,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub messages: Vec<Message>,
    pub user_message: String,
}

impl AgentRequest {
    pub fn new(user_message: impl Into<String>) -> Self {
        Self {
            messages: Vec::new(),
            user_message: user_message.into(),
        }
    }
}

#[derive(Clone, Default)]
pub struct AgentControl {
    cancel_token: CancellationToken,
    steering: Arc<Mutex<VecDeque<Message>>>,
    follow_up: Arc<Mutex<VecDeque<Message>>>,
}

impl AgentControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    pub fn steer(&self, message: impl Into<String>) {
        lock_queue(&self.steering).push_back(Message::user(message));
    }

    pub fn follow_up(&self, message: impl Into<String>) {
        lock_queue(&self.follow_up).push_back(Message::user(message));
    }

    fn drain_steering(&self) -> Vec<Message> {
        lock_queue(&self.steering).drain(..).collect()
    }

    fn drain_follow_up(&self) -> Vec<Message> {
        lock_queue(&self.follow_up).drain(..).collect()
    }
}

fn lock_queue(queue: &Mutex<VecDeque<Message>>) -> std::sync::MutexGuard<'_, VecDeque<Message>> {
    queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A minimal in-memory Agent. Durable execution belongs to `rove-runtime`.
pub struct Agent {
    model: Box<dyn ModelClient>,
    tools: ToolRegistry,
    config: AgentConfig,
    policy: Arc<dyn ToolPolicy>,
}

impl Agent {
    pub fn new(model: Box<dyn ModelClient>, tools: ToolRegistry, config: AgentConfig) -> Self {
        Self {
            model,
            tools,
            config,
            policy: Arc::new(AllowAllToolPolicy),
        }
    }

    pub fn with_policy(mut self, policy: Arc<dyn ToolPolicy>) -> Self {
        self.policy = policy;
        self
    }

    pub fn ask<'a>(&'a self, user_message: impl Into<String>) -> BoxStream<'a, AgentEvent> {
        self.run(AgentRequest::new(user_message), AgentControl::new())
    }

    pub fn run<'a>(
        &'a self,
        request: AgentRequest,
        control: AgentControl,
    ) -> BoxStream<'a, AgentEvent> {
        Box::pin(stream! {
            let mut messages = request.messages;
            if let Some(system_prompt) = &self.config.system_prompt
                && !messages.iter().any(|message| message.role == Role::System)
            {
                messages.insert(0, Message::system(system_prompt.clone()));
            }
            messages.push(Message::user(request.user_message));

            let mut usage = Usage::default();
            let mut model_turns = 0_u32;
            let mut tool_calls = 0_u32;
            yield AgentEvent::Started;

            loop {
                messages.extend(control.drain_steering());
                if control.cancel_token.is_cancelled() {
                    yield completed(AgentStopReason::Cancelled, None, usage, model_turns, tool_calls);
                    return;
                }
                if model_turns >= self.config.max_model_turns {
                    yield completed(AgentStopReason::ModelTurnLimit, None, usage, model_turns, tool_calls);
                    return;
                }

                model_turns += 1;
                let mut turn_stream = run_model_turn(
                    self.model.as_ref(),
                    messages.clone(),
                    self.tools.model_schemas(),
                    control.cancellation_token(),
                );
                let turn = loop {
                    match turn_stream.next().await {
                        Some(ModelTurnItem::Event(event)) => yield event,
                        Some(ModelTurnItem::Finished(turn)) => break turn,
                        Some(ModelTurnItem::Cancelled) => {
                            yield completed(AgentStopReason::Cancelled, None, usage, model_turns, tool_calls);
                            return;
                        }
                        Some(ModelTurnItem::Failed(error)) => {
                            yield AgentEvent::Failed { error: AgentError::Model(error) };
                            yield completed(AgentStopReason::Error, None, usage, model_turns, tool_calls);
                            return;
                        }
                        None => {
                            yield AgentEvent::Failed { error: AgentError::Incomplete };
                            yield completed(AgentStopReason::Error, None, usage, model_turns, tool_calls);
                            return;
                        }
                    }
                };
                add_usage(&mut usage, &turn.usage);

                let calls = match turn.action {
                    Action::Final { text } => {
                        let follow_up = control.drain_follow_up();
                        if follow_up.is_empty() {
                            yield completed(
                                AgentStopReason::Final,
                                Some(text),
                                usage,
                                model_turns,
                                tool_calls,
                            );
                            return;
                        }
                        messages.push(Message::assistant(text));
                        messages.extend(follow_up);
                        continue;
                    }
                    Action::ToolCall {
                        call_id,
                        tool_use_id,
                        name,
                        args,
                    } => vec![ToolCallAction {
                            call_id,
                            tool_use_id,
                            name,
                            args,
                        }],
                    Action::ToolBatch { calls } => calls,
                    Action::Malformed { reason } => {
                        messages.push(Message::assistant(turn.full_response));
                        messages.push(Message::user(format!(
                            "Your previous output could not be parsed: {reason}. Please try again."
                        )));
                        continue;
                    }
                };

                messages.push(Message::assistant_with_tool_calls(
                    turn.full_response,
                    turn.tool_calls,
                ));
                for call in calls {
                    if tool_calls >= self.config.max_tool_calls {
                        yield completed(AgentStopReason::ToolCallLimit, None, usage, model_turns, tool_calls);
                        return;
                    }
                    tool_calls += 1;
                    let invocation = invocation_from(call);
                    yield AgentEvent::ToolCallStarted { invocation: invocation.clone() };
                    let descriptor = match self.tools.schema(&invocation.name) {
                        Ok(descriptor) => descriptor,
                        Err(error) => {
                            messages.push(Message::tool(error.to_string(), invocation.tool_use_id.clone()));
                            yield AgentEvent::ToolCallFailed { invocation, error };
                            continue;
                        }
                    };
                    let context = ToolContext::new(invocation.call_id, control.cancellation_token());
                    let before = tokio::select! {
                        biased;
                        _ = control.cancel_token.cancelled() => None,
                        result = self.policy.before_tool(&invocation, &descriptor, &context) => Some(result),
                    };
                    let Some(before) = before else {
                        yield completed(AgentStopReason::Cancelled, None, usage, model_turns, tool_calls);
                        return;
                    };
                    if let Err(error) = before {
                        messages.push(Message::tool(error.to_string(), invocation.tool_use_id.clone()));
                        yield AgentEvent::ToolCallFailed { invocation, error };
                        continue;
                    }
                    let result = tokio::select! {
                        biased;
                        _ = control.cancel_token.cancelled() => None,
                        result = self.tools.execute(&invocation.name, invocation.args.clone(), &context) => Some(result),
                    };
                    let Some(result) = result else {
                        yield completed(AgentStopReason::Cancelled, None, usage, model_turns, tool_calls);
                        return;
                    };
                    match result {
                        Ok(output) => {
                            if let Err(error) = self.policy.after_tool(&invocation, &descriptor, &context, &output).await {
                                messages.push(Message::tool(error.to_string(), invocation.tool_use_id.clone()));
                                yield AgentEvent::ToolCallFailed { invocation, error };
                            } else {
                                messages.push(Message::tool(output.content.clone(), invocation.tool_use_id.clone()));
                                yield AgentEvent::ToolCallCompleted { invocation, output };
                            }
                        }
                        Err(error) => {
                            messages.push(Message::tool(error.to_string(), invocation.tool_use_id.clone()));
                            yield AgentEvent::ToolCallFailed { invocation, error };
                        }
                    }
                }
            }
        })
    }
}

fn invocation_from(call: ToolCallAction) -> ToolInvocation {
    ToolInvocation {
        call_id: call.call_id,
        tool_use_id: call.tool_use_id,
        name: call.name,
        args: call.args,
    }
}

fn add_usage(total: &mut Usage, current: &Usage) {
    total.prompt_tokens = total.prompt_tokens.saturating_add(current.prompt_tokens);
    total.completion_tokens = total
        .completion_tokens
        .saturating_add(current.completion_tokens);
    total.total_tokens = total.total_tokens.saturating_add(current.total_tokens);
    total.cached_tokens = total.cached_tokens.saturating_add(current.cached_tokens);
}

fn completed(
    reason: AgentStopReason,
    output: Option<String>,
    usage: Usage,
    model_turns: u32,
    tool_calls: u32,
) -> AgentEvent {
    AgentEvent::Completed {
        outcome: AgentOutcome {
            reason,
            output,
            usage,
            model_turns,
            tool_calls,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use futures::StreamExt;
    use rove_models::{FakeModelClient, FakeTurn};

    use super::{Agent, AgentConfig, AgentControl, AgentRequest};
    use crate::{
        AgentEvent, AgentStopReason, Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput,
        ToolRegistry,
    };

    struct UppercaseTool(Arc<AtomicUsize>);

    #[async_trait]
    impl Tool for UppercaseTool {
        fn schema(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "uppercase".to_string(),
                description: "Uppercase text".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }),
                destructive: false,
                parallel_safe: true,
                capability: None,
            }
        }

        async fn execute(
            &self,
            args: serde_json::Value,
            _context: &ToolContext<'_>,
        ) -> Result<ToolOutput, ToolError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let text = args["text"].as_str().unwrap();
            Ok(ToolOutput::text(text.to_uppercase()))
        }
    }

    #[tokio::test]
    async fn fake_model_and_custom_tool_run_without_runtime_services() {
        let calls = Arc::new(AtomicUsize::new(0));
        let model = FakeModelClient::with_turns(
            "unused".to_string(),
            vec![
                FakeTurn::ToolUse {
                    id: "call-uppercase".to_string(),
                    name: "uppercase".to_string(),
                    args: serde_json::json!({"text":"rove"}),
                },
                FakeTurn::Text("ROVE".to_string()),
            ],
        );
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(UppercaseTool(calls.clone())));
        let agent = Agent::new(Box::new(model), tools, AgentConfig::default());

        let events = agent.ask("uppercase rove").collect::<Vec<_>>().await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolCallCompleted { output, .. } if output.content == "ROVE"
        )));
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Completed { outcome }) if outcome.output.as_deref() == Some("ROVE")
        ));
    }

    #[tokio::test]
    async fn cancellation_is_an_in_memory_terminal_outcome() {
        let agent = Agent::new(
            Box::new(FakeModelClient::new("unused".to_string())),
            ToolRegistry::new(),
            AgentConfig::default(),
        );
        let control = AgentControl::new();
        control.cancel();

        let events = agent
            .run(AgentRequest::new("stop"), control)
            .collect::<Vec<_>>()
            .await;

        assert!(matches!(
            events.last(),
            Some(AgentEvent::Completed { outcome })
                if outcome.reason == AgentStopReason::Cancelled
        ));
    }

    #[tokio::test]
    async fn queued_follow_up_continues_after_a_model_final() {
        let model = FakeModelClient::with_turns(
            "unused".to_string(),
            vec![
                FakeTurn::Text("first".to_string()),
                FakeTurn::Text("second".to_string()),
            ],
        );
        let agent = Agent::new(Box::new(model), ToolRegistry::new(), AgentConfig::default());
        let control = AgentControl::new();
        control.follow_up("continue");

        let events = agent
            .run(AgentRequest::new("start"), control)
            .collect::<Vec<_>>()
            .await;

        assert!(matches!(
            events.last(),
            Some(AgentEvent::Completed { outcome })
                if outcome.reason == AgentStopReason::Final
                    && outcome.output.as_deref() == Some("second")
                    && outcome.model_turns == 2
        ));
    }
}
