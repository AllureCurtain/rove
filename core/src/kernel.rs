use futures::future::BoxFuture;
use futures::stream::{BoxStream, StreamExt};
use rove_models::{Message, ModelError, Usage};
use tokio_util::sync::CancellationToken;

use crate::model_turn::ModelTurn;
use crate::{Action, ToolCallAction};

/// Independent limits enforced by the runtime-neutral Agent kernel.
#[derive(Debug, Clone, Copy, Default)]
pub struct KernelLimits {
    pub max_model_turns: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub max_repairs: Option<u32>,
}

/// Mutable conversation state owned by the shared Agent kernel.
#[derive(Debug, Clone, Default)]
pub struct KernelState {
    pub history: Vec<Message>,
    pub usage: Usage,
    pub model_turns: u32,
    pub tool_calls: u32,
    pub repairs: u32,
}

impl KernelState {
    pub fn new(history: Vec<Message>) -> Self {
        Self {
            history,
            ..Self::default()
        }
    }
}

#[derive(Debug)]
pub enum KernelTermination<S> {
    Final { output: String },
    ModelTurnLimit,
    ToolCallLimit,
    RepairLimit,
    Cancelled,
    ModelFailed(ModelError),
    IncompleteBeforeModelTurn,
    IncompleteModelTurn,
    IncompleteToolTurn,
    Extension { reason: S, output: Option<String> },
}

#[derive(Debug)]
pub struct KernelResult<S, O> {
    pub termination: KernelTermination<S>,
    pub state: KernelState,
    pub extension: O,
}

#[derive(Debug)]
pub enum KernelItem<E, S, O> {
    Event(E),
    Finished(KernelResult<S, O>),
}

/// Result of a before/after extension callback.
#[derive(Debug)]
pub enum KernelHook<T, E, S> {
    Continue {
        value: T,
        events: Vec<E>,
    },
    Stop {
        reason: S,
        output: Option<String>,
        events: Vec<E>,
    },
}

impl<T, E, S> KernelHook<T, E, S> {
    pub fn continue_with(value: T) -> Self {
        Self::Continue {
            value,
            events: Vec::new(),
        }
    }

    pub fn continue_with_events(value: T, events: Vec<E>) -> Self {
        Self::Continue { value, events }
    }

