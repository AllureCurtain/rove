use std::collections::HashMap;
use std::path::Path;

use crate::core::context::estimate_messages_tokens;
use crate::core::engine::planned_step_failure_message;
use crate::core::events::StreamEvent;
use crate::core::types::{
    CallId, JobId, Message, PromptCheckpoint, PromptCompactionMode, PromptCompactionState, Role,
    RunId, SessionId, TaskPlan, TaskState, TerminationReason, ToolMutation, Usage,
};
use crate::core::workspace::Workspace;

use super::report::{RunReport, write_report};
use super::store::StateStore;

const CHECKPOINT_TAIL_MESSAGES: usize = 12;
const CHECKPOINT_SUMMARY_CHARS: usize = 180;

pub struct RunArtifactRecorder {
    session_id: SessionId,
    job_id: JobId,
    run_id: RunId,
    goal: String,
    initial_step: u32,
    history: Vec<Message>,
    summary: Option<String>,
    plan: Option<TaskPlan>,
    steps: u32,
    tool_calls: u32,
    tool_failures: u32,
    tool_mutations: Vec<ToolMutation>,
    total_usage: Usage,
    final_reason: TerminationReason,
    final_output: Option<String>,
    pending_tool_use_ids: HashMap<CallId, Option<String>>,
    last_event_seq: Option<u64>,
    compaction: PromptCompactionState,
}

impl RunArtifactRecorder {
    pub fn new(
        session_id: SessionId,
        job_id: JobId,
        run_id: RunId,
        goal: String,
        resume_state: Option<&TaskState>,
    ) -> Self {
        let mut history = resume_state
            .map(|state| state.history.clone())
            .unwrap_or_default();
        let needs_current_user_message = history
            .last()
            .map(|message| message.role != Role::User || message.content != goal)
            .unwrap_or(true);
        if needs_current_user_message {
            history.push(Message::user(goal.clone()));
        }
        Self {
            session_id,
            job_id,
            run_id,
            goal,
            initial_step: resume_state.map(|state| state.step).unwrap_or(0),
            history,
            summary: resume_state.and_then(|state| state.summary.clone()),
            plan: resume_state.and_then(|state| state.plan.clone()),
            steps: 0,
            tool_calls: 0,
            tool_failures: 0,
            tool_mutations: Vec::new(),
            total_usage: Usage::default(),
            final_reason: TerminationReason::Error,
            final_output: None,
            pending_tool_use_ids: HashMap::new(),
            last_event_seq: None,
            compaction: PromptCompactionState::default(),
        }
    }

