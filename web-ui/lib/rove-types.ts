export type RunStatus = "init" | "running" | "done" | "error" | "cancelled" | "interrupted";

export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  cached_tokens?: number;
}

export interface PlanStep {
  id: string;
  title: string;
  done: boolean;
}

export interface TaskPlan {
  goal: string;
  steps: PlanStep[];
  current_step: number;
}

export type StepRecordStatus =
  | "succeeded"
  | "partial"
  | "failed"
  | "blocked"
  | "skipped"
  | "budget_exhausted"
  | "cancelled"
  | "interrupted";

export type StepCompletionBasis =
  | "model_conclusion"
  | "deterministic_rule"
  | "user_decision"
  | "runtime_failure";

export interface StepRecord {
  record_id: string;
  plan_id: string;
  plan_revision_id: string;
  step_id: string;
  attempt: number;
  status: StepRecordStatus;
  started_at: string;
  finished_at: string;
  summary: string;
  completion_basis: StepCompletionBasis;
  evidence_refs?: string[];
  tool_call_ids?: string[];
  artifact_refs?: string[];
  mutations?: ToolMutation[];
  model_turns_used: number;
  tool_calls_used: number;
  token_usage: Usage;
  error_code?: string;
  safe_error_summary?: string;
  supersedes_record_id?: string;
}

export interface ExecutionBudgetUsage {
  plan_steps: number;
  step_attempts: number;
  model_turns: number;
  tool_calls: number;
  plan_revisions: number;
  wall_time_ms: number;
  total_tokens: number;
  cost_microunits: number;
}

export type PlanDecisionKind = "continue" | "replace_remaining" | "finish";

export type PlanFinishReason =
  | "completed"
  | "partial"
  | "blocked"
  | "budget_exhausted"
  | "failed"
  | "cancelled"
  | "interrupted";

export interface PlanDecision {
  decision_id: string;
  kind: PlanDecisionKind;
  safe_reason_codes?: string[];
  safe_summary: string;
  remaining_work_requirements?: string[];
  finish_reason?: PlanFinishReason;
}

export interface PlanDecisionRecord {
  trigger_step_record_id: string;
  decided_at: string;
  decision: PlanDecision;
}

export interface PlanRevision {
  plan_id: string;
  revision_id: string;
  parent_revision_id?: string;
  revision: number;
  created_at: string;
  trigger_step_record_id?: string;
  decision_id: string;
  safe_reason_codes?: string[];
  retained_step_ids?: string[];
  superseded_remaining_step_ids?: string[];
  remaining_steps?: PlanStep[];
  capability_snapshot_id?: string;
  budget_snapshot: ExecutionBudgetUsage;
}

export type PromptCompactionMode =
  | "none"
  | "deterministic"
  | "model_generated"
  | "automatic"
  | "degraded"
  | "disabled";

export interface PromptCompactionState {
  mode: PromptCompactionMode;
  auto_triggered: boolean;
  degraded: boolean;
  consecutive_failures: number;
  circuit_open: boolean;
  model?: string;
  prompt_version?: string;
  source_message_count: number;
  last_error?: string;
}

export interface PromptBuildMetadata {
  prompt_hash: string;
  stable_prefix_hash: string;
  workspace_fingerprint: string;
  tool_signature: string;
  token_estimate: number;
  included_history_messages: number;
  dropped_history_messages: number;
  prompt_cache_key: string;
}

export interface ToolResult {
  call_id: string;
  output: string;
  mutations?: ToolMutation[];
}

export interface ToolMutation {
  path: string;
  operation: ToolMutationOperation;
  diff?: string | null;
}

export type ToolMutationOperation = "create" | "update" | "delete" | "unknown";

export interface ToolCallRef {
  id: string;
  name: string;
  args: unknown;
}

export interface ToolError {
  code: string;
  reason?: string;
  timeout_ms?: number;
  name?: string;
  [key: string]: unknown;
}

export type StreamEvent =
  | {
      type: "run_started";
      run_id: string;
      job_id: string;
      user_message: string;
    }
  | {
      type: "llm_chunk";
      delta: string;
    }
  | {
      type: "model_status";
      status: string;
      message: string;
    }
  | {
      type: "llm_message";
      full: string;
      usage: Usage;
      tool_calls?: ToolCallRef[];
    }
  | {
      type: "tool_call_started";
      call_id: string;
      tool_use_id?: string | null;
      name: string;
      args: unknown;
    }
  | {
      type: "tool_call_approval_needed";
      call_id: string;
      name: string;
      args: unknown;
      reason: string;
    }
  | {
      type: "tool_call_completed";
      call_id: string;
      result: ToolResult;
    }
  | {
      type: "tool_call_failed";
      call_id: string;
      error: ToolError;
    }
  | {
      type: "input_needed";
      input_id: string;
      prompt: string;
    }
  | {
      type: "plan_created";
      plan: TaskPlan;
      plan_id?: string;
      plan_revision_id?: string;
      revision?: number;
      plan_revision?: PlanRevision;
    }
  | {
      type: "plan_step_started";
      step: PlanStep;
      index: number;
      plan_id?: string;
      plan_revision_id?: string;
      step_id?: string;
      attempt?: number;
      started_at?: string;
    }
  | {
      type: "plan_step_completed";
      step: PlanStep;
      index: number;
    }
  | {
      type: "plan_step_failed";
      step: PlanStep;
      index: number;
      reason: string;
    }
  | {
      type: "step_result";
      record: StepRecord;
    }
  | {
      type: "plan_decision";
      record: PlanDecisionRecord;
    }
  | {
      type: "plan_revised";
      plan: TaskPlan;
      revision: PlanRevision;
    }
  | {
      type: "prompt_compacted";
      summary?: string | null;
      state: PromptCompactionState;
    }
  | {
      type: "memory_flushed";
      notes: string[];
    }
  | {
      type: "prompt_built";
      metadata: PromptBuildMetadata;
    }
  | {
      type: "run_completed";
      reason: string;
      output?: string | null;
    };

