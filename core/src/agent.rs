use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_stream::stream;
use futures::StreamExt;
use futures::future::{BoxFuture, join_all};
use futures::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use rove_models::{Message, ModelClient, Role, ToolCallRef};

use crate::{
    AgentError, AgentEvent, AgentKernelHost, AgentOutcome, AgentStopReason, AllowAllToolPolicy,
    KernelBeforeModelTurnItem, KernelFinalAction, KernelHook, KernelItem, KernelLimits,
    KernelModelTurnItem, KernelState, KernelTermination, KernelToolAction, KernelToolTurnItem,
    ToolCallAction, ToolContext, ToolInvocation, ToolOutput, ToolPolicy, ToolRegistry,
    run_agent_kernel,
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
            yield AgentEvent::Started;

            let host = EmbeddedKernelHost {
                model: self.model.as_ref(),
                tools: &self.tools,
                policy: self.policy.as_ref(),
                control: control.clone(),
            };
            let limits = KernelLimits {
                max_model_turns: Some(self.config.max_model_turns),
                max_tool_calls: Some(self.config.max_tool_calls),
            };
            let mut kernel = run_agent_kernel(
                host,
                KernelState::new(messages),
                limits,
                control.cancellation_token(),
            );
            while let Some(item) = kernel.next().await {
                match item {
                    KernelItem::Event(event) => yield event,
                    KernelItem::Finished(result) => {
                        let (reason, output, failure) = match result.termination {
                            KernelTermination::Final { output } => {
                                (AgentStopReason::Final, Some(output), None)
                            }
                            KernelTermination::ModelTurnLimit => {
                                (AgentStopReason::ModelTurnLimit, None, None)
                            }
                            KernelTermination::ToolCallLimit => {
                                (AgentStopReason::ToolCallLimit, None, None)
                            }
                            KernelTermination::Cancelled => {
                                (AgentStopReason::Cancelled, None, None)
                            }
                            KernelTermination::ModelFailed(error) => (
                                AgentStopReason::Error,
                                None,
                                Some(AgentError::Model(error)),
                            ),
                            KernelTermination::IncompleteBeforeModelTurn
                            | KernelTermination::IncompleteModelTurn
                            | KernelTermination::IncompleteToolTurn => (
                                AgentStopReason::Error,
                                None,
                                Some(AgentError::Incomplete),
                            ),
                            KernelTermination::Extension { reason, output } => {
                                (AgentStopReason::Error, output, Some(reason))
                            }
                        };
                        if let Some(error) = failure {
                            yield AgentEvent::Failed { error };
                        }
                        yield completed(reason, output, result.state);
                        return;
                    }
                }
            }
        })
    }
}

struct EmbeddedKernelHost<'a> {
    model: &'a dyn ModelClient,
    tools: &'a ToolRegistry,
    policy: &'a dyn ToolPolicy,
    control: AgentControl,
}

#[derive(Debug)]
struct EmbeddedToolRecord {
    invocation: ToolInvocation,
    result: Result<ToolOutput, crate::ToolError>,
}

#[derive(Debug)]
struct EmbeddedToolOutcome {
    records: Vec<EmbeddedToolRecord>,
}

enum EmbeddedExecution {
    Finished(EmbeddedToolRecord),
    Cancelled,
}

