use std::path::Path;

use crate::core::engine::planned_step_failure_message;
use crate::core::events::StreamEvent;
use crate::core::types::{
    JobId, Message, Role, RunId, SessionId, TaskPlan, TaskState, TerminationReason, Usage,
};
use crate::core::workspace::Workspace;

use super::report::{RunReport, write_report};
use super::store::StateStore;

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
    total_usage: Usage,
    final_reason: TerminationReason,
    final_output: Option<String>,
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
            history.push(Message {
                role: Role::User,
                content: goal.clone(),
            });
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
            total_usage: Usage::default(),
            final_reason: TerminationReason::Error,
            final_output: None,
        }
    }

    pub async fn record_event(&mut self, event: &StreamEvent, state_store: &StateStore) {
        match event {
            StreamEvent::RunStarted { .. } => {
                self.write_snapshot(state_store).await;
            }
            StreamEvent::LlmMessage { full, usage } => {
                self.steps += 1;
                self.history.push(Message {
                    role: Role::Assistant,
                    content: full.clone(),
                });
                self.total_usage.prompt_tokens += usage.prompt_tokens;
                self.total_usage.completion_tokens += usage.completion_tokens;
                self.total_usage.total_tokens += usage.total_tokens;
                self.write_snapshot(state_store).await;
            }
            StreamEvent::ToolCallStarted { .. } => {
                self.tool_calls += 1;
                self.write_snapshot(state_store).await;
            }
            StreamEvent::ToolCallCompleted { result, .. } => {
                self.history.push(Message {
                    role: Role::Tool,
                    content: result.output.clone(),
                });
                self.write_snapshot(state_store).await;
            }
            StreamEvent::ToolCallFailed { error, .. } => {
                self.tool_failures += 1;
                self.history.push(Message {
                    role: Role::Tool,
                    content: format!("Error: {error}"),
                });
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
                self.history.push(Message {
                    role: Role::User,
                    content: planned_step_failure_message(&step.title, reason),
                });
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
        self.write_report(workspace, model_id, run_dir);
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
            plan: self.plan.clone(),
        };
        if let Err(err) = state_store.write_task_state(&state).await {
            tracing::warn!("Failed to write task_state.json: {}", err);
        }
    }

    fn write_report(&self, workspace: &Workspace, model_id: &str, run_dir: &Path) {
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
        report.output = self.final_output.clone();

        if let Err(err) = write_report(run_dir, &report) {
            tracing::warn!("Failed to write report.json: {}", err);
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
