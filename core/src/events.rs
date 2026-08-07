use rove_models::{AssistantTurn, ToolCallRef, Usage};

use crate::{AgentError, ToolInvocation, ToolOutput};

#[derive(Debug, Clone)]
pub enum AgentEvent {
    Started,
    ModelStatus {
        status: String,
        message: String,
    },
    TextDelta {
        delta: String,
    },
    ModelMessage {
        full: String,
        usage: Usage,
        tool_calls: Vec<ToolCallRef>,
        assistant_turn: Box<AssistantTurn>,
    },
    ToolCallStarted {
        invocation: ToolInvocation,
    },
    ToolCallCompleted {
        invocation: ToolInvocation,
        output: ToolOutput,
    },
    ToolCallFailed {
        invocation: ToolInvocation,
        error: crate::ToolError,
    },
    Failed {
        error: AgentError,
    },
    Completed {
        outcome: AgentOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStopReason {
    Final,
    ModelTurnLimit,
    ToolCallLimit,
    Cancelled,
    Error,
}

#[derive(Debug, Clone)]
pub struct AgentOutcome {
    pub reason: AgentStopReason,
    pub output: Option<String>,
    pub usage: Usage,
    pub model_turns: u32,
    pub tool_calls: u32,
}
