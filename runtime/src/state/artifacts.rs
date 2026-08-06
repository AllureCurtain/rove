use std::collections::HashMap;
use std::path::Path;

use crate::events::StreamEvent;
use crate::execution::{
    PlanIdentity, StepLedgerState, StepRecordStatus, planned_step_failure_message,
};
use crate::prompt_metadata::{PromptBuildMetadata, estimate_messages_tokens};
use crate::runtime_identity::RuntimeIdentity;
use crate::types::{
    JobId, PromptCheckpoint, PromptCompactionMode, PromptCompactionState, RunId, SessionId,
    TaskPlan, TaskState, TerminationReason,
};
use crate::workspace::Workspace;
use rove_core::{CallId, ToolExecutionMetadata, ToolMutation};
use rove_models::{Message, Role, Usage};

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
    tool_execution_metadata: Vec<ToolExecutionMetadata>,
    prompt_builds: Vec<PromptBuildMetadata>,
    runtime_identity: Option<RuntimeIdentity>,
    step_ledger: StepLedgerState,
    total_usage: Usage,
    final_reason: TerminationReason,
    final_output: Option<String>,
    pending_tool_use_ids: HashMap<CallId, Option<String>>,
    pending_steers: HashMap<String, String>,
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
        runtime_identity: Option<RuntimeIdentity>,
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
            tool_execution_metadata: Vec::new(),
            prompt_builds: Vec::new(),
            runtime_identity: runtime_identity
                .or_else(|| resume_state.and_then(|state| state.runtime_identity.clone())),
            step_ledger: resume_state
                .map(|state| state.step_ledger.clone())
                .unwrap_or_default(),
            total_usage: Usage::default(),
            final_reason: TerminationReason::Error,
            final_output: None,
            pending_tool_use_ids: HashMap::new(),
            pending_steers: HashMap::new(),
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
                self.total_usage.cached_tokens += usage.cached_tokens;
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PromptBuilt { metadata } => {
                self.prompt_builds.push(metadata.clone());
                self.write_snapshot(state_store).await;
            }
            // An accepted steer is not yet part of prompt history. Keep it
            // only until the engine confirms that its next model turn began;
            // cancellation or a budget boundary may still drop it.
            StreamEvent::SteerAccepted { id, content } => {
                self.pending_steers.insert(id.clone(), content.clone());
            }
            StreamEvent::SteerApplied { id } => {
                if let Some(content) = self.pending_steers.remove(id) {
                    self.history.push(Message::user(content));
                    self.write_snapshot(state_store).await;
                }
            }
            StreamEvent::SteerDropped { id, .. } => {
                self.pending_steers.remove(id);
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
                self.tool_execution_metadata.push(result.metadata.clone());
                self.write_snapshot(state_store).await;
            }
            StreamEvent::ToolCallFailed {
                call_id,
                error,
                metadata,
            } => {
                self.tool_failures += 1;
                let tool_use_id = self.pending_tool_use_ids.remove(call_id).flatten();
                self.history
                    .push(Message::tool(format!("Error: {error}"), tool_use_id));
                self.tool_execution_metadata.push(metadata.clone());
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PlanCreated {
                plan,
                identity,
                plan_revision,
            } => {
                self.plan = Some(plan.clone());
                self.step_ledger.set_plan_identity(identity);
                if let Some(revision) = plan_revision {
                    self.step_ledger
                        .plan_lifecycle
                        .push_revision(revision.as_ref().clone());
                }
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PlanDecision { record } => {
                self.step_ledger
                    .plan_lifecycle
                    .push_decision(record.as_ref().clone());
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PlanRevised { plan, revision } => {
                self.plan = Some(plan.clone());
                self.step_ledger.set_plan_identity(&revision.identity());
                self.step_ledger
                    .plan_lifecycle
                    .push_revision(revision.as_ref().clone());
                self.write_snapshot(state_store).await;
            }
            StreamEvent::PlanStepStarted { attempt, .. } => {
                if attempt.is_complete() {
                    self.step_ledger.active_step_attempt = Some(attempt.clone());
                    self.step_ledger.set_plan_identity(&PlanIdentity {
                        plan_id: attempt.plan_id.clone(),
                        plan_revision_id: attempt.plan_revision_id.clone(),
                        revision: self.step_ledger.active_plan_revision,
                    });
                }
                self.write_snapshot(state_store).await;
            }
            StreamEvent::StepResult { record } => {
                if !self
                    .step_ledger
                    .step_records
                    .iter()
                    .any(|saved| saved.record_id == record.record_id)
                {
                    self.step_ledger.step_records.push(record.as_ref().clone());
                }
                if self
                    .step_ledger
                    .active_step_attempt
                    .as_ref()
                    .is_some_and(|attempt| {
                        attempt.plan_id == record.plan_id
                            && attempt.plan_revision_id == record.plan_revision_id
                            && attempt.step_id == record.step_id
                            && attempt.attempt == record.attempt
                    })
                {
                    self.step_ledger.active_step_attempt = None;
                }
                if matches!(
                    record.status,
                    StepRecordStatus::Succeeded | StepRecordStatus::Skipped
                ) && let Some(active_plan) = self.plan.as_mut()
                    && let Some(saved_step) = active_plan
                        .steps
                        .iter_mut()
                        .find(|saved_step| saved_step.id == record.step_id)
                {
                    saved_step.done = true;
                    active_plan.current_step = active_plan
                        .steps
                        .iter()
                        .position(|saved_step| !saved_step.done)
                        .unwrap_or(active_plan.steps.len());
                }
                if matches!(
                    record.status,
                    StepRecordStatus::Failed
                        | StepRecordStatus::Blocked
                        | StepRecordStatus::Interrupted
                        | StepRecordStatus::BudgetExhausted
                ) {
                    let step_title = self
                        .plan
                        .as_ref()
                        .and_then(|plan| {
                            plan.steps
                                .iter()
                                .find(|step| step.id == record.step_id)
                                .map(|step| step.title.clone())
                        })
                        .unwrap_or_else(|| record.step_id.clone());
                    let reason = record
                        .safe_error_summary
                        .as_deref()
                        .unwrap_or(record.summary.as_str());
                    self.history
                        .push(Message::user(planned_step_failure_message(
                            &step_title,
                            reason,
                        )));
                }
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
            runtime_identity: self.runtime_identity.clone(),
            step_ledger: self.step_ledger.clone(),
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
        report.tool_execution_metadata = self.tool_execution_metadata.clone();
        report.prompt_builds = self.prompt_builds.clone();
        report.runtime_identity = self.runtime_identity.clone();
        report.step_records = self.step_ledger.step_records.clone();
        report.plan_decisions = self.step_ledger.plan_lifecycle.decisions.clone();
        report.plan_revisions = self.step_ledger.plan_lifecycle.revisions.clone();
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
            runtime_identity: self.runtime_identity.clone(),
            step_ledger: self.step_ledger.checkpoint(),
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

#[cfg(test)]
mod tests {
    use super::RunArtifactRecorder;
    use crate::events::StreamEvent;
    use crate::runtime_identity::RuntimeIdentity;
    use crate::state::store::StateStore;
    use crate::types::{ApprovalPolicy, JobId, RunId, SessionId};
    use crate::workspace::{Workspace, WorkspaceKind};
    use rove_core::{CallId, ToolExecutionMetadata, ToolExecutionStatus, ToolResult};
    use rove_models::Role;

    fn runtime_identity() -> RuntimeIdentity {
        RuntimeIdentity {
            cwd: "D:/workspace".to_string(),
            workspace_kind: WorkspaceKind::Repo,
            model_id: "gpt-4.1-mini".to_string(),
            provider_target: "openai-responses:https://api.openai.com/v1:gpt-4.1-mini".to_string(),
            approval_policy: ApprovalPolicy::Auto,
            max_steps: 12,
            plan_enabled: true,
            system_prompt_hash: "sha256:system".to_string(),
            planner_prompt_hash: "sha256:planner".to_string(),
            workspace_fingerprint: "sha256:workspace".to_string(),
            tool_signature: "sha256:tools".to_string(),
        }
    }

    #[tokio::test]
    async fn recorder_persists_runtime_identity_in_state_and_checkpoint() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_store = StateStore::new(tmp.path());
        let session_id = SessionId::new();
        let job_id = JobId::new();
        let run_id = RunId::new();
        let identity = runtime_identity();
        let mut recorder = RunArtifactRecorder::new(
            session_id,
            job_id,
            run_id,
            "inspect".to_string(),
            None,
            Some(identity.clone()),
        );

        recorder
            .record_event(
                &StreamEvent::RunStarted {
                    run_id,
                    job_id,
                    user_message: "inspect".to_string(),
                },
                &state_store,
            )
            .await;

        let state = state_store.load_task_state(run_id).await.unwrap();
        assert_eq!(state.runtime_identity.as_ref(), Some(&identity));
        assert_eq!(
            state
                .checkpoint
                .as_ref()
                .and_then(|checkpoint| checkpoint.runtime_identity.as_ref()),
            Some(&identity)
        );

        let workspace = Workspace {
            root: tmp.path().to_path_buf(),
            kind: WorkspaceKind::Folder,
            state_dir: tmp.path().join(".rove"),
        };
        let run_dir = tmp.path().join("report-run");
        recorder
            .finalize(&state_store, &workspace, "gpt-4.1-mini", &run_dir)
            .await;
        let report_json = tokio::fs::read_to_string(run_dir.join("report.json"))
            .await
            .unwrap();
        let report: crate::state::report::RunReport = serde_json::from_str(&report_json).unwrap();

        assert_eq!(report.runtime_identity.as_ref(), Some(&identity));
    }

    #[tokio::test]
    async fn recorder_persists_tool_execution_metadata_in_report() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_store = StateStore::new(tmp.path());
        let session_id = SessionId::new();
        let job_id = JobId::new();
        let run_id = RunId::new();
        let call_id = CallId::new();
        let metadata = ToolExecutionMetadata {
            status: ToolExecutionStatus::Ok,
            ..ToolExecutionMetadata::default()
        };
        let mut recorder = RunArtifactRecorder::new(
            session_id,
            job_id,
            run_id,
            "inspect".to_string(),
            None,
            None,
        );
        recorder
            .record_event(
                &StreamEvent::ToolCallCompleted {
                    call_id,
                    result: ToolResult {
                        call_id,
                        output: "done".to_string(),
                        mutations: Vec::new(),
                        metadata: metadata.clone(),
                    },
                },
                &state_store,
            )
            .await;

        let workspace = Workspace {
            root: tmp.path().to_path_buf(),
            kind: WorkspaceKind::Folder,
            state_dir: tmp.path().join(".rove"),
        };
        let run_dir = tmp.path().join("tool-report-run");
        recorder
            .finalize(&state_store, &workspace, "fake", &run_dir)
            .await;
        let report_json = tokio::fs::read_to_string(run_dir.join("report.json"))
            .await
            .unwrap();
        let report: crate::state::report::RunReport = serde_json::from_str(&report_json).unwrap();

        assert_eq!(report.tool_execution_metadata, vec![metadata]);
    }

    #[tokio::test]
    async fn recorder_persists_an_applied_steer_in_resumable_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_store = StateStore::new(tmp.path());
        let session_id = SessionId::new();
        let job_id = JobId::new();
        let run_id = RunId::new();
        let mut recorder = RunArtifactRecorder::new(
            session_id,
            job_id,
            run_id,
            "original goal".to_string(),
            None,
            None,
        );

        recorder
            .record_event(
                &StreamEvent::RunStarted {
                    run_id,
                    job_id,
                    user_message: "original goal".to_string(),
                },
                &state_store,
            )
            .await;
        recorder
            .record_event(
                &StreamEvent::SteerAccepted {
                    id: "steer-1".to_string(),
                    content: "use the safe migration path".to_string(),
                },
                &state_store,
            )
            .await;
        recorder
            .record_event(
                &StreamEvent::SteerApplied {
                    id: "steer-1".to_string(),
                },
                &state_store,
            )
            .await;

        let state = state_store.load_task_state(run_id).await.unwrap();
        assert!(state.history.iter().any(|message| {
            message.role == Role::User && message.content == "use the safe migration path"
        }));
    }

    #[tokio::test]
    async fn recorder_does_not_persist_a_dropped_steer_in_resumable_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let state_store = StateStore::new(tmp.path());
        let session_id = SessionId::new();
        let job_id = JobId::new();
        let run_id = RunId::new();
        let mut recorder = RunArtifactRecorder::new(
            session_id,
            job_id,
            run_id,
            "original goal".to_string(),
            None,
            None,
        );

        recorder
            .record_event(
                &StreamEvent::RunStarted {
                    run_id,
                    job_id,
                    user_message: "original goal".to_string(),
                },
                &state_store,
            )
            .await;
        recorder
            .record_event(
                &StreamEvent::SteerAccepted {
                    id: "steer-2".to_string(),
                    content: "this must not be replayed".to_string(),
                },
                &state_store,
            )
            .await;
        recorder
            .record_event(
                &StreamEvent::SteerDropped {
                    id: "steer-2".to_string(),
                    reason: "run cancelled".to_string(),
                },
                &state_store,
            )
            .await;

        let state = state_store.load_task_state(run_id).await.unwrap();
        assert!(
            state
                .history
                .iter()
                .all(|message| message.content != "this must not be replayed")
        );
    }
}