    pub fn stop(reason: S, output: Option<String>) -> Self {
        Self::Stop {
            reason,
            output,
            events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelFinalAction {
    Complete,
    Continue,
}

#[derive(Debug, Clone)]
pub enum KernelToolAction {
    Call(ToolCallAction),
    Batch(Vec<ToolCallAction>),
}

impl KernelToolAction {
    pub fn call_count(&self) -> u32 {
        match self {
            Self::Call(_) => 1,
            Self::Batch(calls) => u32::try_from(calls.len()).unwrap_or(u32::MAX),
        }
    }
}

#[derive(Debug)]
pub enum KernelModelTurnItem<E> {
    Event(E),
    Finished(ModelTurn),
    Cancelled,
    Failed(ModelError),
}

#[derive(Debug)]
pub enum KernelBeforeModelTurnItem<E, S> {
    Event(E),
    Ready(Vec<Message>),
    Stop { reason: S, output: Option<String> },
}

#[derive(Debug)]
pub enum KernelToolTurnItem<E, T> {
    Event(E),
    Finished(T),
    Cancelled,
}

/// Runtime-neutral extension plane used by embedded and durable execution.
///
/// The kernel owns the multi-turn state machine. Hosts supply context/model
/// preparation, tool execution, event translation, and before/after policy
/// without moving workspace, persistence, approval, input, or UI types into
/// `rove-core`.
pub trait AgentKernelHost: Send {
    type Event: Send;
    type Stop: Send;
    type ToolOutcome: Send;
    type Output: Send;

    fn before_model_turn<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelBeforeModelTurnItem<Self::Event, Self::Stop>>;

    fn model_turn<'a>(
        &'a mut self,
        messages: Vec<Message>,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelModelTurnItem<Self::Event>>;

    fn after_model_turn<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        turn: &'a ModelTurn,
        cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<(), Self::Event, Self::Stop>>;

    fn tool_turn<'a>(
        &'a mut self,
        action: KernelToolAction,
        cancel_token: CancellationToken,
    ) -> BoxStream<'a, KernelToolTurnItem<Self::Event, Self::ToolOutcome>>;

    fn tool_history(&mut self, full_response: &str, outcome: &Self::ToolOutcome) -> Vec<Message>;

    fn after_tool_turn<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        outcome: &'a Self::ToolOutcome,
        cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<(), Self::Event, Self::Stop>>;

    fn after_final<'a>(
        &'a mut self,
        state: &'a mut KernelState,
        output: &'a str,
        cancel_token: CancellationToken,
    ) -> BoxFuture<'a, KernelHook<KernelFinalAction, Self::Event, Self::Stop>>;

    fn malformed_retry_message(&self, reason: &str) -> String {
        format!("Your previous output could not be parsed: {reason}. Please try again.")
    }

    fn finish_output(&mut self, state: &KernelState) -> Self::Output;
}

/// Drive one complete model/tool conversation through a host-supplied
/// extension plane.
pub fn run_agent_kernel<'a, H>(
    mut host: H,
    mut state: KernelState,
    limits: KernelLimits,
    cancel_token: CancellationToken,
) -> BoxStream<'a, KernelItem<H::Event, H::Stop, H::Output>>
where
    H: AgentKernelHost + 'a,
    H::Event: 'a,
    H::Stop: 'a,
    H::ToolOutcome: 'a,
    H::Output: 'a,
{
    Box::pin(async_stream::stream! {
        loop {
            if cancel_token.is_cancelled() {
                yield finished_item(&mut host, state, KernelTermination::Cancelled);
                return;
            }
            if limits
                .max_model_turns
                .is_some_and(|limit| state.model_turns >= limit)
            {
                yield finished_item(&mut host, state, KernelTermination::ModelTurnLimit);
                return;
            }

            let messages = {
                let mut before_model =
                    host.before_model_turn(&mut state, cancel_token.clone());
                loop {
                    let item = tokio::select! {
                        biased;
                        _ = cancel_token.cancelled() => None,
                        item = before_model.next() => Some(item),
                    };
                    let Some(item) = item else {
                        drop(before_model);
                        yield finished_item(&mut host, state, KernelTermination::Cancelled);
                        return;
                    };
                    match item {
                        Some(KernelBeforeModelTurnItem::Event(event)) => {
                            yield KernelItem::Event(event);
                        }
                        Some(KernelBeforeModelTurnItem::Ready(messages)) => break messages,
                        Some(KernelBeforeModelTurnItem::Stop { reason, output }) => {
                            drop(before_model);
                            yield finished_item(
                                &mut host,
                                state,
                                KernelTermination::Extension { reason, output },
                            );
                            return;
                        }
                        None => {
                            drop(before_model);
                            yield finished_item(
                                &mut host,
                                state,
                                KernelTermination::IncompleteBeforeModelTurn,
                            );
                            return;
                        }
                    }
                }
            };

            if cancel_token.is_cancelled() {
                yield finished_item(&mut host, state, KernelTermination::Cancelled);
                return;
            }
            state.model_turns = state.model_turns.saturating_add(1);

            let turn = {
                let mut stream = host.model_turn(messages, cancel_token.clone());
                loop {
                    match stream.next().await {
                        Some(KernelModelTurnItem::Event(event)) => {
                            yield KernelItem::Event(event);
                        }
                        Some(KernelModelTurnItem::Finished(turn)) => break turn,
                        Some(KernelModelTurnItem::Cancelled) => {
                            drop(stream);
                            yield finished_item(&mut host, state, KernelTermination::Cancelled);
                            return;
                        }
                        Some(KernelModelTurnItem::Failed(error)) => {
                            drop(stream);
                            yield finished_item(
                                &mut host,
                                state,
                                KernelTermination::ModelFailed(error),
                            );
                            return;
                        }
                        None => {
                            drop(stream);
                            yield finished_item(
                                &mut host,
                                state,
                                KernelTermination::IncompleteModelTurn,
                            );
                            return;
                        }
                    }
                }
            };
            add_usage(&mut state.usage, &turn.usage);

            let after_model = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => None,
                result = host.after_model_turn(&mut state, &turn, cancel_token.clone()) => Some(result),
            };
            let Some(after_model) = after_model else {
                yield finished_item(&mut host, state, KernelTermination::Cancelled);
                return;
            };
            match after_model {
                KernelHook::Continue { events, .. } => {
                    for event in events {
                        yield KernelItem::Event(event);
                    }
                }
                KernelHook::Stop {
                    reason,
                    output,
                    events,
                } => {
                    for event in events {
                        yield KernelItem::Event(event);
                    }
                    yield finished_item(
                        &mut host,
                        state,
                        KernelTermination::Extension { reason, output },
                    );
                    return;
                }
            }

            let tool_action = match turn.action.clone() {
                Action::Final { text } => {
                    state.history.push(Message::assistant(text.clone()));
                    let after_final = tokio::select! {
                        biased;
                        _ = cancel_token.cancelled() => None,
                        result = host.after_final(&mut state, &text, cancel_token.clone()) => Some(result),
                    };
                    let Some(after_final) = after_final else {
                        yield finished_item(&mut host, state, KernelTermination::Cancelled);
                        return;
                    };
                    match after_final {
                        KernelHook::Continue { value, events } => {
                            for event in events {
                                yield KernelItem::Event(event);
                            }
                            match value {
                                KernelFinalAction::Complete => {
                                    yield finished_item(
                                        &mut host,
                                        state,
                                        KernelTermination::Final { output: text },
                                    );
                                    return;
                                }
                                KernelFinalAction::Continue => continue,
                            }
                        }
                        KernelHook::Stop {
                            reason,
                            output,
                            events,
                        } => {
                            for event in events {
                                yield KernelItem::Event(event);
                            }
                            yield finished_item(
                                &mut host,
                                state,
                                KernelTermination::Extension { reason, output },
                            );
                            return;
                        }
                    }
                }
                Action::Malformed { reason } => {
                    if limits
                        .max_repairs
                        .is_some_and(|limit| state.repairs >= limit)
                    {
                        yield finished_item(&mut host, state, KernelTermination::RepairLimit);
                        return;
                    }
                    state.repairs = state.repairs.saturating_add(1);
                    state
                        .history
                        .push(Message::assistant(turn.full_response.clone()));
                    state
                        .history
                        .push(Message::user(host.malformed_retry_message(&reason)));
                    continue;
                }
                Action::ToolCall {
                    call_id,
                    tool_use_id,
                    name,
                    args,
                } => KernelToolAction::Call(ToolCallAction {
                        call_id,
                        tool_use_id,
                        name,
                        args,
                    }),
                Action::ToolBatch { calls } => KernelToolAction::Batch(calls),
            };

            let call_count = tool_action.call_count();
            if limits
                .max_tool_calls
                .is_some_and(|limit| state.tool_calls.saturating_add(call_count) > limit)
            {
                yield finished_item(&mut host, state, KernelTermination::ToolCallLimit);
                return;
            }
            // Reserve an entire batch before dispatch so a limit can never
            // leave half of a model-requested batch executed.
            state.tool_calls = state.tool_calls.saturating_add(call_count);

            let outcome = {
                let mut tool_stream = host.tool_turn(tool_action, cancel_token.clone());
                loop {
                    match tool_stream.next().await {
                        Some(KernelToolTurnItem::Event(event)) => {
                            // Approval and input events must be forwarded while
                            // execution is waiting so an interface can resolve
                            // the corresponding live channel.
                            yield KernelItem::Event(event);
                        }
                        Some(KernelToolTurnItem::Finished(outcome)) => break outcome,
                        Some(KernelToolTurnItem::Cancelled) => {
                            drop(tool_stream);
                            yield finished_item(
                                &mut host,
                                state,
                                KernelTermination::Cancelled,
                            );
                            return;
                        }
                        None => {
                            drop(tool_stream);
                            yield finished_item(
                                &mut host,
                                state,
                                KernelTermination::IncompleteToolTurn,
                            );
                            return;
                        }
                    }
                }
            };

            state
                .history
                .extend(host.tool_history(&turn.full_response, &outcome));
            let after_tool = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => None,
                result = host.after_tool_turn(&mut state, &outcome, cancel_token.clone()) => Some(result),
            };
            let Some(after_tool) = after_tool else {
                yield finished_item(&mut host, state, KernelTermination::Cancelled);
                return;
            };
            match after_tool {
                KernelHook::Continue { events, .. } => {
                    for event in events {
                        yield KernelItem::Event(event);
                    }
                }
                KernelHook::Stop {
                    reason,
                    output,
                    events,
                } => {
                    for event in events {
                        yield KernelItem::Event(event);
                    }
                    yield finished_item(
                        &mut host,
                        state,
                        KernelTermination::Extension { reason, output },
                    );
                    return;
                }
            }
        }
    })
}

fn finished_item<H: AgentKernelHost>(
    host: &mut H,
    state: KernelState,
    termination: KernelTermination<H::Stop>,
) -> KernelItem<H::Event, H::Stop, H::Output> {
    let extension = host.finish_output(&state);
    KernelItem::Finished(KernelResult {
        termination,
        state,
        extension,
    })
}

fn add_usage(total: &mut Usage, current: &Usage) {
    total.prompt_tokens = total.prompt_tokens.saturating_add(current.prompt_tokens);
    total.completion_tokens = total
        .completion_tokens
        .saturating_add(current.completion_tokens);
    total.total_tokens = total.total_tokens.saturating_add(current.total_tokens);
    total.cached_tokens = total.cached_tokens.saturating_add(current.cached_tokens);
}
