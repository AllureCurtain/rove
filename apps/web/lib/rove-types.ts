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
  | "rejected"
  | "skipped"
  | "budget_exhausted"
  | "cancelled"
  | "interrupted"
  | "indeterminate";

export type StepCompletionBasis =
  | "model_conclusion"
  | "deterministic_rule"
  | "user_decision"
  | "runtime_failure";

/** Semantic uncertainty that deterministic lifecycle rules cannot resolve. */
export type PlanAmbiguityKind =
  | "remaining_work_may_be_unnecessary"
  | "plan_assumption_may_be_invalid"
  | "recoverable_alternative_may_exist"
  | "goal_may_be_partially_satisfied"
  | "remaining_dependencies_may_need_reordering";

export interface PlanAmbiguity {
  kind: PlanAmbiguityKind;
  safe_summary: string;
  evidence_refs?: string[];
}

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
  ambiguity?: PlanAmbiguity;
}

export interface ExecutionBudgetUsage {
  plan_steps: number;
  step_attempts: number;
  model_turns: number;
  tool_calls: number;
  plan_revisions: number;
  model_repairs: number;
  planner_turns: number;
  evaluator_turns: number;
  replanner_turns: number;
  finalization_turns: number;
  wall_time_ms: number;
  total_tokens: number;
  cost_microunits: number;
}

/** Independent per-dimension execution limits. Absent means unresolved. */
export interface ExecutionBudgetLimits {
  max_plan_steps?: number | null;
  max_step_attempts?: number | null;
  max_model_turns?: number | null;
  max_model_turns_per_step?: number | null;
  max_tool_calls?: number | null;
  max_tool_calls_per_step?: number | null;
  max_plan_revisions?: number | null;
  max_model_repairs?: number | null;
  max_finalization_turns?: number | null;
  max_wall_time_ms?: number | null;
  max_total_tokens?: number | null;
  max_cost_microunits?: number | null;
}

export type ExecutionPhase =
  | "planner"
  | "step"
  | "evaluator"
  | "replanner"
  | "finalizer"
  | "run";

export type ExecutionBudgetDimension =
  | "plan_steps"
  | "step_attempts"
  | "model_turns"
  | "model_turns_per_step"
  | "tool_calls"
  | "tool_calls_per_step"
  | "plan_revisions"
  | "model_repairs"
  | "finalization_turns"
  | "wall_time"
  | "total_tokens"
  | "cost";

export interface ExecutionBudgetExhaustion {
  dimension: ExecutionBudgetDimension;
  phase: ExecutionPhase;
  limit: number;
  consumed: number;
  safe_summary: string;
}

export interface ExecutionBudgetSnapshot {
  limits: ExecutionBudgetLimits;
  consumed: ExecutionBudgetUsage;
  exhausted?: ExecutionBudgetExhaustion | null;
  /** Cost is enforceable only when the provider supplies priced usage. */
  cost_enforced: boolean;
}

export type ExecutionStrategy = "react" | "plan_react";

export type StrategySelectionSource =
  | "request"
  | "session"
  | "config"
  | "compatibility_default"
  | "max_steps_and_plan_flag";

export type EvaluatorMode = "rule_only" | "rule_first_model_on_ambiguity";

export type FinalizerPolicy = "deterministic" | "model_preferred";

export interface ExecutionPolicy {
  version: number;
  strategy: ExecutionStrategy;
  selection_source: StrategySelectionSource;
  budgets: ExecutionBudgetLimits;
  evaluator_mode?: EvaluatorMode;
  finalizer_policy?: FinalizerPolicy;
}

/** Explicit, safe degradation fact. Fallbacks are never silent. */
export interface ExecutionDegradation {
  degradation_id: string;
  phase: ExecutionPhase;
  code: string;
  safe_summary: string;
  occurred_at: string;
}

/** User-visible terminal classification, distinct from `TerminationReason`. */
export type FinalOutcomeStatus =
  | "success"
  | "partial"
  | "blocked"
  | "rejected"
  | "cancelled"
  | "interrupted"
  | "exhausted"
  | "indeterminate"
  | "failed";

export type FinalizationMode =
  | "direct"
  | "model"
  | "deterministic"
  | "deterministic_fallback";

export type FinalizationPhase = "started" | "completed";

export interface FinalizationRecord {
  finalization_id: string;
  phase: FinalizationPhase;
  finish_reason: PlanFinishReason;
  outcome?: FinalOutcomeStatus | null;
  mode: FinalizationMode;
  started_at: string;
  completed_at?: string | null;
  output?: string | null;
  evidence_refs?: string[];
  incomplete_step_ids?: string[];
  budget_before?: ExecutionBudgetUsage;
  budget_after?: ExecutionBudgetUsage;
}

/** Materialized run lifecycle projection stored in state and reports. */
export interface ExecutionLifecycleState {
  policy?: ExecutionPolicy | null;
  budget_usage?: ExecutionBudgetUsage;
  budget_exhaustion?: ExecutionBudgetExhaustion | null;
  finalization?: FinalizationRecord | null;
  degradations?: ExecutionDegradation[];
}

