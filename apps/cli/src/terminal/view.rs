use rove_core::ToolError;
use rove_runtime::events::StreamEvent;
use rove_runtime::execution::{
    PlanDecisionKind, PlanDecisionRecord, PlanRevision, StepRecord, StepRecordStatus,
};
use rove_runtime::prompt_metadata::PromptBuildMetadata;
use rove_runtime::types::{
    CallId, JobId, PlanStep, PromptCompactionState, RunId, TaskPlan, TerminationReason, ToolResult,
    Usage,
};

/// Maximum number of renderer-neutral entries retained for one run.
///
/// The terminal projection is intentionally an in-memory view, not another
/// persistence stream. Older entries are evicted from the front once this
/// bound is reached; canonical events and run artifacts remain unchanged.
pub const MAX_RUN_TIMELINE_ENTRIES: usize = 512;

/// Maximum length of free-form text copied into the visible timeline.
pub const MAX_RUN_TIMELINE_TEXT_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunTimelineEntry {
    /// Monotonic delivery sequence within this `RunViewState`.
    ///
    /// A sequence is consumed for every canonical update, including an
    /// idempotent update that does not add a second visible entry. This keeps
    /// ordering metadata honest while allowing the visible ledger to dedupe
    /// repeated lifecycle notifications.
    pub sequence: u64,
    pub run_id: Option<RunId>,
    pub job_id: Option<JobId>,
    pub kind: RunTimelineEntryKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunTimelineEntryKind {
    User {
        message: String,
    },
    Assistant {
        text: String,
        /// `false` is a streamed visible delta; `true` is the provider's
        /// completed normalized assistant message.
        final_message: bool,
    },
    /// The engine guarantees that `ModelStatus` is a safe progress note, not
    /// hidden model reasoning. Text is still bounded and conservatively
    /// redacted before it enters the projection.
    ModelStatus {
        status: String,
        message: String,
    },
    Plan {
        goal: String,
        step_count: usize,
    },
    PlanDecision {
        kind: PlanDecisionKind,
        summary: String,
    },
    PlanRevision {
        revision: u32,
        step_count: usize,
        superseded_step_count: usize,
    },
    PlanStep {
        index: usize,
        step_id: String,
        title: String,
        status: RunTimelinePlanStepStatus,
        reason: Option<String>,
    },
    Tool {
        call_id: CallId,
        name: String,
        status: RunTimelineToolStatus,
        /// Error codes are stable, non-secret summaries. Raw error text is
        /// deliberately not copied into the timeline.
        error_code: Option<String>,
    },
    Approval {
        call_id: CallId,
        tool_name: String,
        reason: String,
    },
    Input {
        input_id: CallId,
        prompt: String,
    },
    Compaction {
        mode: rove_runtime::types::PromptCompactionMode,
        source_message_count: usize,
        degraded: bool,
        summary_available: bool,
    },
    Memory {
        note_count: usize,
    },
    Completion {
        reason: TerminationReason,
        output: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunTimelinePlanStepStatus {
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunTimelineToolStatus {
    Started,
    Completed,
    Failed,
}

#[derive(Debug, Clone)]
pub enum RunViewUpdate {
    RunStarted {
        run_id: RunId,
        job_id: JobId,
        user_message: String,
    },
    AssistantDelta {
        delta: String,
    },
    ModelStatus {
        status: String,
        message: String,
    },
    LlmMessage {
        full: String,
        usage: Usage,
        tool_call_count: usize,
    },
    ToolCallStarted {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
    },
    ToolCallApprovalNeeded {
        call_id: CallId,
        name: String,
        args: serde_json::Value,
        reason: String,
    },
    ToolCallCompleted {
        call_id: CallId,
        result: ToolResult,
    },
    ToolCallFailed {
        call_id: CallId,
        error: ToolError,
    },
    InputNeeded {
        input_id: CallId,
        prompt: String,
    },
    PlanCreated {
        plan: TaskPlan,
        revision: Option<PlanRevision>,
    },
    PlanDecision {
        record: PlanDecisionRecord,
    },
    PlanRevised {
        plan: TaskPlan,
        revision: PlanRevision,
    },
    PlanStepStarted {
        step: PlanStep,
        index: usize,
    },
    StepResult {
        record: StepRecord,
    },
    PromptCompacted {
        summary: Option<String>,
        state: PromptCompactionState,
    },
    MemoryFlushed {
        note_count: usize,
    },
    PromptBuilt {
        metadata: PromptBuildMetadata,
    },
    RunCompleted {
        reason: TerminationReason,
        output: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct ToolCallView {
    pub call_id: CallId,
    pub name: String,
    pub args: serde_json::Value,
    pub status: ToolCallStatus,
    pub output: Option<String>,
    pub error: Option<ToolError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCallStatus {
    Started,
    WaitingApproval,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone)]
pub struct PendingApprovalView {
    pub call_id: CallId,
    pub name: String,
    pub args: serde_json::Value,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingInputView {
    pub input_id: CallId,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub struct RunCompletionView {
    pub reason: TerminationReason,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RunViewState {
    pub run_id: Option<RunId>,
    pub job_id: Option<JobId>,
    pub user_message: Option<String>,
    pub assistant_text: String,
    pub model_status: Option<(String, String)>,
    pub last_usage: Option<Usage>,
    pub plan: Option<TaskPlan>,
    pub plan_decisions: Vec<PlanDecisionRecord>,
    pub plan_revisions: Vec<PlanRevision>,
    pub current_step: Option<(usize, PlanStep)>,
    pub failed_steps: Vec<(usize, PlanStep, String)>,
    pub step_records: Vec<StepRecord>,
    pub prompt_compaction: Option<(Option<String>, PromptCompactionState)>,
    pub tool_calls: Vec<ToolCallView>,
    pub pending_approvals: Vec<PendingApprovalView>,
    pub pending_inputs: Vec<PendingInputView>,
    pub completed: Option<RunCompletionView>,
    /// Bounded, renderer-neutral visible history for this run.
    pub timeline: Vec<RunTimelineEntry>,
    /// Highest canonical update delivery sequence observed by this state.
    ///
    /// This includes duplicate, internal-only, and post-completion updates.
    /// A different `RunStarted` identity resets the per-run sequence to zero
    /// before recording that run's user entry.
    pub timeline_high_watermark: u64,
    /// Incremental, renderer-neutral safety state for streamed assistant text.
    /// Provider chunks can split tags and line markers at arbitrary boundaries.
    pub(crate) assistant_visibility: AssistantVisibilityProjection,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AssistantVisibilityProjection {
    block_pending: String,
    hidden_close_tag: Option<&'static str>,
    line_mode: AssistantLineMode,
    line_pending: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum AssistantLineMode {
    #[default]
    Detecting,
    Visible,
    Hidden,
}

const HIDDEN_REASONING_TAGS: [(&str, &str); 3] = [
    ("<think>", "</think>"),
    ("<analysis>", "</analysis>"),
    ("<reasoning>", "</reasoning>"),
];

const HIDDEN_REASONING_LINE_MARKERS: [&str; 4] =
    ["thought:", "reasoning:", "analysis:", "chain-of-thought:"];

impl AssistantVisibilityProjection {
    fn feed(&mut self, chunk: &str) -> String {
        self.block_pending.push_str(chunk);
        let visible = self.drain_block_pending(false);
        self.filter_line_markers(&visible)
    }

    fn finish(&mut self) -> String {
        let visible = self.drain_block_pending(true);
        let mut output = self.filter_line_markers(&visible);
        match self.line_mode {
            AssistantLineMode::Detecting => {
                output.push_str(&self.line_pending);
            }
            AssistantLineMode::Visible | AssistantLineMode::Hidden => {}
        }
        self.hidden_close_tag = None;
        self.block_pending.clear();
        self.line_pending.clear();
        self.line_mode = AssistantLineMode::Detecting;
        output
    }

    fn drain_block_pending(&mut self, finish: bool) -> String {
        let mut output = String::new();
        loop {
            if let Some(close) = self.hidden_close_tag {
                let lower = self.block_pending.to_ascii_lowercase();
                if let Some(index) = lower.find(close) {
                    let end = index + close.len();
                    self.block_pending.drain(..end);
                    self.hidden_close_tag = None;
                    continue;
                }
                if finish {
                    self.block_pending.clear();
                    break;
                }
                let keep = longest_suffix_prefix(&lower, [close]);
                let discard = self.block_pending.len().saturating_sub(keep);
                self.block_pending.drain(..discard);
                break;
            }

            let lower = self.block_pending.to_ascii_lowercase();
            let next_open = HIDDEN_REASONING_TAGS
                .iter()
                .filter_map(|(open, close)| lower.find(open).map(|index| (index, *close)))
                .min_by_key(|(index, _)| *index);
            if let Some((index, close)) = next_open {
                output.push_str(&self.block_pending[..index]);
                let open_len = HIDDEN_REASONING_TAGS
                    .iter()
                    .find(|(open, _)| lower[index..].starts_with(open))
                    .map(|(open, _)| open.len())
                    .unwrap_or_default();
                self.block_pending.drain(..index + open_len);
                self.hidden_close_tag = Some(close);
                continue;
            }

            if finish {
                output.push_str(&self.block_pending);
                self.block_pending.clear();
                break;
            }
            let keep =
                longest_suffix_prefix(&lower, HIDDEN_REASONING_TAGS.iter().map(|(open, _)| *open));
            let safe = self.block_pending.len().saturating_sub(keep);
            output.push_str(&self.block_pending[..safe]);
            self.block_pending.drain(..safe);
            break;
        }
        output
    }

    fn filter_line_markers(&mut self, input: &str) -> String {
        let mut output = String::new();
        for character in input.chars() {
            match self.line_mode {
                AssistantLineMode::Visible => {
                    output.push(character);
                    if character == '\n' {
                        self.line_mode = AssistantLineMode::Detecting;
                    }
                }
                AssistantLineMode::Hidden => {
                    if character == '\n' {
                        self.line_pending.clear();
                        self.line_mode = AssistantLineMode::Detecting;
                    }
                }
                AssistantLineMode::Detecting => {
                    if character == '\n' {
                        output.push_str(&self.line_pending);
                        output.push('\n');
                        self.line_pending.clear();
                        continue;
                    }
                    self.line_pending.push(character);
                    let candidate = self.line_pending.trim_start().to_ascii_lowercase();
                    if HIDDEN_REASONING_LINE_MARKERS
                        .iter()
                        .any(|marker| candidate == *marker)
                    {
                        self.line_pending.clear();
                        self.line_mode = AssistantLineMode::Hidden;
                    } else if !HIDDEN_REASONING_LINE_MARKERS
                        .iter()
                        .any(|marker| marker.starts_with(&candidate))
                    {
                        output.push_str(&self.line_pending);
                        self.line_pending.clear();
                        self.line_mode = AssistantLineMode::Visible;
                    }
                }
            }
        }
        output
    }
}

fn longest_suffix_prefix<'a>(value: &str, patterns: impl IntoIterator<Item = &'a str>) -> usize {
    patterns
        .into_iter()
        .map(|pattern| {
            (1..=pattern.len())
                .rev()
                .find(|length| value.ends_with(&pattern[..*length]))
                .unwrap_or(0)
        })
        .max()
        .unwrap_or(0)
}

impl RunViewState {
    pub fn apply_event(&mut self, event: &StreamEvent) -> RunViewUpdate {
        let update = RunViewUpdate::from(event);
        self.apply_update(update.clone());
        update
    }

    /// Returns the bounded visible ledger in canonical delivery order.
    ///
    /// The slice is renderer-facing state only. It is not persisted and does
    /// not replace `trace.jsonl`, task state, or reports as runtime facts.
    pub fn timeline_entries(&self) -> &[RunTimelineEntry] {
        &self.timeline
    }

    /// Iterates over the retained visible entries without exposing mutable
    /// projection state to a renderer.
    pub fn timeline_iter(&self) -> impl Iterator<Item = &RunTimelineEntry> {
        self.timeline.iter()
    }

    pub fn apply_update(&mut self, update: RunViewUpdate) {
        if let RunViewUpdate::RunStarted { run_id, job_id, .. } = &update {
            let new_identity = self.run_id != Some(*run_id);
            if new_identity {
                self.timeline.clear();
                self.timeline_high_watermark = 0;
                self.assistant_visibility = AssistantVisibilityProjection::default();
            }
            self.run_id = Some(*run_id);
            self.job_id = Some(*job_id);
        }
        self.record_timeline_update(&update);

        match update {
            RunViewUpdate::RunStarted {
                run_id,
                job_id,
                user_message,
            } => {
                self.run_id = Some(run_id);
                self.job_id = Some(job_id);
                self.user_message = Some(user_message);
            }
            RunViewUpdate::AssistantDelta { delta } => {
                self.assistant_text.push_str(&delta);
            }
            RunViewUpdate::ModelStatus { status, message } => {
                self.model_status = Some((status, message));
            }
            RunViewUpdate::LlmMessage { usage, .. } => {
                self.last_usage = Some(usage);
                self.model_status = None;
            }
            RunViewUpdate::ToolCallStarted {
                call_id,
                name,
                args,
            } => {
                self.model_status = None;
                self.tool_calls.push(ToolCallView {
                    call_id,
                    name,
                    args,
                    status: ToolCallStatus::Started,
                    output: None,
                    error: None,
                });
            }
            RunViewUpdate::ToolCallApprovalNeeded {
                call_id,
                name,
                args,
                reason,
            } => {
                if let Some(approval) = self
                    .pending_approvals
                    .iter_mut()
                    .find(|approval| approval.call_id == call_id)
                {
                    approval.name = name.clone();
                    approval.args = args.clone();
                    approval.reason = reason;
                } else {
                    self.pending_approvals.push(PendingApprovalView {
                        call_id,
                        name: name.clone(),
                        args: args.clone(),
                        reason,
                    });
                }
                upsert_tool_status(
                    &mut self.tool_calls,
                    call_id,
                    name,
                    args,
                    ToolCallStatus::WaitingApproval,
                );
            }
            RunViewUpdate::ToolCallCompleted { call_id, result } => {
                if let Some(tool) = self
                    .tool_calls
                    .iter_mut()
                    .rev()
                    .find(|tool| tool.call_id == call_id)
                {
                    tool.status = ToolCallStatus::Completed;
                    tool.output = Some(result.output);
                }
                self.pending_approvals
                    .retain(|approval| approval.call_id != call_id);
                self.pending_inputs
                    .retain(|input| input.input_id != call_id);
            }
            RunViewUpdate::ToolCallFailed { call_id, error } => {
                if let Some(tool) = self
                    .tool_calls
                    .iter_mut()
                    .rev()
                    .find(|tool| tool.call_id == call_id)
                {
                    tool.status = ToolCallStatus::Failed;
                    tool.error = Some(error);
                }
                self.pending_approvals
                    .retain(|approval| approval.call_id != call_id);
                self.pending_inputs
                    .retain(|input| input.input_id != call_id);
            }
            RunViewUpdate::InputNeeded { input_id, prompt } => {
                if let Some(input) = self
                    .pending_inputs
                    .iter_mut()
                    .find(|input| input.input_id == input_id)
                {
                    input.prompt = prompt;
                } else {
                    self.pending_inputs
                        .push(PendingInputView { input_id, prompt });
                }
            }
            RunViewUpdate::PlanCreated { plan, revision } => {
                self.plan = Some(plan);
                if let Some(revision) = revision
                    && self
                        .plan_revisions
                        .iter()
                        .all(|saved| saved.revision_id != revision.revision_id)
                {
                    self.plan_revisions.push(revision);
                }
            }
            RunViewUpdate::PlanDecision { record } => {
                if self.plan_decisions.iter().all(|saved| {
                    saved.decision.decision_id != record.decision.decision_id
                        && saved.trigger_step_record_id != record.trigger_step_record_id
                }) {
                    self.plan_decisions.push(record);
                }
            }
            RunViewUpdate::PlanRevised { plan, revision } => {
                self.plan = Some(plan);
                if self
                    .plan_revisions
                    .iter()
                    .all(|saved| saved.revision_id != revision.revision_id)
                {
                    self.plan_revisions.push(revision);
                }
            }
            RunViewUpdate::PlanStepStarted { step, index } => {
                if let Some(plan) = &mut self.plan {
                    plan.current_step = index.min(plan.steps.len());
                    if let Some(stored_step) = plan.steps.get_mut(index) {
                        *stored_step = step.clone();
                    }
                }
                self.model_status = None;
                self.current_step = Some((index, step));
            }
            RunViewUpdate::StepResult { record } => {
                if !self
                    .step_records
                    .iter()
                    .any(|saved| saved.record_id == record.record_id)
                {
                    self.step_records.push(record.clone());
                }
                if let Some(plan) = &mut self.plan
                    && let Some((index, stored_step)) = plan
                        .steps
                        .iter_mut()
                        .enumerate()
                        .find(|(_, stored_step)| stored_step.id == record.step_id)
                {
                    match record.status {
                        StepRecordStatus::Succeeded | StepRecordStatus::Skipped => {
                            stored_step.done = true;
                            plan.current_step = plan
                                .steps
                                .iter()
                                .position(|candidate| !candidate.done)
                                .unwrap_or(plan.steps.len());
                            self.current_step = None;
                        }
                        StepRecordStatus::Failed
                        | StepRecordStatus::Blocked
                        | StepRecordStatus::Interrupted => {
                            let reason = record
                                .safe_error_summary
                                .clone()
                                .unwrap_or_else(|| record.summary.clone());
                            self.failed_steps.push((index, stored_step.clone(), reason));
                            self.current_step = None;
                        }
                        _ => {}
                    }
                } else {
                    self.current_step = None;
                }
            }
            RunViewUpdate::PromptCompacted { summary, state } => {
                self.prompt_compaction = Some((summary, state));
            }
            RunViewUpdate::MemoryFlushed { .. } => {}
            RunViewUpdate::PromptBuilt { .. } => {}
            RunViewUpdate::RunCompleted { reason, output } => {
                self.model_status = None;
                self.current_step = None;
                self.pending_approvals.clear();
                self.pending_inputs.clear();
                for tool in &mut self.tool_calls {
                    if matches!(
                        tool.status,
                        ToolCallStatus::Started | ToolCallStatus::WaitingApproval
                    ) {
                        tool.status = ToolCallStatus::Interrupted;
                    }
                }
                self.completed = Some(RunCompletionView { reason, output });
            }
        }
    }

    fn record_timeline_update(&mut self, update: &RunViewUpdate) {
        self.timeline_high_watermark = self.timeline_high_watermark.saturating_add(1);
        let sequence = self.timeline_high_watermark;

        if self
            .timeline
            .iter()
            .any(|entry| matches!(entry.kind, RunTimelineEntryKind::Completion { .. }))
        {
            return;
        }

        let kind = match update {
            RunViewUpdate::RunStarted { user_message, .. } => Some(RunTimelineEntryKind::User {
                message: bounded_visible_text(user_message),
            }),
            RunViewUpdate::AssistantDelta { delta } => {
                let visible = self.assistant_visibility.feed(delta);
                (!visible.is_empty()).then(|| RunTimelineEntryKind::Assistant {
                    text: bounded_visible_text(&visible),
                    final_message: false,
                })
            }
            RunViewUpdate::ModelStatus { status, message } => {
                Some(RunTimelineEntryKind::ModelStatus {
                    status: bounded_visible_text(status),
                    message: bounded_visible_text(message),
                })
            }
            RunViewUpdate::LlmMessage { full, .. } => {
                let _ = self.assistant_visibility.finish();
                let visible = visible_assistant_text(full);
                self.assistant_visibility = AssistantVisibilityProjection::default();
                visible.map(|text| RunTimelineEntryKind::Assistant {
                    text,
                    final_message: true,
                })
            }
            RunViewUpdate::ToolCallStarted { call_id, name, .. } => {
                Some(RunTimelineEntryKind::Tool {
                    call_id: *call_id,
                    name: bounded_visible_text(name),
                    status: RunTimelineToolStatus::Started,
                    error_code: None,
                })
            }
            RunViewUpdate::ToolCallApprovalNeeded {
                call_id,
                name,
                reason,
                ..
            } => Some(RunTimelineEntryKind::Approval {
                call_id: *call_id,
                tool_name: bounded_visible_text(name),
                reason: bounded_visible_text(reason),
            }),
            RunViewUpdate::ToolCallCompleted { call_id, .. } => Some(RunTimelineEntryKind::Tool {
                call_id: *call_id,
                name: self.timeline_tool_name(*call_id),
                status: RunTimelineToolStatus::Completed,
                error_code: None,
            }),
            RunViewUpdate::ToolCallFailed { call_id, error } => Some(RunTimelineEntryKind::Tool {
                call_id: *call_id,
                name: self.timeline_tool_name(*call_id),
                status: RunTimelineToolStatus::Failed,
                error_code: Some(error.error_code().to_string()),
            }),
            RunViewUpdate::InputNeeded { input_id, prompt } => Some(RunTimelineEntryKind::Input {
                input_id: *input_id,
                prompt: bounded_visible_text(prompt),
            }),
            RunViewUpdate::PlanCreated { plan, .. } => Some(RunTimelineEntryKind::Plan {
                goal: bounded_visible_text(&plan.goal),
                step_count: plan.steps.len(),
            }),
            RunViewUpdate::PlanDecision { record } => Some(RunTimelineEntryKind::PlanDecision {
                kind: record.decision.kind,
                summary: bounded_visible_text(&record.decision.safe_summary),
            }),
            RunViewUpdate::PlanRevised { revision, .. } => {
                Some(RunTimelineEntryKind::PlanRevision {
                    revision: revision.revision,
                    step_count: revision.remaining_steps.len(),
                    superseded_step_count: revision.superseded_remaining_step_ids.len(),
                })
            }
            RunViewUpdate::PlanStepStarted { step, index } => Some(timeline_plan_step(
                *index,
                step,
                RunTimelinePlanStepStatus::Started,
                None,
            )),
            RunViewUpdate::StepResult { record } => {
                if self
                    .step_records
                    .iter()
                    .any(|saved| saved.record_id == record.record_id)
                {
                    None
                } else {
                    let status = match record.status {
                        StepRecordStatus::Succeeded | StepRecordStatus::Skipped => {
                            Some(RunTimelinePlanStepStatus::Completed)
                        }
                        StepRecordStatus::Failed
                        | StepRecordStatus::Blocked
                        | StepRecordStatus::Interrupted => Some(RunTimelinePlanStepStatus::Failed),
                        _ => None,
                    };
                    status.map(|status| {
                        let reason = match status {
                            RunTimelinePlanStepStatus::Failed => Some(bounded_visible_text(
                                record
                                    .safe_error_summary
                                    .as_deref()
                                    .unwrap_or(record.summary.as_str()),
                            )),
                            _ => None,
                        };
                        let title = self
                            .plan
                            .as_ref()
                            .and_then(|plan| {
                                plan.steps
                                    .iter()
                                    .find(|candidate| candidate.id == record.step_id)
                                    .map(|step| step.title.clone())
                            })
                            .filter(|title| !title.trim().is_empty())
                            .unwrap_or_else(|| {
                                if record.summary.trim().is_empty() {
                                    record.step_id.clone()
                                } else {
                                    record.summary.clone()
                                }
                            });
                        let step = PlanStep {
                            id: record.step_id.clone(),
                            title,
                            done: matches!(
                                record.status,
                                StepRecordStatus::Succeeded | StepRecordStatus::Skipped
                            ),
                        };
                        let index = self
                            .plan
                            .as_ref()
                            .and_then(|plan| {
                                plan.steps
                                    .iter()
                                    .position(|candidate| candidate.id == record.step_id)
                            })
                            .unwrap_or(0);
                        timeline_plan_step(index, &step, status, reason)
                    })
                }
            }
            // Canonical step_result owns the visible plan-step timeline entry.
            RunViewUpdate::PromptCompacted { summary, state } => {
                Some(RunTimelineEntryKind::Compaction {
                    mode: state.mode.clone(),
                    source_message_count: state.source_message_count,
                    degraded: state.degraded,
                    summary_available: summary.is_some(),
                })
            }
            RunViewUpdate::MemoryFlushed { note_count } => Some(RunTimelineEntryKind::Memory {
                note_count: *note_count,
            }),
            // Prompt construction metadata is internal runtime telemetry, not
            // visible transcript content.
            RunViewUpdate::PromptBuilt { .. } => None,
            RunViewUpdate::RunCompleted { reason, output } => {
                let _ = self.assistant_visibility.finish();
                Some(RunTimelineEntryKind::Completion {
                    reason: reason.clone(),
                    output: output.as_deref().and_then(visible_assistant_text),
                })
            }
        };

        if let Some(kind) = kind {
            self.push_timeline_entry(sequence, kind);
        }
    }

    fn timeline_tool_name(&self, call_id: CallId) -> String {
        self.tool_calls
            .iter()
            .rev()
            .find(|tool| tool.call_id == call_id)
            .map(|tool| bounded_visible_text(&tool.name))
            .unwrap_or_else(|| "unknown tool".to_string())
    }

    fn push_timeline_entry(&mut self, sequence: u64, kind: RunTimelineEntryKind) {
        if timeline_kind_is_idempotent(&kind)
            && self.timeline.iter().any(|entry| {
                entry.run_id == self.run_id && entry.job_id == self.job_id && entry.kind == kind
            })
        {
            return;
        }

        self.timeline.push(RunTimelineEntry {
            sequence,
            run_id: self.run_id,
            job_id: self.job_id,
            kind,
        });
        let excess = self.timeline.len().saturating_sub(MAX_RUN_TIMELINE_ENTRIES);
        if excess > 0 {
            self.timeline.drain(..excess);
        }
    }
}

fn timeline_plan_step(
    index: usize,
    step: &PlanStep,
    status: RunTimelinePlanStepStatus,
    reason: Option<String>,
) -> RunTimelineEntryKind {
    RunTimelineEntryKind::PlanStep {
        index,
        step_id: bounded_visible_text(&step.id),
        title: bounded_visible_text(&step.title),
        status,
        reason,
    }
}

fn timeline_kind_is_idempotent(kind: &RunTimelineEntryKind) -> bool {
    // Status notes, memory flushes, and streamed deltas have no stable event
    // identity and therefore remain append-only even when their payloads
    // repeat. The remaining variants carry a run/call/step/terminal identity
    // and can safely coalesce replayed notifications.
    matches!(
        kind,
        RunTimelineEntryKind::User { .. }
            | RunTimelineEntryKind::Assistant {
                final_message: true,
                ..
            }
            | RunTimelineEntryKind::Plan { .. }
            | RunTimelineEntryKind::PlanDecision { .. }
            | RunTimelineEntryKind::PlanRevision { .. }
            | RunTimelineEntryKind::PlanStep { .. }
            | RunTimelineEntryKind::Tool { .. }
            | RunTimelineEntryKind::Approval { .. }
            | RunTimelineEntryKind::Input { .. }
            | RunTimelineEntryKind::Compaction { .. }
            | RunTimelineEntryKind::Completion { .. }
    )
}

fn visible_assistant_text(value: &str) -> Option<String> {
    let visible = strip_hidden_reasoning_blocks(value);
    let visible = visible.trim();
    (!visible.is_empty()).then(|| bounded_visible_text(visible))
}

fn strip_hidden_reasoning_blocks(value: &str) -> String {
    let mut projection = AssistantVisibilityProjection::default();
    let mut visible = projection.feed(value);
    visible.push_str(&projection.finish());
    visible
}

fn bounded_visible_text(value: &str) -> String {
    if contains_secret_signal(value) {
        return "[redacted]".to_string();
    }

    let mut chars = value.chars();
    let mut visible = String::new();
    for character in chars.by_ref().take(MAX_RUN_TIMELINE_TEXT_CHARS) {
        if character.is_control() && character != '\n' && character != '\t' {
            visible.push(' ');
        } else {
            visible.push(character);
        }
    }
    if chars.next().is_some() {
        let keep = MAX_RUN_TIMELINE_TEXT_CHARS.saturating_sub(3);
        visible = visible.chars().take(keep).collect();
        visible.push_str("...");
    }
    visible
}

fn contains_secret_signal(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "api_key=",
        "api_key:",
        "api-key=",
        "api-key:",
        "apikey=",
        "access_token=",
        "refresh_token=",
        "token=",
        "password=",
        "password:",
        "passwd=",
        "secret=",
        "secret:",
        "api_key\"",
        "api-key\"",
        "apikey\"",
        "access_token\"",
        "refresh_token\"",
        "token\"",
        "password\"",
        "passwd\"",
        "secret\"",
        "authorization:",
        "bearer ",
        "-----begin private key-----",
        "-----begin rsa private key-----",
    ]
    .iter()
    .any(|signal| lower.contains(signal))
        || lower
            .split(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '\'' | ',' | ';')
            })
            .any(|token| {
                (token.starts_with("sk-") && token.len() > 8)
                    || (token.starts_with("ghp_") && token.len() > 8)
                    || (token.starts_with("xoxb-") && token.len() > 8)
            })
}

fn upsert_tool_status(
    tools: &mut Vec<ToolCallView>,
    call_id: CallId,
    name: String,
    args: serde_json::Value,
    status: ToolCallStatus,
) {
    if let Some(tool) = tools.iter_mut().rev().find(|tool| tool.call_id == call_id) {
        tool.status = status;
    } else {
        tools.push(ToolCallView {
            call_id,
            name,
            args,
            status,
            output: None,
            error: None,
        });
    }
}

impl From<&StreamEvent> for RunViewUpdate {
    fn from(event: &StreamEvent) -> Self {
        match event {
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message,
            } => Self::RunStarted {
                run_id: *run_id,
                job_id: *job_id,
                user_message: user_message.clone(),
            },
            StreamEvent::LlmChunk { delta } => Self::AssistantDelta {
                delta: delta.clone(),
            },
            StreamEvent::ModelStatus { status, message } => Self::ModelStatus {
                status: status.clone(),
                message: message.clone(),
            },
            StreamEvent::LlmMessage {
                full,
                usage,
                tool_calls,
            } => Self::LlmMessage {
                full: full.clone(),
                usage: usage.clone(),
                tool_call_count: tool_calls.len(),
            },
            StreamEvent::ToolCallStarted {
                call_id,
                name,
                args,
                ..
            } => Self::ToolCallStarted {
                call_id: *call_id,
                name: name.clone(),
                args: args.clone(),
            },
            StreamEvent::ToolCallApprovalNeeded {
                call_id,
                name,
                args,
                reason,
            } => Self::ToolCallApprovalNeeded {
                call_id: *call_id,
                name: name.clone(),
                args: args.clone(),
                reason: reason.clone(),
            },
            StreamEvent::ToolCallCompleted { call_id, result } => Self::ToolCallCompleted {
                call_id: *call_id,
                result: result.clone(),
            },
            StreamEvent::ToolCallFailed { call_id, error, .. } => Self::ToolCallFailed {
                call_id: *call_id,
                error: error.clone(),
            },
            StreamEvent::InputNeeded { input_id, prompt } => Self::InputNeeded {
                input_id: *input_id,
                prompt: prompt.clone(),
            },
            StreamEvent::PlanCreated {
                plan,
                plan_revision,
                ..
            } => Self::PlanCreated {
                plan: plan.clone(),
                revision: plan_revision.as_deref().cloned(),
            },
            StreamEvent::PlanDecision { record } => Self::PlanDecision {
                record: record.as_ref().clone(),
            },
            StreamEvent::PlanRevised { plan, revision } => Self::PlanRevised {
                plan: plan.clone(),
                revision: revision.as_ref().clone(),
            },
            StreamEvent::PlanStepStarted { step, index, .. } => Self::PlanStepStarted {
                step: step.clone(),
                index: *index,
            },
            StreamEvent::StepResult { record } => Self::StepResult {
                record: record.as_ref().clone(),
            },
            StreamEvent::PromptCompacted { summary, state } => Self::PromptCompacted {
                summary: summary.clone(),
                state: state.clone(),
            },
            StreamEvent::MemoryFlushed { notes } => Self::MemoryFlushed {
                note_count: notes.len(),
            },
            StreamEvent::PromptBuilt { metadata } => Self::PromptBuilt {
                metadata: metadata.clone(),
            },
            StreamEvent::RunCompleted { reason, output } => Self::RunCompleted {
                reason: reason.clone(),
                output: output.clone(),
            },
            StreamEvent::SteerAccepted { .. } => Self::ModelStatus {
                status: "steer".to_string(),
                message: "Steer accepted for the next model turn.".to_string(),
            },
            StreamEvent::SteerApplied { .. } => Self::ModelStatus {
                status: "steer".to_string(),
                message: "Steer applied to a model turn.".to_string(),
            },
            StreamEvent::SteerDropped { reason, .. } => Self::ModelStatus {
                status: "steer".to_string(),
                message: format!("Steer dropped: {reason}"),
            },
            StreamEvent::FollowupQueued { .. } => Self::ModelStatus {
                status: "follow-up".to_string(),
                message: "Follow-up queued for the next completed turn.".to_string(),
            },
            StreamEvent::FollowupDequeued { .. } => Self::ModelStatus {
                status: "follow-up".to_string(),
                message: "Follow-up dequeued to start its next turn.".to_string(),
            },
            StreamEvent::FollowupAbandoned { reason, .. } => Self::ModelStatus {
                status: "follow-up".to_string(),
                message: format!("Follow-up needs confirmation: {reason}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::terminal::view::{
        MAX_RUN_TIMELINE_ENTRIES, MAX_RUN_TIMELINE_TEXT_CHARS, RunTimelineEntryKind,
        RunTimelinePlanStepStatus, RunTimelineToolStatus, RunViewUpdate,
    };
    use rove_core::ToolError;
    use rove_runtime::events::StreamEvent;
    use rove_runtime::execution::{StepCompletionBasis, StepRecord, StepRecordStatus};
    use rove_runtime::prompt_metadata::PromptBuildMetadata;
    use rove_runtime::types::{
        CallId, JobId, PlanStep, PromptCompactionMode, PromptCompactionState, RunId, TaskPlan,
        TerminationReason, ToolResult, Usage,
    };

    fn usage() -> Usage {
        Usage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: 0,
        }
    }

    fn step() -> PlanStep {
        PlanStep {
            id: "step-1".to_string(),
            title: "Read files".to_string(),
            done: false,
        }
    }

    fn step_record() -> StepRecord {
        StepRecord {
            record_id: "record-1".to_string(),
            plan_id: "plan-1".to_string(),
            plan_revision_id: "revision-1".to_string(),
            step_id: "step-1".to_string(),
            attempt: 1,
            status: StepRecordStatus::Succeeded,
            started_at: "2026-07-20T00:00:00Z".to_string(),
            finished_at: "2026-07-20T00:00:01Z".to_string(),
            summary: "done".to_string(),
            completion_basis: StepCompletionBasis::ModelConclusion,
            evidence_refs: Vec::new(),
            tool_call_ids: Vec::new(),
            artifact_refs: Vec::new(),
            mutations: Vec::new(),
            model_turns_used: 1,
            tool_calls_used: 0,
            token_usage: Usage::default(),
            error_code: None,
            safe_error_summary: None,
            supersedes_record_id: None,
        }
    }

    #[test]
    fn maps_all_stream_events_to_terminal_updates() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let call_id = CallId::new();
        let input_id = CallId::new();
        let plan = TaskPlan {
            goal: "goal".to_string(),
            steps: vec![step()],
            current_step: 0,
        };
        let compaction = PromptCompactionState {
            mode: PromptCompactionMode::Deterministic,
            auto_triggered: false,
            degraded: false,
            consecutive_failures: 0,
            circuit_open: false,
            model: None,
            prompt_version: None,
            source_message_count: 4,
            last_error: None,
        };
        let events = vec![
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "hello".to_string(),
            },
            StreamEvent::LlmChunk {
                delta: "hi".to_string(),
            },
            StreamEvent::ModelStatus {
                status: "thinking".to_string(),
                message: "planning".to_string(),
            },
            StreamEvent::LlmMessage {
                full: "full".to_string(),
                usage: usage(),
                tool_calls: Vec::new(),
            },
            StreamEvent::ToolCallStarted {
                call_id,
                tool_use_id: None,
                name: "read_file".to_string(),
                args: serde_json::json!({"path":"README.md"}),
            },
            StreamEvent::ToolCallApprovalNeeded {
                call_id,
                name: "write_file".to_string(),
                args: serde_json::json!({"path":"out.txt"}),
                reason: "writes a file".to_string(),
            },
            StreamEvent::ToolCallCompleted {
                call_id,
                result: ToolResult {
                    call_id,
                    output: "done".to_string(),
                    mutations: Vec::new(),
                    metadata: Default::default(),
                },
            },
            StreamEvent::ToolCallFailed {
                call_id,
                error: ToolError::ExecutionFailed {
                    reason: "boom".to_string(),
                },
                metadata: Default::default(),
            },
            StreamEvent::InputNeeded {
                input_id,
                prompt: "Which branch?".to_string(),
            },
            StreamEvent::PlanCreated {
                plan: plan.clone(),
                identity: Default::default(),
                plan_revision: None,
            },
            StreamEvent::PlanStepStarted {
                step: step(),
                index: 0,
                attempt: Default::default(),
            },
            StreamEvent::StepResult {
                record: Box::new(step_record()),
            },
            StreamEvent::PromptCompacted {
                summary: Some("summary".to_string()),
                state: compaction,
            },
            StreamEvent::MemoryFlushed {
                notes: vec!["tool result: created file src/memory/session.rs".to_string()],
            },
            StreamEvent::PromptBuilt {
                metadata: PromptBuildMetadata::default(),
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("ok".to_string()),
            },
            StreamEvent::SteerAccepted {
                id: "steer-1".to_string(),
                content: "focus on tests".to_string(),
            },
            StreamEvent::SteerApplied {
                id: "steer-1".to_string(),
            },
            StreamEvent::SteerDropped {
                id: "steer-2".to_string(),
                reason: "run completed".to_string(),
            },
            StreamEvent::FollowupQueued {
                id: "follow-up-1".to_string(),
                content: "continue".to_string(),
            },
            StreamEvent::FollowupDequeued {
                id: "follow-up-1".to_string(),
            },
            StreamEvent::FollowupAbandoned {
                id: "follow-up-2".to_string(),
                reason: "run cancelled".to_string(),
            },
        ];

        let updates: Vec<RunViewUpdate> = events.iter().map(RunViewUpdate::from).collect();

        assert_eq!(updates.len(), 22);
        assert!(matches!(
            updates[0],
            RunViewUpdate::RunStarted {
                user_message: ref message,
                ..
            } if message == "hello"
        ));
        assert!(matches!(
            updates[4],
            RunViewUpdate::ToolCallStarted { ref name, .. } if name == "read_file"
        ));
        assert!(matches!(
            updates[8],
            RunViewUpdate::InputNeeded { ref prompt, .. } if prompt == "Which branch?"
        ));
        assert!(matches!(updates[11], RunViewUpdate::StepResult { .. }));
        assert!(matches!(
            updates[13],
            RunViewUpdate::MemoryFlushed { note_count: 1 }
        ));
        assert!(matches!(updates[14], RunViewUpdate::PromptBuilt { .. }));
        assert!(matches!(
            updates[15],
            RunViewUpdate::RunCompleted {
                reason: TerminationReason::Final,
                ..
            }
        ));
        assert!(matches!(
            updates[16],
            RunViewUpdate::ModelStatus { ref status, .. } if status == "steer"
        ));
        assert!(matches!(
            updates[19],
            RunViewUpdate::ModelStatus { ref status, .. } if status == "follow-up"
        ));
    }

    #[test]
    fn run_view_state_tracks_plan_tools_pending_items_and_completion() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let call_id = CallId::new();
        let input_id = CallId::new();
        let mut state = super::RunViewState::default();

        state.apply_update(RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "hello".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "hi".to_string(),
        });
        state.apply_update(RunViewUpdate::PlanCreated {
            plan: TaskPlan {
                goal: "goal".to_string(),
                steps: vec![step()],
                current_step: 0,
            },
            revision: None,
        });
        state.apply_update(RunViewUpdate::ToolCallStarted {
            call_id,
            name: "read_file".to_string(),
            args: serde_json::json!({"path":"README.md"}),
        });
        state.apply_update(RunViewUpdate::ToolCallApprovalNeeded {
            call_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        });
        state.apply_update(RunViewUpdate::InputNeeded {
            input_id,
            prompt: "Which branch?".to_string(),
        });
        state.apply_update(RunViewUpdate::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("ok".to_string()),
        });

        assert_eq!(state.run_id, Some(run_id));
        assert_eq!(state.job_id, Some(job_id));
        assert_eq!(state.user_message.as_deref(), Some("hello"));
        assert_eq!(state.assistant_text, "hi");
        assert_eq!(state.plan.as_ref().unwrap().steps.len(), 1);
        assert_eq!(state.tool_calls.len(), 1);
        assert!(state.pending_approvals.is_empty());
        assert!(state.pending_inputs.is_empty());
        assert_eq!(
            state.completed.as_ref().unwrap().reason,
            TerminationReason::Final
        );
    }

    #[test]
    fn completed_plan_steps_update_the_stored_plan_and_cursor() {
        let mut state = super::RunViewState::default();
        let first = step();
        let second = PlanStep {
            id: "step-2".to_string(),
            title: "Write tests".to_string(),
            done: false,
        };
        state.apply_update(RunViewUpdate::PlanCreated {
            plan: TaskPlan {
                goal: "finish the TUI".to_string(),
                steps: vec![first.clone(), second],
                current_step: 0,
            },
            revision: None,
        });
        state.apply_update(RunViewUpdate::PlanStepStarted {
            step: first.clone(),
            index: 0,
        });
        state.apply_update(RunViewUpdate::StepResult {
            record: StepRecord {
                record_id: "record-1".to_string(),
                plan_id: "plan-1".to_string(),
                plan_revision_id: "revision-1".to_string(),
                step_id: first.id,
                attempt: 1,
                status: StepRecordStatus::Succeeded,
                started_at: "2026-07-20T00:00:00Z".to_string(),
                finished_at: "2026-07-20T00:00:01Z".to_string(),
                summary: first.title,
                completion_basis: StepCompletionBasis::ModelConclusion,
                evidence_refs: Vec::new(),
                tool_call_ids: Vec::new(),
                artifact_refs: Vec::new(),
                mutations: Vec::new(),
                model_turns_used: 1,
                tool_calls_used: 0,
                token_usage: Usage::default(),
                error_code: None,
                safe_error_summary: None,
                supersedes_record_id: None,
            },
        });

        let plan = state.plan.as_ref().unwrap();
        assert!(plan.steps[0].done);
        assert_eq!(plan.current_step, 1);
        assert!(state.current_step.is_none());
    }

    #[test]
    fn step_result_is_structured_state_and_timeline_entry_is_deduped_by_record_id() {
        let mut state = super::RunViewState::default();
        let record = step_record();

        state.apply_update(RunViewUpdate::StepResult {
            record: record.clone(),
        });
        state.apply_update(RunViewUpdate::StepResult { record });

        assert_eq!(state.step_records.len(), 1);
        assert_eq!(state.timeline.len(), 1);
        assert_eq!(state.timeline_high_watermark, 2);
    }

    #[test]
    fn pending_interactions_are_idempotent_and_input_clears_with_its_tool() {
        let call_id = CallId::new();
        let mut state = super::RunViewState::default();
        let approval = RunViewUpdate::ToolCallApprovalNeeded {
            call_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        };
        let input = RunViewUpdate::InputNeeded {
            input_id: call_id,
            prompt: "Which branch?".to_string(),
        };

        state.apply_update(approval.clone());
        state.apply_update(approval);
        state.apply_update(input.clone());
        state.apply_update(input);

        assert_eq!(state.pending_approvals.len(), 1);
        assert_eq!(state.pending_inputs.len(), 1);
        assert_eq!(
            state
                .timeline
                .iter()
                .filter(|entry| matches!(entry.kind, RunTimelineEntryKind::Approval { .. }))
                .count(),
            1
        );
        assert_eq!(
            state
                .timeline
                .iter()
                .filter(|entry| matches!(entry.kind, RunTimelineEntryKind::Input { .. }))
                .count(),
            1
        );

        state.apply_update(RunViewUpdate::ToolCallFailed {
            call_id,
            error: ToolError::ExecutionFailed {
                reason: "cancelled".to_string(),
            },
        });

        assert!(state.pending_approvals.is_empty());
        assert!(state.pending_inputs.is_empty());
    }

    #[test]
    fn llm_message_clears_transient_status_without_repeating_streamed_text() {
        let mut state = super::RunViewState::default();
        state.apply_update(RunViewUpdate::ModelStatus {
            status: "thinking".to_string(),
            message: "working".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "answer".to_string(),
        });
        state.apply_update(RunViewUpdate::LlmMessage {
            full: "answer".to_string(),
            usage: usage(),
            tool_call_count: 0,
        });

        assert_eq!(state.assistant_text, "answer");
        assert!(state.model_status.is_none());
    }

    #[test]
    fn streamed_reasoning_is_filtered_across_chunk_boundaries() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let mut state = super::RunViewState::default();
        state.apply_update(RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "show the answer".to_string(),
        });
        for delta in ["<thi", "nk>hidden", "</thi", "nk>visible"] {
            state.apply_update(RunViewUpdate::AssistantDelta {
                delta: delta.to_string(),
            });
        }

        let visible = state
            .timeline
            .iter()
            .filter_map(|entry| match &entry.kind {
                RunTimelineEntryKind::Assistant { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(visible, "visible");

        let mut line_state = super::RunViewState::default();
        line_state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "prefix\nReas".to_string(),
        });
        line_state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "oning: hidden\nvisible".to_string(),
        });
        let line_visible = line_state
            .timeline
            .iter()
            .filter_map(|entry| match &entry.kind {
                RunTimelineEntryKind::Assistant { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(line_visible, "prefix\nvisible");
    }

    #[test]
    fn reasoning_projection_fails_closed_for_unclosed_blocks_and_preserves_state_on_duplicate_start()
     {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let mut state = super::RunViewState::default();
        state.apply_update(RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "prompt".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "<think>hidden".to_string(),
        });
        state.apply_update(RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "prompt".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "</think>visible".to_string(),
        });
        assert!(state
            .timeline
            .iter()
            .any(|entry| matches!(&entry.kind, RunTimelineEntryKind::Assistant { text, .. } if text == "visible")));

        let mut unclosed = super::RunViewState::default();
        unclosed.apply_update(RunViewUpdate::AssistantDelta {
            delta: "<think>DO_NOT_RENDER".to_string(),
        });
        unclosed.apply_update(RunViewUpdate::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("<think>DO_NOT_RENDER".to_string()),
        });
        let projected = format!("{:?}", unclosed.timeline);
        assert!(!projected.contains("DO_NOT_RENDER"));
    }

    #[test]
    fn visible_timeline_follows_canonical_update_order_with_typed_entries() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let call_id = CallId::new();
        let input_id = CallId::new();
        let mut state = super::RunViewState::default();
        let plan = TaskPlan {
            goal: "ship timeline".to_string(),
            steps: vec![step()],
            current_step: 0,
        };

        state.apply_update(RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "inspect the run".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "working".to_string(),
        });
        state.apply_update(RunViewUpdate::ModelStatus {
            status: "planning".to_string(),
            message: "Building a safe plan".to_string(),
        });
        state.apply_update(RunViewUpdate::LlmMessage {
            full: "working".to_string(),
            usage: usage(),
            tool_call_count: 1,
        });
        state.apply_update(RunViewUpdate::PlanCreated {
            plan,
            revision: None,
        });
        state.apply_update(RunViewUpdate::PlanStepStarted {
            step: step(),
            index: 0,
        });
        state.apply_update(RunViewUpdate::ToolCallStarted {
            call_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
        });
        state.apply_update(RunViewUpdate::ToolCallApprovalNeeded {
            call_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        });
        state.apply_update(RunViewUpdate::InputNeeded {
            input_id,
            prompt: "Which branch?".to_string(),
        });
        state.apply_update(RunViewUpdate::ToolCallCompleted {
            call_id,
            result: ToolResult {
                call_id,
                output: "done".to_string(),
                mutations: Vec::new(),
                metadata: Default::default(),
            },
        });
        state.apply_update(RunViewUpdate::PromptCompacted {
            summary: Some("summary".to_string()),
            state: PromptCompactionState {
                source_message_count: 7,
                ..PromptCompactionState::default()
            },
        });
        state.apply_update(RunViewUpdate::MemoryFlushed { note_count: 2 });
        state.apply_update(RunViewUpdate::PromptBuilt {
            metadata: PromptBuildMetadata::default(),
        });
        state.apply_update(RunViewUpdate::RunCompleted {
            reason: TerminationReason::Final,
            output: Some("done".to_string()),
        });

        assert_eq!(state.timeline.len(), 13);
        assert_eq!(state.timeline_entries().len(), 13);
        assert_eq!(state.timeline_iter().count(), 13);
        assert_eq!(state.timeline_high_watermark, 14);
        assert_eq!(
            state
                .timeline
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            (1..=12).chain([14]).collect::<Vec<_>>()
        );
        assert!(
            state
                .timeline
                .iter()
                .all(|entry| entry.run_id == Some(run_id) && entry.job_id == Some(job_id))
        );
        assert!(matches!(
            state.timeline[0].kind,
            RunTimelineEntryKind::User { ref message } if message == "inspect the run"
        ));
        assert!(matches!(
            state.timeline[1].kind,
            RunTimelineEntryKind::Assistant {
                final_message: false,
                ..
            }
        ));
        assert!(matches!(
            state.timeline[2].kind,
            RunTimelineEntryKind::ModelStatus { .. }
        ));
        assert!(matches!(
            state.timeline[3].kind,
            RunTimelineEntryKind::Assistant {
                final_message: true,
                ..
            }
        ));
        assert!(matches!(
            state.timeline[4].kind,
            RunTimelineEntryKind::Plan { step_count: 1, .. }
        ));
        assert!(matches!(
            state.timeline[5].kind,
            RunTimelineEntryKind::PlanStep {
                status: RunTimelinePlanStepStatus::Started,
                ..
            }
        ));
        assert!(matches!(
            state.timeline[6].kind,
            RunTimelineEntryKind::Tool {
                call_id: id,
                status: RunTimelineToolStatus::Started,
                ..
            } if id == call_id
        ));
        assert!(matches!(
            state.timeline[7].kind,
            RunTimelineEntryKind::Approval { call_id: id, .. } if id == call_id
        ));
        assert!(matches!(
            state.timeline[8].kind,
            RunTimelineEntryKind::Input { input_id: id, .. } if id == input_id
        ));
        assert!(matches!(
            state.timeline[9].kind,
            RunTimelineEntryKind::Tool {
                call_id: id,
                status: RunTimelineToolStatus::Completed,
                ..
            } if id == call_id
        ));
        assert!(matches!(
            state.timeline[10].kind,
            RunTimelineEntryKind::Compaction {
                source_message_count: 7,
                summary_available: true,
                ..
            }
        ));
        assert!(matches!(
            state.timeline[11].kind,
            RunTimelineEntryKind::Memory { note_count: 2 }
        ));
        assert!(matches!(
            state.timeline[12].kind,
            RunTimelineEntryKind::Completion {
                reason: TerminationReason::Final,
                ..
            }
        ));
    }

    #[test]
    fn timeline_dedupes_idempotent_updates_without_hiding_streamed_deltas() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let mut state = super::RunViewState::default();
        let started = RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "hello".to_string(),
        };
        let status = RunViewUpdate::ModelStatus {
            status: "working".to_string(),
            message: "Still working".to_string(),
        };
        let final_message = RunViewUpdate::LlmMessage {
            full: "same answer".to_string(),
            usage: usage(),
            tool_call_count: 0,
        };
        let memory = RunViewUpdate::MemoryFlushed { note_count: 1 };

        for update in [started.clone(), started, status.clone(), status] {
            state.apply_update(update);
        }
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "same".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "same".to_string(),
        });
        for update in [final_message.clone(), final_message, memory.clone(), memory] {
            state.apply_update(update);
        }

        assert_eq!(state.timeline_high_watermark, 10);
        assert_eq!(state.timeline.len(), 8);
        assert_eq!(
            state
                .timeline
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            vec![1, 3, 4, 5, 6, 7, 9, 10]
        );
        assert_eq!(
            state
                .timeline
                .iter()
                .filter(|entry| matches!(entry.kind, RunTimelineEntryKind::Assistant { .. }))
                .count(),
            3
        );
        assert_eq!(
            state
                .timeline
                .iter()
                .filter(|entry| matches!(entry.kind, RunTimelineEntryKind::ModelStatus { .. }))
                .count(),
            2
        );
        assert_eq!(
            state
                .timeline
                .iter()
                .filter(|entry| matches!(entry.kind, RunTimelineEntryKind::Memory { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn timeline_bounds_entries_and_text_without_splitting_unicode() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let mut state = super::RunViewState::default();
        state.apply_update(RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "hello".to_string(),
        });

        for index in 0..(MAX_RUN_TIMELINE_ENTRIES + 20) {
            state.apply_update(RunViewUpdate::AssistantDelta {
                delta: format!("chunk-{index}"),
            });
        }

        assert_eq!(state.timeline.len(), MAX_RUN_TIMELINE_ENTRIES);
        assert_eq!(
            state.timeline.last().unwrap().sequence,
            state.timeline_high_watermark
        );
        assert_eq!(state.timeline.first().unwrap().sequence, 22);

        let mut text_state = super::RunViewState::default();
        text_state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "界".repeat(MAX_RUN_TIMELINE_TEXT_CHARS + 20),
        });
        let RunTimelineEntryKind::Assistant { text, .. } =
            &text_state.timeline.first().unwrap().kind
        else {
            panic!("expected assistant entry");
        };
        assert_eq!(text.chars().count(), MAX_RUN_TIMELINE_TEXT_CHARS);
        assert!(text.ends_with("..."));
    }

    #[test]
    fn a_new_run_gets_a_new_ledger_while_cloned_history_stays_stable() {
        let first_run = RunId::new();
        let first_job = JobId::new();
        let second_run = RunId::new();
        let second_job = JobId::new();
        let mut state = super::RunViewState::default();
        state.apply_update(RunViewUpdate::RunStarted {
            run_id: first_run,
            job_id: first_job,
            user_message: "first".to_string(),
        });
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "first answer".to_string(),
        });
        let archived = state.clone();

        state.apply_update(RunViewUpdate::RunStarted {
            run_id: second_run,
            job_id: second_job,
            user_message: "second".to_string(),
        });

        assert_eq!(archived.timeline.len(), 2);
        assert!(
            archived
                .timeline
                .iter()
                .all(|entry| entry.run_id == Some(first_run))
        );
        assert_eq!(state.timeline_high_watermark, 1);
        assert_eq!(state.timeline.len(), 1);
        assert_eq!(state.timeline[0].sequence, 1);
        assert_eq!(state.timeline[0].run_id, Some(second_run));
        assert_eq!(state.timeline[0].job_id, Some(second_job));
    }

    #[test]
    fn cancellation_closes_the_visible_ledger_and_preserves_completion_order() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let call_id = CallId::new();
        let mut state = super::RunViewState::default();
        state.apply_update(RunViewUpdate::RunStarted {
            run_id,
            job_id,
            user_message: "cancel me".to_string(),
        });
        state.apply_update(RunViewUpdate::ToolCallApprovalNeeded {
            call_id,
            name: "write_file".to_string(),
            args: serde_json::json!({"path":"out.txt"}),
            reason: "writes a file".to_string(),
        });
        state.apply_update(RunViewUpdate::InputNeeded {
            input_id: call_id,
            prompt: "Continue?".to_string(),
        });
        state.apply_update(RunViewUpdate::RunCompleted {
            reason: TerminationReason::Cancelled,
            output: None,
        });
        let visible_len = state.timeline.len();
        state.apply_update(RunViewUpdate::AssistantDelta {
            delta: "late output".to_string(),
        });

        assert!(state.pending_approvals.is_empty());
        assert!(state.pending_inputs.is_empty());
        assert_eq!(state.timeline.len(), visible_len);
        assert_eq!(state.timeline_high_watermark, 5);
        assert!(matches!(
            state.timeline.last().unwrap().kind,
            RunTimelineEntryKind::Completion {
                reason: TerminationReason::Cancelled,
                ..
            }
        ));
    }

    #[test]
    fn timeline_omits_raw_reasoning_tool_payloads_memory_notes_and_secrets() {
        let run_id = RunId::new();
        let job_id = JobId::new();
        let call_id = CallId::new();
        let mut state = super::RunViewState::default();
        let compaction = PromptCompactionState {
            model: Some("CANARY_COMPACTION_MODEL".to_string()),
            prompt_version: Some("CANARY_PROMPT_VERSION".to_string()),
            last_error: Some("CANARY_COMPACTION_ERROR".to_string()),
            ..PromptCompactionState::default()
        };

        let events = [
            StreamEvent::RunStarted {
                run_id,
                job_id,
                user_message: "token=CANARY_USER_SECRET".to_string(),
            },
            StreamEvent::LlmChunk {
                delta: "<think>CANARY_HIDDEN_REASONING</think>visible answer".to_string(),
            },
            StreamEvent::ToolCallStarted {
                call_id,
                tool_use_id: None,
                name: "read_file".to_string(),
                args: serde_json::json!({"api_key":"CANARY_TOOL_ARG"}),
            },
            StreamEvent::ToolCallApprovalNeeded {
                call_id,
                name: "read_file".to_string(),
                args: serde_json::json!({"password":"CANARY_APPROVAL_ARG"}),
                reason: "secret:CANARY_APPROVAL_REASON".to_string(),
            },
            StreamEvent::ToolCallFailed {
                call_id,
                error: ToolError::ExecutionFailed {
                    reason: "CANARY_TOOL_ERROR".to_string(),
                },
                metadata: Default::default(),
            },
            StreamEvent::PromptCompacted {
                summary: Some("CANARY_COMPACTION_SUMMARY".to_string()),
                state: compaction,
            },
            StreamEvent::MemoryFlushed {
                notes: vec!["CANARY_MEMORY_NOTE".to_string()],
            },
            StreamEvent::RunCompleted {
                reason: TerminationReason::Final,
                output: Some("password=CANARY_COMPLETION_SECRET".to_string()),
            },
        ];
        for event in &events {
            state.apply_event(event);
        }

        let projected = format!("{:?}", state.timeline);
        for canary in [
            "CANARY_USER_SECRET",
            "CANARY_HIDDEN_REASONING",
            "CANARY_TOOL_ARG",
            "CANARY_APPROVAL_ARG",
            "CANARY_APPROVAL_REASON",
            "CANARY_TOOL_ERROR",
            "CANARY_COMPACTION_MODEL",
            "CANARY_PROMPT_VERSION",
            "CANARY_COMPACTION_ERROR",
            "CANARY_COMPACTION_SUMMARY",
            "CANARY_MEMORY_NOTE",
            "CANARY_COMPLETION_SECRET",
        ] {
            assert!(!projected.contains(canary), "timeline leaked {canary}");
        }
        assert!(projected.contains("visible answer"));
        assert!(projected.contains("execution_failed"));
        assert!(projected.contains("[redacted]"));
    }
}