export interface CreateJobRequest {
  message: string;
  model?: string;
  max_steps?: number;
  approval?: ApprovalPolicy;
  resume?: ResumeMode;
  provider?: ProviderProfile;
}

export interface ProviderProfile {
  name: "openai-compatible" | "openai-responses" | "openai" | "anthropic" | "ollama" | "fake";
  api_base: string;
  api_key_env?: string;
}

export interface ProviderTestRequest {
  provider: ProviderProfile;
  model?: string;
  models_endpoint?: string;
}

export interface ProviderTestResponse {
  status: string;
  provider: string;
  api_base: string;
  key_env: string;
  key_present: boolean;
  model?: string | null;
  model_present?: boolean | null;
  models_count: number;
}

export interface CreateJobResponse {
  job_id: string;
  run_id: string;
  resumed_from_run_id?: string | null;
}

export interface JobStateResponse {
  job_id: string;
  run_id: string;
  resumed_from_run_id?: string | null;
  status: RunStatus;
  event_count: number;
  events: JobStreamEvent[];
  pending_approvals: PendingApproval[];
  pending_inputs: PendingInput[];
}

export interface JobStreamEvent {
  seq: number;
  event: StreamEvent;
}

export interface ListRunsResponse {
  runs: RunSummary[];
}

export interface RunSummary {
  run_id: string;
  session_id: string;
  job_id: string;
  status: RunStatus;
  last_event_seq: number;
  has_report: boolean;
}

export interface RunReport {
  session_id: string;
  job_id: string;
  run_id: string;
  workspace_root: string;
  workspace_kind: string;
  model_id: string;
  status: string;
  termination_reason: string;
  steps: number;
  total_usage: Usage;
  tool_calls: number;
  tool_failures: number;
  tool_mutations: ToolMutation[];
  step_records?: StepRecord[];
  output?: string | null;
  timestamp: string;
}

export interface PendingApproval {
  call_id: string;
  name: string;
  args: unknown;
  reason: string;
}

export interface PendingInput {
  input_id: string;
  prompt: string;
}

export type ApprovalDecision = "approve" | "reject";

export type ApprovalPolicy = "ask" | "auto" | "never";

export type ResumeMode = "latest";

export const STREAM_EVENT_NAMES = [
  "run_started",
  "llm_chunk",
  "model_status",
  "llm_message",
  "tool_call_started",
  "tool_call_approval_needed",
  "tool_call_completed",
  "tool_call_failed",
  "input_needed",
  "plan_created",
  "plan_step_started",
  "plan_step_completed",
  "plan_step_failed",
  "step_result",
  "plan_decision",
  "plan_revised",
  "prompt_compacted",
  "memory_flushed",
  "prompt_built",
  "run_completed",
] as const;

export type StreamEventName = (typeof STREAM_EVENT_NAMES)[number];

export interface BenchSuiteInfo {
  name: string;
  description: string;
  profiles: string[];
}

export interface ListBenchSuitesResponse {
  suites: BenchSuiteInfo[];
}

export interface StartBenchRunRequest {
  suite: string;
  profile: string;
}

export interface StartBenchRunResponse {
  bench_run_id: string;
  suite: string;
  profile: string;
  status: string;
}

export interface BenchRunSummary {
  bench_run_id: string;
  suite: string;
  profile: string;
  status: string;
  total_tasks: number;
  passed_tasks: number;
  failed_tasks: number;
  started_at: string | null;
  finished_at: string | null;
  evidence_root: string | null;
}

export interface ListBenchRunsResponse {
  runs: BenchRunSummary[];
}

export interface BenchCheckResult {
  kind: string;
  description: string;
  passed: boolean;
  detail: string;
}

export interface BenchArtifacts {
  run_dir: string;
  trace_jsonl: string;
  task_state_json: string;
  report_json: string;
}

export interface BenchTaskResult {
  name: string;
  outcome: string;
  termination_reason: string;
  steps: number;
  tool_calls: number;
  tool_failures: number;
  artifacts: BenchArtifacts;
  output: string | null;
  check_results: BenchCheckResult[];
  failures: string[];
}

export interface BenchRunDetail {
  bench_run_id: string;
  suite: string;
  profile: string;
  status: string;
  started_at: string | null;
  finished_at: string | null;
  total_tasks: number;
  passed_tasks: number;
  failed_tasks: number;
  evidence_root: string | null;
  summary_md: string | null;
  tasks: BenchTaskResult[];
}