export type PlanDecisionKind = "continue" | "replace_remaining" | "finish";

export type PlanFinishReason =
  | "completed"
  | "partial"
  | "blocked"
  | "budget_exhausted"
  | "failed"
  | "cancelled"
  | "interrupted"
  | "rejected"
  | "indeterminate";

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
  metadata?: ToolExecutionMetadata;
}

export interface ToolMutation {
  path: string;
  operation: ToolMutationOperation;
  diff?: string | null;
}

export type ToolMutationOperation = "create" | "update" | "delete" | "unknown";

export type ToolExecutionStatus =
  | "ok"
  | "error"
  | "rejected"
  | "partial_success";

export type ToolRiskLevel = "low" | "high";

export interface ToolExecutionMetadata {
  status: ToolExecutionStatus;
  error_code?: string;
  security_event_type?: string;
  risk_level: ToolRiskLevel;
  read_only: boolean;
  /** Omitted on the wire when the canonical list is empty. */
  affected_paths?: string[];
  workspace_changed: boolean;
  /** Omitted on the wire when the canonical list is empty. */
  diff_summary?: string[];
}

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
      type: "execution_strategy_selected";
      policy: ExecutionPolicy;
    }
  | {
      type: "execution_budget_updated";
      phase: ExecutionPhase;
      snapshot: ExecutionBudgetSnapshot;
    }
  | {
      type: "execution_degraded";
      record: ExecutionDegradation;
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
      metadata?: ToolExecutionMetadata;
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
      budget?: ExecutionBudgetSnapshot;
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
      type: "finalization_started";
      record: FinalizationRecord;
    }
  | {
      type: "finalization_completed";
      record: FinalizationRecord;
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
    }
  | {
      type: "steer_accepted";
      id: string;
      content: string;
    }
  | {
      type: "steer_applied";
      id: string;
    }
  | {
      type: "steer_dropped";
      id: string;
      reason: string;
    }
  | {
      type: "followup_queued";
      id: string;
      content: string;
    }
  | {
      type: "followup_dequeued";
      id: string;
    }
  | {
      type: "followup_abandoned";
      id: string;
      reason: string;
    };

export type CreateJobWorkspaceKind = "folder" | "repo" | "task";

/**
 * Per-job workspace binding.
 *
 * - `folder` / `repo`: bind tools/state to an absolute local `root`.
 * - `task`: isolated workspace under `base`/`name`.
 */
export interface CreateJobWorkspace {
  kind: CreateJobWorkspaceKind;
  /** Task workspace name (`kind = "task"` only). */
  name?: string;
  /** Task base directory (`kind = "task"` only). */
  base?: string;
  /** Absolute local directory for `folder` / `repo`. */
  root?: string;
}

export interface CreateJobRequest {
  message: string;
  model?: string;
  max_steps?: number;
  approval?: ApprovalPolicy;
  resume?: ResumeMode;
  workspace?: CreateJobWorkspace;
  provider?: ProviderProfile;
  /** Server-owned product session; the API resolves its exact runtime run. */
  product_session_id?: string;
}

/**
 * User-facing provider type (protocol family). Official and relay endpoints
 * share the same type; only base URL / key / model differ. Gemini relays that
 * expose an OpenAI Chat Completions API use the `openai` type.
 */
export type ProviderType =
  | "openai"
  | "openai-responses"
  | "anthropic"
  | "ollama"
  | "fake";

export interface ProviderProfile {
  /**
   * Provider type: openai | openai-responses | anthropic | ollama | fake.
   * Maps to an internal wire protocol on the API. Not "official vs relay".
   */
  provider_type?: ProviderType;
  /**
   * Optional display label. When omitted/empty the API derives a name from
   * `api_base`. Use `provider_type` to select the type, not `name`.
   */
  name?: string;
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
  provider_type?: string | null;
  wire_protocol?: string | null;
  api_base: string;
  key_env: string;
  key_present: boolean;
  model?: string | null;
  model_present?: boolean | null;
  models_count: number;
}

/** Request body for listing models available on a provider endpoint. */
export interface ProviderModelsRequest {
  provider: ProviderProfile;
  /** Optional override for the models inventory URL. */
  models_endpoint?: string;
}

/** Catalog of model ids returned by a provider inventory endpoint. */
export interface ProviderModelsResponse {
  provider: string;
  provider_type: string;
  wire_protocol: string;
  api_base: string;
  key_env: string;
  key_present: boolean;
  models: string[];
  models_count: number;
}

export interface CreateJobResponse {
  job_id: string;
  run_id: string;
  resumed_from_run_id?: string | null;
  workspace_activation?: "restricted" | "trusted";
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
  "execution_strategy_selected",
  "execution_budget_updated",
  "execution_degraded",
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
  "step_result",
  "plan_decision",
  "plan_revised",
  "finalization_started",
  "finalization_completed",
  "prompt_compacted",
  "memory_flushed",
  "prompt_built",
  "run_completed",
  "steer_accepted",
  "steer_applied",
  "steer_dropped",
  "followup_queued",
  "followup_dequeued",
  "followup_abandoned",
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