impl AgentKernelHost for EmbeddedKernelHost<'_> {
    type Event = AgentEvent;
    type Stop = AgentError;
    type ToolOutcome = EmbeddedToolOutcome;
    type Output = ();

    fn before_model_turn<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        _cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelBeforeModelTurnItem<Self::Event, Self::Stop>> {
        Box::pin(stream! {
            state.history.extend(self.control.drain_steering());
            yield KernelBeforeModelTurnItem::Ready(state.history.clone());
        })
    }

    fn model_turn<'a>(
        &'a mut self,
        messages: Vec<Message>,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelModelTurnItem<Self::Event>> {
        Box::pin(stream! {
            let mut turn = crate::model_turn::run_model_turn(
                self.model,
                messages,
                self.tools.model_schemas(),
                cancel_token,
            );
            while let Some(item) = turn.next().await {
                match item {
                    crate::model_turn::ModelTurnItem::Event(event) => {
                        yield KernelModelTurnItem::Event(event);
                    }
                    crate::model_turn::ModelTurnItem::Finished(turn) => {
                        yield KernelModelTurnItem::Finished(turn);
                        return;
                    }
                    crate::model_turn::ModelTurnItem::Cancelled => {
                        yield KernelModelTurnItem::Cancelled;
                        return;
                    }
                    crate::model_turn::ModelTurnItem::Failed(error) => {
                        yield KernelModelTurnItem::Failed(error);
                        return;
                    }
                }
            }
        })
    }

    fn after_model_turn<'a>(
        &'a mut self,
        _state: &'a mut KernelState,
        _turn: &'a crate::model_turn::ModelTurn,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<(), Self::Event, Self::Stop>> {
        Box::pin(async { KernelHook::continue_with(()) })
    }

    fn tool_turn<'a>(
        &'a mut self,
        action: KernelToolAction,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelToolTurnItem<Self::Event, Self::ToolOutcome>> {
        Box::pin(stream! {
            let calls = match action {
                KernelToolAction::Call(call) => vec![call],
                KernelToolAction::Batch(calls) => calls,
            };
            let parallel = calls.len() > 1
                && calls.iter().all(|call| {
                    self.tools.descriptor(&call.name).is_ok_and(|descriptor| {
                        descriptor.parallel_safe && !descriptor.destructive
                    })
                });
            let mut records = Vec::new();
            if parallel {
                for call in &calls {
                    yield KernelToolTurnItem::Event(AgentEvent::ToolCallStarted {
                        invocation: invocation_from(call.clone()),
                    });
                }
                let executions = join_all(calls.into_iter().map(|call| {
                    execute_embedded_tool(
                        self.tools,
                        self.policy,
                        invocation_from(call),
                        cancel_token.clone(),
                    )
                }))
                .await;
                for execution in executions {
                    match execution {
                        EmbeddedExecution::Finished(record) => records.push(record),
                        EmbeddedExecution::Cancelled => {
                            yield KernelToolTurnItem::Cancelled;
                            return;
                        }
                    }
                }
            } else {
                for call in calls {
                    yield KernelToolTurnItem::Event(AgentEvent::ToolCallStarted {
                        invocation: invocation_from(call.clone()),
                    });
                    match execute_embedded_tool(
                        self.tools,
                        self.policy,
                        invocation_from(call),
                        cancel_token.clone(),
                    )
                    .await
                    {
                        EmbeddedExecution::Finished(record) => {
                            let failed = record.result.is_err();
                            records.push(record);
                            if failed {
                                break;
                            }
                        }
                        EmbeddedExecution::Cancelled => {
                            yield KernelToolTurnItem::Cancelled;
                            return;
                        }
                    }
                }
            }

            for record in &records {
                match &record.result {
                    Ok(output) => {
                        yield KernelToolTurnItem::Event(AgentEvent::ToolCallCompleted {
                            invocation: record.invocation.clone(),
                            output: output.clone(),
                        });
                    }
                    Err(error) => {
                        yield KernelToolTurnItem::Event(AgentEvent::ToolCallFailed {
                            invocation: record.invocation.clone(),
                            error: error.clone(),
                        });
                    }
                }
            }
            yield KernelToolTurnItem::Finished(EmbeddedToolOutcome { records });
        })
    }

    fn tool_history(&mut self, full_response: &str, outcome: &Self::ToolOutcome) -> Vec<Message> {
        embedded_tool_history(full_response, outcome)
    }

    fn after_tool_turn<'a>(
        &'a mut self,
        _state: &'a mut KernelState,
        _outcome: &'a Self::ToolOutcome,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<(), Self::Event, Self::Stop>> {
        Box::pin(async { KernelHook::continue_with(()) })
    }

    fn after_final<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        _output: &'a str,
        _cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<KernelFinalAction, Self::Event, Self::Stop>> {
        Box::pin(async move {
            let follow_up = self.control.drain_follow_up();
            if follow_up.is_empty() {
                KernelHook::continue_with(KernelFinalAction::Complete)
            } else {
                state.history.extend(follow_up);
                KernelHook::continue_with(KernelFinalAction::Continue)
            }
        })
    }

    fn finish_output(&mut self, _state: &KernelState) -> Self::Output {}
}