    pub async fn record_event(&mut self, event: &StreamEvent, state_store: &StateStore) {
        self.refresh_last_event_seq(state_store).await;
        match event {
            StreamEvent::RunStarted { .. } => {
                self.write_snapshot(state_store).await;
            }
            StreamEvent::LlmMessage {
                full,
                usage,
                tool_calls,
            } => {
                self.steps += 1;
                if tool_calls.is_empty() {
                    self.history.push(Message::assistant(full.clone()));
                } else {
                    self.history.push(Message::assistant_with_tool_calls(
                        full.clone(),
                        tool_calls.clone(),
                    ));
                }
                self.total_usage.prompt_tokens += usage.prompt_tokens;
                self.total_usage.completion_tokens += usage.completion_tokens;
                self.total_usage.total_tokens += usage.total_tokens;
                self.write_snapshot(state_store).await;
            }
            StreamEvent::ToolCallStarted {
                call_id,
                tool_use_id,
                ..
            } => {
                self.tool_calls += 1;
                self.pending_tool_use_ids
                    .insert(*call_id, tool_use_id.clone());
                self.write_snapshot(state_store).await;
            }
            StreamEvent::ToolCallCompleted { call_id, result } => {
                let tool_use_id = self.pending_tool_use_ids.remove(call_id).flatten();
                self.history
                    .push(Message::tool(result.output.clone(), tool_use_id));
                self.tool_mutations.extend(result.mutations.clone());
                self.write_snapshot(state_store).await;
            }
            StreamEvent::ToolCallFailed { call_id, error } => {
                self.tool_failures += 1;
                let tool_use_id = self.pending_tool_use_ids.remove(call_id).flatten();
                self.history
                    .push(Message::tool(format!("Error: {error}"), tool_use_id));
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PlanCreated { plan } => {
                self.plan = Some(plan.clone());
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PlanStepCompleted { step, .. } => {
                if let Some(active_plan) = self.plan.as_mut()
                    && let Some(saved_step) = active_plan
                        .steps
                        .iter_mut()
                        .find(|saved_step| saved_step.id == step.id)
                {
                    saved_step.done = true;
                    active_plan.current_step = active_plan
                        .steps
                        .iter()
                        .position(|saved_step| !saved_step.done)
                        .unwrap_or(active_plan.steps.len());
                }
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PlanStepFailed { step, reason, .. } => {
                self.history
                    .push(Message::user(planned_step_failure_message(
                        &step.title,
                        reason,
                    )));
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PromptCompacted { summary, state } => {
                if let Some(summary) = summary.clone() {
                    self.summary = Some(summary);
                }
                self.compaction = state.clone();
                self.write_snapshot(state_store).await;
            }
            StreamEvent::RunCompleted { reason, output } => {
                self.final_reason = reason.clone();
                self.final_output = output.clone();
                if self.summary.is_none() {
                    self.summary = self
                        .final_output
                        .as_ref()
                        .map(|output| truncate_summary(output));
                }
                self.write_snapshot(state_store).await;
            }
            _ => {}
        }
    }

    pub async fn finalize(
        &mut self,
        state_store: &StateStore,
        workspace: &Workspace,
        model_id: &str,
        run_dir: &Path,
    ) {
        if self.summary.is_none() {
            self.summary = self
                .final_output
                .as_ref()
                .map(|output| truncate_summary(output));
        }
        self.write_snapshot(state_store).await;
        self.write_report(state_store, workspace, model_id, run_dir)
            .await;
    }

    async fn write_snapshot(&self, state_store: &StateStore) {
        let state = TaskState {
            schema_version: 1,
            session_id: self.session_id,
            job_id: self.job_id,
            run_id: self.run_id,
            goal: self.goal.clone(),
            step: self.initial_step + self.steps,
            history: self.history.clone(),
            summary: self.summary.clone(),
            checkpoint: Some(self.prompt_checkpoint()),
            plan: self.plan.clone(),
        };
        if let Err(err) = state_store.write_task_state(&state).await {
            tracing::warn!("Failed to write task_state.json: {}", err);
        }
    }

    async fn refresh_last_event_seq(&mut self, state_store: &StateStore) {
        match state_store.index.last_event_seq(self.run_id) {
            Ok(0) => {}
            Ok(seq) => {
                self.last_event_seq = Some(seq);
            }
            Err(err) => {
                tracing::warn!("Failed to read last event sequence: {}", err);
            }
        }
    }

    async fn write_report(
        &self,
        state_store: &StateStore,
        workspace: &Workspace,
        model_id: &str,
        run_dir: &Path,
    ) {
        let mut report = RunReport::new(
            self.session_id,
            self.job_id,
            self.run_id,
            workspace.root.clone(),
            workspace.kind.clone(),
            model_id.to_string(),
            self.final_reason.clone(),
        );
        report.steps = self.steps;
        report.total_usage = self.total_usage.clone();
        report.tool_calls = self.tool_calls;
        report.tool_failures = self.tool_failures;
        report.tool_mutations = self.tool_mutations.clone();
        report.output = self.final_output.clone();

        match write_report(run_dir, &report) {
            Ok(path) => {
                if let Err(err) = state_store
                    .record_report(
                        self.run_id,
                        path,
                        report.status.clone(),
                        termination_reason_label(&report.termination_reason).to_string(),
                    )
                    .await
                {
                    tracing::warn!("Failed to index report.json: {}", err);
                }
            }
            Err(err) => {
                tracing::warn!("Failed to write report.json: {}", err);
            }
        }
    }

    fn prompt_checkpoint(&self) -> PromptCheckpoint {
        let step = self.initial_step + self.steps;
        let compacted_history_messages =
            self.history.len().saturating_sub(CHECKPOINT_TAIL_MESSAGES);
        let mut preserved_tail: Vec<_> = self
            .history
            .iter()
            .rev()
            .take(CHECKPOINT_TAIL_MESSAGES)
            .cloned()
            .collect();
        preserved_tail.reverse();
        let compacted = &self.history[..compacted_history_messages];
        let summary = self
            .summary
            .clone()
            .or_else(|| checkpoint_summary(compacted));
        let token_estimate = estimate_messages_tokens(&preserved_tail)
            + summary
                .as_ref()
                .map(|summary| summary.chars().count().div_ceil(4))
                .unwrap_or_default();

        PromptCheckpoint {
            summary,
            preserved_tail,
            plan: self.plan.clone(),
            session_memory_pointer: Some(format!(".rove/memory/sessions/{}.md", self.session_id)),
            durable_memory_pointer: Some(".rove/memory/MEMORY.md".to_string()),
            last_step: step,
            last_event_seq: self.last_event_seq,
            token_estimate,
            compacted_history_messages,
            compaction: self.checkpoint_compaction_state(compacted_history_messages),
        }
    }

    fn checkpoint_compaction_state(
        &self,
        compacted_history_messages: usize,
    ) -> PromptCompactionState {
        if self.compaction.auto_triggered || self.compaction.degraded {
            return self.compaction.clone();
        }
        PromptCompactionState {
            mode: if compacted_history_messages > 0 {
                PromptCompactionMode::Deterministic
            } else {
                PromptCompactionMode::None
            },
            auto_triggered: compacted_history_messages > 0,
            degraded: false,
            consecutive_failures: 0,
            circuit_open: false,
            model: None,
            prompt_version: None,
            source_message_count: compacted_history_messages,
            last_error: None,
        }
    }
}

fn truncate_summary(output: &str) -> String {
    let summary = output.trim();
    if summary.is_empty() {
        "completed".to_string()
    } else {
        summary.chars().take(120).collect()
    }
}

fn checkpoint_summary(compacted: &[Message]) -> Option<String> {
    let last = compacted.last()?;
    let content = compact(last.content.trim(), CHECKPOINT_SUMMARY_CHARS);
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

fn role_label(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    }
}

fn termination_reason_label(reason: &TerminationReason) -> &'static str {
    match reason {
        TerminationReason::Final => "final",
        TerminationReason::StepLimit => "step_limit",
        TerminationReason::TokenLimit => "token_limit",
        TerminationReason::TimeLimit => "time_limit",
        TerminationReason::Error => "error",
        TerminationReason::Cancelled => "cancelled",
    }
}