async fn execute_embedded_tool(
    tools: &ToolRegistry,
    policy: &dyn ToolPolicy,
    invocation: ToolInvocation,
    cancel_token: CancellationToken,
) -> EmbeddedExecution {
    let descriptor = match tools.descriptor(&invocation.name) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            return EmbeddedExecution::Finished(EmbeddedToolRecord {
                invocation,
                result: Err(error),
            });
        }
    };
    let context = ToolContext::new(invocation.call_id, cancel_token.clone());
    let before = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return EmbeddedExecution::Cancelled,
        result = policy.before_tool(&invocation, &descriptor, &context) => result,
    };
    if let Err(error) = before {
        return EmbeddedExecution::Finished(EmbeddedToolRecord {
            invocation,
            result: Err(error),
        });
    }
    let result = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => return EmbeddedExecution::Cancelled,
        result = tools.execute(&invocation.name, invocation.args.clone(), &context) => result,
    };
    let result = match result {
        Ok(output) => {
            let after = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => return EmbeddedExecution::Cancelled,
                result = policy.after_tool(&invocation, &descriptor, &context, &output) => result,
            };
            match after {
                Ok(()) => Ok(output),
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    EmbeddedExecution::Finished(EmbeddedToolRecord { invocation, result })
}

fn embedded_tool_history(full_response: &str, outcome: &EmbeddedToolOutcome) -> Vec<Message> {
    let tool_calls = outcome
        .records
        .iter()
        .filter_map(|record| {
            record
                .invocation
                .tool_use_id
                .as_ref()
                .map(|id| ToolCallRef {
                    id: id.clone(),
                    name: record.invocation.name.clone(),
                    args: record.invocation.args.clone(),
                })
        })
        .collect::<Vec<_>>();
    let mut history = if tool_calls.is_empty() {
        vec![Message::assistant(full_response.to_string())]
    } else {
        vec![Message::assistant_with_tool_calls(
            full_response.to_string(),
            tool_calls,
        )]
    };
    for record in &outcome.records {
        let content = match &record.result {
            Ok(output) => output.content.clone(),
            Err(error) => error.to_string(),
        };
        history.push(Message::tool(
            content,
            record.invocation.tool_use_id.clone(),
        ));
    }
    history
}

fn invocation_from(call: ToolCallAction) -> ToolInvocation {
    ToolInvocation {
        call_id: call.call_id,
        tool_use_id: call.tool_use_id,
        name: call.name,
        args: call.args,
    }
}

fn completed(reason: AgentStopReason, output: Option<String>, state: KernelState) -> AgentEvent {
    AgentEvent::Completed {
        outcome: AgentOutcome {
            reason,
            output,
            usage: state.usage,
            model_turns: state.model_turns,
            tool_calls: state.tool_calls,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use futures::StreamExt;
    use rove_models::{FakeModelClient, FakeTurn};

    use super::{Agent, AgentConfig, AgentControl, AgentRequest};
    use crate::{
        AgentEvent, AgentStopReason, Tool, ToolContext, ToolDescriptor, ToolError, ToolOutput,
        ToolRegistry,
    };

    struct UppercaseTool(Arc<AtomicUsize>);

    struct DelayTool {
        parallel_safe: bool,
        calls: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Tool for DelayTool {
        fn schema(&self) -> ToolDescriptor {
            ToolDescriptor {
                name: "delay".to_string(),
                description: "Return a label after a bounded delay".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "label": { "type": "string" },
                        "delay_ms": { "type": "integer" }
                    },
                    "required": ["label", "delay_ms"],
                    "additionalProperties": false
                }),
                destructive: false,
                parallel_safe: self.parallel_safe,
                capability_id: None,
                capability: None,
            }
        }

        async fn execute(
            &self,
            args: serde_json::Value,
            _context: &ToolContext<'_>,
        ) -> Result<ToolOutput, ToolError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            let delay_ms = args["delay_ms"].as_u64().unwrap();
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(ToolOutput::text(args["label"].as_str().unwrap()))
        }
    }

    #[derive(Default)]
    struct CountingPolicy {
        before: AtomicUsize,
        after: AtomicUsize,
    }

    #[async_trait]
    impl crate::ToolPolicy for CountingPolicy {
        async fn before_tool(
            &self,
            _invocation: &crate::ToolInvocation,
            _descriptor: &ToolDescriptor,
            _context: &ToolContext<'_>,
        ) -> Result<(), ToolError> {
            self.before.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn after_tool(
            &self,
            _invocation: &crate::ToolInvocation,
            _descriptor: &ToolDescriptor,
            _context: &ToolContext<'_>,
            _output: &ToolOutput,
        ) -> Result<(), ToolError> {
            self.after.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

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
                capability_id: None,
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

    #[tokio::test]
    async fn queued_steer_is_visible_to_the_next_shared_kernel_turn() {
        let agent = Agent::new(
            Box::new(FakeModelClient::new("fake response".to_string())),
            ToolRegistry::new(),
            AgentConfig::default(),
        );
        let control = AgentControl::new();
        control.steer("redirected request");

        let events = agent
            .run(AgentRequest::new("original request"), control)
            .collect::<Vec<_>>()
            .await;

        assert!(matches!(
            events.last(),
            Some(AgentEvent::Completed { outcome })
                if outcome.output.as_deref() == Some("fake response: redirected request")
        ));
    }

    #[tokio::test]
    async fn embedded_parallel_safe_batch_runs_concurrently_with_ordered_events() {
        let calls = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let model = FakeModelClient::with_turns(
            "unused".to_string(),
            vec![
                FakeTurn::ToolBatch(vec![
                    (
                        "call-a".to_string(),
                        "delay".to_string(),
                        serde_json::json!({"label":"a","delay_ms":40}),
                    ),
                    (
                        "call-b".to_string(),
                        "delay".to_string(),
                        serde_json::json!({"label":"b","delay_ms":40}),
                    ),
                ]),
                FakeTurn::Text("done".to_string()),
            ],
        );
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(DelayTool {
            parallel_safe: true,
            calls: calls.clone(),
            active,
            peak: peak.clone(),
        }));
        let agent = Agent::new(Box::new(model), tools, AgentConfig::default());

        let events = agent.ask("run batch").collect::<Vec<_>>().await;
        let completed = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolCallCompleted { output, .. } => Some(output.content.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(completed, ["a", "b"]);
    }

    #[tokio::test]
    async fn embedded_non_parallel_safe_batch_runs_serially() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let model = FakeModelClient::with_turns(
            "unused".to_string(),
            vec![
                FakeTurn::ToolBatch(vec![
                    (
                        "call-a".to_string(),
                        "delay".to_string(),
                        serde_json::json!({"label":"a","delay_ms":10}),
                    ),
                    (
                        "call-b".to_string(),
                        "delay".to_string(),
                        serde_json::json!({"label":"b","delay_ms":10}),
                    ),
                ]),
                FakeTurn::Text("done".to_string()),
            ],
        );
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(DelayTool {
            parallel_safe: false,
            calls: Arc::new(AtomicUsize::new(0)),
            active,
            peak: peak.clone(),
        }));
        let agent = Agent::new(Box::new(model), tools, AgentConfig::default());

        let _ = agent.ask("run serial batch").collect::<Vec<_>>().await;

        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn embedded_batch_reserves_tool_budget_before_any_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let model = FakeModelClient::with_turns(
            "unused".to_string(),
            vec![FakeTurn::ToolBatch(vec![
                (
                    "call-a".to_string(),
                    "uppercase".to_string(),
                    serde_json::json!({"text":"a"}),
                ),
                (
                    "call-b".to_string(),
                    "uppercase".to_string(),
                    serde_json::json!({"text":"b"}),
                ),
            ])],
        );
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(UppercaseTool(calls.clone())));
        let agent = Agent::new(
            Box::new(model),
            tools,
            AgentConfig {
                max_tool_calls: 1,
                ..AgentConfig::default()
            },
        );

        let events = agent.ask("run over-budget batch").collect::<Vec<_>>().await;

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEvent::ToolCallStarted { .. }))
        );
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Completed { outcome })
                if outcome.reason == AgentStopReason::ToolCallLimit
                    && outcome.tool_calls == 0
        ));
    }

    #[tokio::test]
    async fn embedded_policy_runs_before_and_after_through_kernel_extension_plane() {
        let policy = Arc::new(CountingPolicy::default());
        let model = FakeModelClient::with_turns(
            "unused".to_string(),
            vec![
                FakeTurn::ToolUse {
                    id: "call-uppercase".to_string(),
                    name: "uppercase".to_string(),
                    args: serde_json::json!({"text":"rove"}),
                },
                FakeTurn::Text("done".to_string()),
            ],
        );
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(UppercaseTool(Arc::new(AtomicUsize::new(0)))));
        let agent =
            Agent::new(Box::new(model), tools, AgentConfig::default()).with_policy(policy.clone());

        let _ = agent.ask("uppercase").collect::<Vec<_>>().await;

        assert_eq!(policy.before.load(Ordering::SeqCst), 1);
        assert_eq!(policy.after.load(Ordering::SeqCst), 1);
    }
}
