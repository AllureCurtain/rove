import {
  ARTIFACT_VALIDATION_STATES,
  MAX_PRUNED_PAYLOAD_DIGESTS,
  MAX_PRUNING_DIGEST_BYTES,
  MAX_PRUNING_POLICY_BYTES,
  PRUNING_WIRE_COUNT_KEYS,
  readPromptPruningFacts,
  STREAM_EVENT_NAMES,
  TOOL_ARTIFACT_KINDS,
  TOOL_RESULT_OUTCOMES,
  type AgentDiagnostic,
  type AgentProfileIdentity,
  type ExecutionBudgetExhaustion,
  type ExecutionBudgetLimits,
  type ExecutionBudgetSnapshot,
  type ExecutionBudgetUsage,
  type ExecutionDegradation,
  type ExecutionPolicy,
  type FinalizationRecord,
  type PlanAmbiguity,
  type PlanDecision,
  type PlanDecisionRecord,
  type PlanRevision,
  type PlanStep,
  type PromptCompactionState,
  type PromptPruningFacts,
  type ProcedureReference,
  type ProcedureApplication,
  type ProcedureCapabilityBinding,
  type ProcedureDeviation,
  type RunStatus,
  type StepRecord,
  type StreamEvent,
  type TaskPlan,
  type ToolArtifactRef,
  type ToolArtifactSource,
  type ToolCallRef,
  type ToolContentBlock,
  type ToolContentBlockMeta,
  type ToolError,
  type ToolMutation,
  type ToolMutationOperation,
  type ToolOutputEnvelope,
  type ToolProtocolMetadata,
  type Usage,
} from "../lib/rove-types";

export const M1_BROWSER_SOURCE_SCHEMA_VERSION = 1 as const;
export const MAX_PRODUCT_WORKSPACES = 256;
export const MAX_PRODUCT_SESSIONS = 2_048;
export const MAX_PRODUCT_PROVIDER_PROFILES = 128;
export const MAX_PRODUCT_TEXT_BYTES = 512;
export const MAX_PRODUCT_API_BASE_BYTES = 2_048;
export const MAX_PRODUCT_PATH_BYTES = 32_768;
export const MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES = 128;
export const MAX_PRODUCT_CONTROL_CONTENT_BYTES = 32_768;
export const MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES = 128;

export type ProductWorkspaceId = string;
export type ProductSessionId = string;
export type ProductForkId = string;
export type ProductReviewId = string;
export type ProductProviderProfileId = string;
export type ProductMigrationReceiptId = string;

export const PRODUCT_SESSION_STATUSES = [
  "idle",
  "running",
  "error",
  "needs_attention",
  "archived",
] as const;
export type ProductSessionStatus = (typeof PRODUCT_SESSION_STATUSES)[number];

/**
 * Outcome of the most recently finished turn. `ProductSessionStatus` cannot
 * express it: a session that just succeeded and one that never ran are both
 * `idle`, so the sidebar could not draw a success mark before this field
 * existed (R3, migration 016).
 */
export const PRODUCT_SESSION_OUTCOMES = [
  "success",
  "failed",
  "cancelled",
] as const;
export type ProductSessionOutcome = (typeof PRODUCT_SESSION_OUTCOMES)[number];

export const PRODUCT_PROVIDER_TYPES = [
  "openai",
  "openai-responses",
  "anthropic",
  "ollama",
  "fake",
] as const;
export type ProductProviderType = (typeof PRODUCT_PROVIDER_TYPES)[number];

export const PRODUCT_THEME_PREFERENCES = ["light", "dark", "system"] as const;
export type ProductThemePreference =
  (typeof PRODUCT_THEME_PREFERENCES)[number];

export const PRODUCT_APPROVAL_PREFERENCES = ["ask", "auto", "never"] as const;
export type ProductApprovalPreference =
  (typeof PRODUCT_APPROVAL_PREFERENCES)[number];

export const PRODUCT_REASONING_PREFERENCES = [
  "default",
  "low",
  "medium",
  "high",
] as const;
export type ProductReasoningPreference =
  (typeof PRODUCT_REASONING_PREFERENCES)[number];

export const MAX_PRODUCT_MAX_STEPS = 256;

export const PRODUCT_WORKSPACE_KINDS = ["folder", "repo"] as const;
export type ProductWorkspaceKind = (typeof PRODUCT_WORKSPACE_KINDS)[number];

export interface ProductWorkspace {
  id: ProductWorkspaceId;
  canonical_root: string;
  kind: ProductWorkspaceKind;
  display_name: string;
  pinned: boolean;
  last_opened_at: string;
  created_at: string;
  updated_at: string;
}

export interface ProductRuntimeBinding {
  ordinal: number;
  runtime_session_id: string;
  latest_job_id: string;
  latest_run_id: string;
}

export interface ProductSession {
  id: ProductSessionId;
  workspace_id: ProductWorkspaceId;
  title: string;
  status: ProductSessionStatus;
  runtime_binding?: ProductRuntimeBinding;
  parent_session_id?: ProductSessionId;
  fork_point_run_id?: string;
  fork_point_seq?: number;
  /**
   * Absent means no turn has finished yet; the API omits both fields together
   * rather than sending `null`, and the parser rejects a lone one.
   */
  last_outcome?: ProductSessionOutcome;
  last_outcome_at?: string;
  created_at: string;
  updated_at: string;
}

/** `GET /product/runtime` — bounded runtime facts for the rail footer (§7.1). */
export const PRODUCT_CONNECTION_STATUSES = ["connected"] as const;
export type ProductConnectionStatus =
  (typeof PRODUCT_CONNECTION_STATUSES)[number];

export const PRODUCT_STORE_STATUSES = ["ready", "unavailable"] as const;
export type ProductStoreStatus = (typeof PRODUCT_STORE_STATUSES)[number];

export const PRODUCT_RESUME_HEALTH_STATUSES = [
  "healthy",
  "needs_attention",
] as const;
export type ProductResumeHealthStatus =
  (typeof PRODUCT_RESUME_HEALTH_STATUSES)[number];

export const PRODUCT_EXECUTION_ADAPTERS = ["local"] as const;
export type ProductExecutionAdapter =
  (typeof PRODUCT_EXECUTION_ADAPTERS)[number];

export const PRODUCT_EXECUTION_WORKSPACE_KINDS = [
  "folder",
  "repo",
  "task",
] as const;
export type ProductExecutionWorkspaceKind =
  (typeof PRODUCT_EXECUTION_WORKSPACE_KINDS)[number];

export interface ProductExecutionCapabilities {
  filesystem_read: boolean;
  filesystem_write: boolean;
  process_run: boolean;
  process_stdio: boolean;
  observations: boolean;
  process_background: boolean;
  process_pty: boolean;
  workspace_checkpoints: boolean;
  artifact_projection: boolean;
}

export interface ProductExecutionEnvironmentInfo {
  adapter: ProductExecutionAdapter;
  workspace_kind: ProductExecutionWorkspaceKind;
  workspace_digest: string;
  capabilities: ProductExecutionCapabilities;
}

export interface ProductAgentRuntimeInfo {
  selector: string;
  workspace_source_authorized: boolean;
  workspace_instructions_enabled: boolean;
  allow_remediation_procedures: boolean;
  max_procedure_selections: number;
}

export interface ProductResumeHealth {
  status: ProductResumeHealthStatus;
  workspace_count: number;
  session_count: number;
  bound_session_count: number;
  running_session_count: number;
  needs_attention_session_count: number;
}

export interface ProductRuntimeInfo {
  api_version: string;
  connection: ProductConnectionStatus;
  product_store: ProductStoreStatus;
  execution_environment: ProductExecutionEnvironmentInfo;
  agent: ProductAgentRuntimeInfo;
  resume_health?: ProductResumeHealth;
}

export const PRODUCT_REVIEW_TARGET_KINDS = [
  "uncommitted",
  "base",
  "commit",
] as const;
export type ProductReviewTargetKind =
  (typeof PRODUCT_REVIEW_TARGET_KINDS)[number];

export const PRODUCT_REVIEW_STATUSES = [
  "queued",
  "running",
  "pass",
  "findings",
  "partial",
  "stale",
  "needs_attention",
  "unavailable",
  "cancelled",
  "error",
] as const;
export type ProductReviewStatus = (typeof PRODUCT_REVIEW_STATUSES)[number];

export const PRODUCT_REVIEW_CONCLUSIONS = [
  "pass",
  "findings",
  "partial",
  "stale",
  "unavailable",
  "cancelled",
  "error",
] as const;
export type ProductReviewConclusion =
  (typeof PRODUCT_REVIEW_CONCLUSIONS)[number];

export const PRODUCT_REVIEW_SEVERITIES = [
  "critical",
  "high",
  "medium",
  "low",
  "info",
] as const;
export type ProductReviewSeverity = (typeof PRODUCT_REVIEW_SEVERITIES)[number];

export const PRODUCT_REVIEW_CONFIDENCES = ["high", "medium", "low"] as const;
export type ProductReviewConfidence =
  (typeof PRODUCT_REVIEW_CONFIDENCES)[number];

export const PRODUCT_REVIEW_LOCATION_STATUSES = [
  "validated",
  "unvalidated",
  "invalid",
] as const;
export type ProductReviewLocationStatus =
  (typeof PRODUCT_REVIEW_LOCATION_STATUSES)[number];

export interface ProductReviewTargetSpec {
  kind: ProductReviewTargetKind;
  revision?: string;
}

export interface ProductReviewTargetSummary {
  schema_version: number;
  spec: ProductReviewTargetSpec;
  workspace_kind: ProductWorkspaceKind;
  workspace_digest: string;
  resolved_base?: string;
  captured_at: string;
  entries: number;
  entries_truncated: number;
  digest: string;
}

export interface ProductReviewLocation {
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
}

export interface ProductReviewEvidence {
  snippet: string;
  source: string;
  reference?: string;
}

export interface ProductReviewFinding {
  finding_id: string;
  severity: ProductReviewSeverity;
  confidence: ProductReviewConfidence;
  category: string;
  path: string;
  location: ProductReviewLocation;
  location_status: ProductReviewLocationStatus;
  title: string;
  explanation: string;
  evidence: ProductReviewEvidence[];
  rule: string;
  suggestion: string;
  status: string;
}

export interface ProductReviewStats {
  files_scanned: number;
  bytes_scanned: number;
  duration_ms: number;
  concurrency_limit: number;
  findings_total: number;
  truncated_findings: number;
}

export interface ProductReviewUnchecked {
  reason: string;
  paths: string[];
}

export interface ProductReviewResult {
  schema_version: number;
  review_id: string;
  run_id: string;
  session_id: string;
  target: ProductReviewTargetSummary;
  conclusion: ProductReviewConclusion;
  findings: ProductReviewFinding[];
  stats: ProductReviewStats;
  unchecked: ProductReviewUnchecked[];
  warnings: string[];
}

export interface ProductReview {
  id: ProductReviewId;
  product_session_id: ProductSessionId;
  workspace_id: ProductWorkspaceId;
  target: ProductReviewTargetSummary;
  status: ProductReviewStatus;
  conclusion?: ProductReviewConclusion;
  runtime_session_id?: string;
  job_id?: string;
  run_id?: string;
  result?: ProductReviewResult;
  findings_count: number;
  unchecked_count: number;
  warnings_count: number;
  created_at: string;
  updated_at: string;
  captured_at: string;
  finalized_at?: string;
}

export interface ProductReviewsResponse {
  reviews: ProductReview[];
}

export interface ProductReviewFindingPageItem {
  finding: ProductReviewFinding;
  sort_key: string;
}

export interface ProductReviewFindingsResponse {
  findings: ProductReviewFindingPageItem[];
  next_cursor?: number;
}

export interface CreateProductReviewRequest {
  target: ProductReviewTargetSpec;
  idempotency_key?: string;
  max_steps?: number;
}

export interface ProductSessionRunBinding {
  product_session_id: ProductSessionId;
  ordinal: number;
  runtime_session_id: string;
  runtime_job_id: string;
  runtime_run_id: string;
  resumed_from_run_id?: string;
  bound_at: string;
}

export interface ProductFork {
  id: ProductForkId;
  parent_product_session_id: ProductSessionId;
  child_product_session_id: ProductSessionId;
  parent_workspace_id: ProductWorkspaceId;
  parent_title: string;
  source_runtime_session_id: string;
  source_runtime_job_id: string;
  source_runtime_run_id: string;
  fork_at_event_seq: number;
  idempotency_key: string;
  created_at: string;
  /**
   * Set when the child was built by editing one of the parent's messages: the
   * ledger sequence the child's seed stops before. Absent on a plain
   * terminal-boundary fork, which keeps the whole prefix.
   */
  truncate_after_message_seq?: number;
  /**
   * The message that sequence identifies, recorded so the child's seed can be
   * cut even after the parent's message ledger is gone.
   */
  truncate_after_message_id?: ProductControlId;
}

export type ProductProviderCredentialSource =
  | { source: "env"; name: string }
  | { source: "file"; path: string }
  | { source: "keyring"; service: string; account: string }
  | { source: "none" };

export interface ProductProviderProfile {
  id: ProductProviderProfileId;
  label: string;
  provider_type: ProductProviderType;
  api_base: string;
  api_key_env?: string;
  credential_source: ProductProviderCredentialSource;
  default_model?: string;
  created_at: string;
  updated_at: string;
  catalog_revision: string;
}

export interface ProductProviderSelection {
  profile_id?: ProductProviderProfileId;
  model: string;
  approval: ProductApprovalPreference;
  max_steps: number;
}

export interface ProductSessionModelConfig {
  product_session_id: ProductSessionId;
  profile_id?: ProductProviderProfileId;
  model: string;
  reasoning: ProductReasoningPreference;
  max_steps: number;
  revision: number;
  updated_at: string;
}

export interface UpdateProductSessionModelConfigRequest {
  profile_id?: ProductProviderProfileId;
  model: string;
  reasoning: ProductReasoningPreference;
  max_steps: number;
  expected_revision?: number;
}

export interface ProductModelDescriptor {
  id: string;
  context_window?: number;
  supports_reasoning: boolean;
  supported_reasoning: ProductReasoningPreference[];
  reasoning_unavailable_reason?: string;
}

export interface ProductProviderModelsResponse {
  profile_id: ProductProviderProfileId;
  default_model?: string;
  models: ProductModelDescriptor[];
}

export interface ProductSessionRunModelView {
  product_session_id: ProductSessionId;
  ordinal: number;
  runtime_run_id: string;
  profile_id?: ProductProviderProfileId;
  model: string;
  reasoning: ProductReasoningPreference;
  max_steps: number;
  provider_type?: string;
  wire_protocol?: string;
  endpoint?: string;
  catalog_revision?: string;
  safe_config_digest?: string;
  context_window?: number;
  pricing_source?: string;
  pricing_version?: string;
  pricing_currency?: string;
  pricing_availability?: ProductPricingAvailability;
  per_mtok_prompt?: number;
  per_mtok_completion?: number;
  per_mtok_cache_read?: number;
}

export interface ProductSessionRunModelsResponse {
  runs: ProductSessionRunModelView[];
}

export const PRODUCT_PRICING_AVAILABILITIES = [
  "priced",
  "local_zero",
  "unpriced",
] as const;
export type ProductPricingAvailability =
  (typeof PRODUCT_PRICING_AVAILABILITIES)[number];

export interface ProductUsage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  cached_tokens: number;
}

export interface ProductCostBreakdown {
  currency: string;
  availability: ProductPricingAvailability;
  total_usd?: number;
  prompt_usd?: number;
  completion_usd?: number;
  cache_read_usd?: number;
  pricing_source?: string;
  pricing_version?: string;
}

export interface ProductContextOccupancy {
  token_estimate: number;
  context_window?: number;
  estimate_kind: string;
  included_history_messages: number;
  dropped_history_messages: number;
  compaction_mode?: string;
  compaction_degraded: boolean;
  compaction_auto_triggered: boolean;
  compacted_history_messages: number;
  compaction_source_messages: number;
  compaction_prompt_version?: string;
  prompt_hash?: string;
}

export interface ProductRunUsage {
  runtime_run_id: string;
  ordinal: number;
  model: string;
  usage: ProductUsage;
  cost?: ProductCostBreakdown;
  context?: ProductContextOccupancy;
  steps: number;
  tool_calls: number;
}

export interface ProductSessionUsageResponse {
  product_session_id: ProductSessionId;
  totals: ProductUsage;
  totals_cost?: ProductCostBreakdown;
  latest_context?: ProductContextOccupancy;
  runs: ProductRunUsage[];
  partial_reasons: string[];
}

/** Characters of a compaction summary the API returns. */
export const MAX_PRODUCT_COMPACTION_SUMMARY_CHARS = 400;

/**
 * Bounded facts about one manual compaction (R9). `triggered` describes *this*
 * call; the remaining fields describe the state the session holds afterwards, so
 * "already compacted", "nothing to compact", and "the breaker refuses" stay
 * distinguishable. The response carries no provider message — only a typed
 * `failure_code`.
 */
export interface ProductSessionCompaction {
  product_session_id: ProductSessionId;
  /** Absent when the session has no run yet, i.e. there is nothing to compact. */
  runtime_run_id?: string;
  triggered: boolean;
  /**
   * `none` | `deterministic` | `model_generated` | `automatic` | `degraded` |
   * `disabled`. Kept as a string so a newer mode is not a schema error.
   */
  mode: string;
  /** The stored summary is the deterministic fallback, not a model summary. */
  degraded: boolean;
  /** Persisted session history, not this request's outcome. */
  consecutive_failures: number;
  /**
   * The session's summary model has failed to the configured threshold. Not by
   * itself a refusal; read it together with `next_attempt_after`.
   */
  circuit_open: boolean;
  /** Present only while the automatic path is actually being refused. */
  next_attempt_after?: string;
  model?: string;
  prompt_version?: string;
  source_message_count: number;
  /** Bounded excerpt; absent when the session holds no summary. */
  summary?: string;
  /** The excerpt above is a fragment of a longer summary. */
  summary_truncated: boolean;
  /** Token estimate of the prompt the next turn will carry. */
  token_estimate: number;
  /** Typed classification of a failed summary call, never a provider message. */
  failure_code?: string;
}


export const PRODUCT_FILE_KINDS = ["file", "directory"] as const;
export type ProductFileKind = (typeof PRODUCT_FILE_KINDS)[number];

export interface ProductFileEntry {
  path: string;
  kind: ProductFileKind;
  size: number;
  modified?: string;
}

export interface ProductFilesResponse {
  workspace_id: string;
  prefix: string;
  entries: ProductFileEntry[];
  next_cursor?: string;
  truncated: boolean;
  scan_limit_reached: boolean;
}

export interface ProductImageMetadata {
  width: number;
  height: number;
  format: string;
}

export interface ProductFileContentEnvelope {
  path: string;
  mime: string;
  size: number;
  truncated: boolean;
  text?: string;
  encoding?: string;
  image?: ProductImageMetadata;
  preview_allowed: boolean;
  validation_error?: string;
}

export const PRODUCT_ARTIFACT_SOURCE_KINDS = [
  "report",
  "task_state",
  "trace",
  "registered",
  "tool_artifact",
] as const;
export type ProductArtifactSourceKind =
  (typeof PRODUCT_ARTIFACT_SOURCE_KINDS)[number];

export const PRODUCT_ARTIFACT_AVAILABILITIES = [
  "available",
  "cleaned",
  "invalid",
  "too_large",
] as const;
export type ProductArtifactAvailability =
  (typeof PRODUCT_ARTIFACT_AVAILABILITIES)[number];

export const PRODUCT_ARTIFACT_PREVIEW_KINDS = [
  "text",
  "raster_image",
  "download_only",
  "unavailable",
] as const;
export type ProductArtifactPreviewKind =
  (typeof PRODUCT_ARTIFACT_PREVIEW_KINDS)[number];

export interface ProductArtifactView {
  artifact_id: string;
  safe_name: string;
  mime: string;
  size?: number;
  sha256?: string;
  source_run_id: string;
  source_kind: ProductArtifactSourceKind;
  availability: ProductArtifactAvailability;
  preview_kind: ProductArtifactPreviewKind;
  image?: ProductImageMetadata;
  validation_error?: string;
}

export interface ProductArtifactsResponse {
  session_id: string;
  artifacts: ProductArtifactView[];
  partial_reasons: string[];
}

export interface ProductArtifactContentEnvelope {
  artifact_id: string;
  safe_name: string;
  mime: string;
  size: number;
  truncated: boolean;
  text?: string;
  encoding?: string;
  image?: ProductImageMetadata;
  preview_allowed: boolean;
  validation_error?: string;
}

export const PRODUCT_DIFF_OPS = [
  "create",
  "update",
  "delete",
  "modified",
  "unknown",
] as const;
export type ProductDiffOp = (typeof PRODUCT_DIFF_OPS)[number];

export const PRODUCT_DIFF_SOURCES = ["run", "git"] as const;
export type ProductDiffSource = (typeof PRODUCT_DIFF_SOURCES)[number];

export interface ProductDiffEntry {
  path: string;
  op: ProductDiffOp;
  source: ProductDiffSource;
  source_run_id?: string;
  diff?: string;
  binary: boolean;
  truncated: boolean;
  reconstructable: boolean;
}

export interface ProductSessionDiffResponse {
  session_id: string;
  scope: string;
  entries: ProductDiffEntry[];
  partial_reasons: string[];
}

export interface ProductPreferences {
  schema_version: number;
  revision: number;
  theme: ProductThemePreference;
  default_approval_policy: ProductApprovalPreference;
  active_workspace_id?: ProductWorkspaceId;
  active_session_id?: ProductSessionId;
  provider_selection?: ProductProviderSelection;
}

export interface CreateProductWorkspaceRequest {
  root: string;
  kind: ProductWorkspaceKind;
  display_name?: string;
  pinned?: boolean;
}

export interface CreateProductSessionRequest {
  workspace_id: ProductWorkspaceId;
  title?: string;
}

export interface CreateProductForkRequest {
  fork_at_run_id: string;
  title?: string;
  idempotency_key: string;
  /**
   * Edit-and-resend (R6): the parent ledger sequence of the user message being
   * replaced. The child keeps the parent prefix up to that message; the edited
   * text is sent into the child as its first message. The fork endpoint itself
   * never accepts the text.
   */
  truncate_after_message_seq?: number;
}

export interface ProductForkResponse {
  fork: ProductFork;
  session: ProductSession;
}

export interface ProductForksResponse {
  forks: ProductFork[];
}

export interface UpdateProductSessionRequest {
  title?: string;
  archived?: boolean;
}

export interface CreateProductProviderProfileRequest {
  label: string;
  provider_type: ProductProviderType;
  api_base: string;
  api_key_env?: string;
  default_model?: string;
  expected_revision?: string;
}

export type UpdateProductProviderProfileRequest =
  CreateProductProviderProfileRequest;

/**
 * `POST /product/provider-onboarding` — the only request body that carries a
 * raw provider credential. The server hands it to the OS credential store and
 * never returns it; the client must not persist it either (no state, no
 * localStorage, no logs).
 */
export interface OnboardProductProviderRequest {
  profile_id?: ProductProviderProfileId;
  label: string;
  provider_type: ProductProviderType;
  api_base: string;
  model: string;
  make_default?: boolean;
  expected_revision?: string;
  credential: string;
}

export interface ProductProviderOnboardingProbe {
  inventory_count: number;
  streaming_supported: boolean;
  native_tool_calls_supported: boolean;
  usage_supported: boolean;
}

export interface ProductProviderOnboardingReceipt {
  profile_id: ProductProviderProfileId;
  label: string;
  provider_type: ProductProviderType;
  api_base: string;
  model: string;
  catalog_revision: string;
  credential_source: string;
  probe: ProductProviderOnboardingProbe;
  selected: boolean;
}

export interface UpdateProductPreferencesRequest {
  schema_version: number;
  expected_revision?: number;
  theme: ProductThemePreference;
  default_approval_policy?: ProductApprovalPreference;
  active_workspace_id?: ProductWorkspaceId;
  active_session_id?: ProductSessionId;
  provider_selection?: ProductProviderSelection;
}

export interface ProductWorkspacesResponse {
  workspaces: ProductWorkspace[];
}

export interface ProductSessionsResponse {
  sessions: ProductSession[];
  /** Opaque token for the next page. Absent on the last page. */
  next_cursor?: string;
}

/** Longest session-list page the API will serve. */
export const MAX_PRODUCT_SESSION_PAGE_LIMIT = 200;

export interface ProductProviderProfilesResponse {
  catalog_revision: string;
  provider_profiles: ProductProviderProfile[];
}

export const PRODUCT_TRANSCRIPT_STATUSES = ["complete", "partial"] as const;
export type ProductTranscriptStatus =
  (typeof PRODUCT_TRANSCRIPT_STATUSES)[number];

export const PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES = [
  "missing_run_mapping",
  "runtime_run_missing",
  "runtime_state_unavailable",
  "runtime_identity_mismatch",
  "missing_event_range",
  "corrupt_event",
  "corrupt_artifact",
  "cleaned_history",
  "response_limit_reached",
  "unknown_event_type",
] as const;
export type ProductTranscriptPartialReasonCode =
  (typeof PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES)[number];

/**
 * Gap reason codes that do not claim a specific sequence position: they report
 * the run as incomplete, corrupt, unreadable, or cut short by the response
 * limit. A client cannot hold them to an exact range, so, exactly as before,
 * any one of them explains a tail that ends before `last_event_seq`.
 *
 * The withheld-row code is deliberately absent: it claims exact durable
 * positions, so it is held to them (see below).
 */
const PRODUCT_TRANSCRIPT_POSITIONLESS_GAP_REASON_CODES = new Set<
  ProductTranscriptPartialReasonCode
>([
  "runtime_state_unavailable",
  "runtime_identity_mismatch",
  "missing_event_range",
  "corrupt_event",
  "corrupt_artifact",
  "cleaned_history",
  "response_limit_reached",
]);

/**
 * Whether a reason belongs to this run ordinal, and to this runtime run when
 * both sides name one.
 */
function partialReasonNamesRun(
  reason: ProductTranscriptPartialReason,
  runOrdinal: number,
  runId?: string,
): boolean {
  if (reason.run_ordinal !== runOrdinal) {
    return false;
  }
  return runId === undefined || reason.run_id === undefined || reason.run_id === runId;
}

/**
 * Whether a withheld-row reason for this run ordinal covers a sequence step from
 * `expectedSeq` to `nextSeq`.
 *
 * The server withholds exactly the durable rows whose kind entered the canonical
 * event contract after the version this client declared, and reports the lowest
 * and highest withheld sequence as one `unknown_event_type` reason per run. Only
 * that declared reason makes a non-contiguous delivered sequence valid.
 */
function unknownEventTypeCoversStep(
  reasons: readonly ProductTranscriptPartialReason[],
  runOrdinal: number,
  expectedSeq: number,
  nextSeq: number,
  runId?: string,
): boolean {
  if (nextSeq <= expectedSeq) {
    return false;
  }
  return reasons.some(
    (reason) =>
      partialReasonNamesRun(reason, runOrdinal, runId) &&
      reason.code === "unknown_event_type" &&
      (reason.expected_seq ?? 0) <= expectedSeq &&
      (reason.observed_seq ?? 0) >= nextSeq - 1,
  );
}

export interface ProductTranscriptPartialReason {
  code: ProductTranscriptPartialReasonCode;
  run_ordinal?: number;
  run_id?: string;
  expected_seq?: number;
  observed_seq?: number;
}

export type ProductTranscriptFallbackSource = "report";

export interface ProductTranscriptFallback {
  source: ProductTranscriptFallbackSource;
  status: string;
  summary?: string;
}

export interface ProductTranscriptRunSegment {
  binding: ProductSessionRunBinding;
  inherited: boolean;
  source_product_session_id?: ProductSessionId;
  run_status: RunStatus;
  observed_through_seq: number;
  last_event_seq: number;
  events: ProductJobStreamEvent[];
  /**
   * Additive R2b marker: `true` only when this segment projects a salvaged
   * partial — an `llm_message` whose `aborted` marker is `true`, meaning the
   * user stopped the turn after it had already published text. Absent means
   * "not a salvaged partial" (a complete answer, a failed turn, or a stop that
   * produced nothing), which is also what a response written before the field
   * existed says as well.
   */
  answer_aborted?: boolean;
  fallback?: ProductTranscriptFallback;
}

/** Longest transcript run page the API will serve for `limit_runs`. */
export const MAX_PRODUCT_TRANSCRIPT_PAGE_RUNS = 64;

export interface ProductTranscriptResponse {
  product_session_id: ProductSessionId;
  workspace_id: ProductWorkspaceId;
  status: ProductTranscriptStatus;
  partial_reasons: ProductTranscriptPartialReason[];
  segments: ProductTranscriptRunSegment[];
  /**
   * Ordinal of the oldest segment in this page while strictly older runs
   * remain; absent on a legacy response and on the oldest page.
   */
  next_before_ordinal?: number;
  /**
   * Present on every cursor page and exactly `next_before_ordinal !== undefined`;
   * absent on a legacy response that passed no pagination parameter.
   */
  has_more?: boolean;
}

export interface ProductAuthorizationOutcome {
  event: string;
  seq: number;
}

export interface ProductAuthorizationRecord {
  call_id: string;
  job_id: string;
  run_id: string;
  tool: string;
  args: unknown;
  reason: string;
  status: string;
  decided_via?: string | null;
  requested_at: string;
  updated_at: string;
  outcome?: ProductAuthorizationOutcome;
}

export interface ProductAuthorizationsResponse {
  session_id: ProductSessionId;
  authorizations: ProductAuthorizationRecord[];
  truncated: boolean;
}

export interface CreateProductPreviewRequest {
  path: string;
}

export interface ProductPreviewSession {
  preview_id: string;
  workspace_id: ProductWorkspaceId;
  entry: string;
  /** Absolute loopback URL embedding the session token. Keep out of logs. */
  url: string;
  created_at: string;
  expires_at: string;
}

export type M1BrowserMigrationSource = "web_m1_local_storage";

export interface M1WorkspaceImport {
  source_id: string;
  root: string;
  kind: ProductWorkspaceKind;
  display_name: string;
  pinned: boolean;
  last_opened_at: string;
}

export interface M1SessionImport {
  source_id: string;
  source_workspace_id: string;
  title: string;
  created_at: string;
  updated_at: string;
  legacy_active_job_id?: string;
  legacy_active_run_id?: string;
  legacy_resumed_from_run_id?: string;
  legacy_has_durable_turn?: boolean;
}

export interface M1ProviderProfileImport {
  source_id: string;
  label: string;
  provider_type: ProductProviderType;
  api_base: string;
  api_key_env?: string;
  default_model?: string;
  updated_at: string;
}

export interface M1ProviderSelectionImport {
  source_profile_id?: string;
  model: string;
  approval: ProductApprovalPreference;
  max_steps: number;
}

export interface M1SafePreferencesImport {
  theme?: ProductThemePreference;
  source_active_workspace_id?: string;
  source_active_session_id?: string;
  provider_selection?: M1ProviderSelectionImport;
}

export interface M1BrowserMigrationRequest {
  source: M1BrowserMigrationSource;
  source_schema_version: number;
  idempotency_key: string;
  workspaces: M1WorkspaceImport[];
  sessions: M1SessionImport[];
  provider_profiles: M1ProviderProfileImport[];
  safe_preferences: M1SafePreferencesImport;
}

export interface M1WorkspaceIdMapping {
  source_id: string;
  workspace_id: ProductWorkspaceId;
}

export interface M1SessionIdMapping {
  source_id: string;
  product_session_id: ProductSessionId;
}

export interface M1ProviderProfileIdMapping {
  source_id: string;
  provider_profile_id: ProductProviderProfileId;
}

export const M1_MIGRATION_DISPOSITIONS = [
  "applied",
  "already_applied",
] as const;
export type M1MigrationDisposition =
  (typeof M1_MIGRATION_DISPOSITIONS)[number];

export const M1_MIGRATION_ISSUE_CODES = [
  "invalid_workspace",
  "missing_workspace",
  "invalid_runtime_hint",
  "ambiguous_runtime_binding",
  "runtime_binding_not_found",
  "invalid_preference_reference",
  "preference_write_conflict",
] as const;
export type M1MigrationIssueCode =
  (typeof M1_MIGRATION_ISSUE_CODES)[number];

export interface M1MigrationIssue {
  code: M1MigrationIssueCode;
  entity: string;
  source_id?: string;
}

export interface M1BrowserMigrationResponse {
  source_schema_version: number;
  idempotency_key: string;
  receipt_id: ProductMigrationReceiptId;
  disposition: M1MigrationDisposition;
  workspace_mappings: M1WorkspaceIdMapping[];
  session_mappings: M1SessionIdMapping[];
  provider_profile_mappings: M1ProviderProfileIdMapping[];
  issues: M1MigrationIssue[];
  applied_at: string;
}

export const PRODUCT_ERROR_CODES = [
  "product_not_found",
  "product_invalid_input",
  "product_store_unavailable",
  "product_session_active",
  "product_session_workspace_mismatch",
  "product_session_resume_conflict",
  "product_session_runtime_state_missing",
  "product_session_runtime_state_corrupt",
  "product_binding_corrupt",
  "product_revision_conflict",
  "product_memory_invalid_slug",
  "product_memory_not_found",
  "product_memory_conflict",
  "project_trust_required",
  "migration_idempotency_conflict",
  "product_control_conflict",
  "product_control_rejected",
  "product_fork_conflict",
  "product_fork_source_invalid",
  "review_target_unavailable",
  "review_conflict",
  "review_unavailable",
  "product_storage_failure",
] as const;
export type ProductErrorCode = (typeof PRODUCT_ERROR_CODES)[number];

export interface ApiErrorResponse {
  code: string;
  error: string;
}

export type ProductToolExecutionStatus =
  | "ok"
  | "error"
  | "rejected"
  | "partial_success";

export type ProductToolRiskLevel = "low" | "high";

export interface ProductToolExecutionMetadata {
  status: ProductToolExecutionStatus;
  error_code?: string;
  security_event_type?: string;
  risk_level: ProductToolRiskLevel;
  read_only: boolean;
  affected_paths: string[];
  workspace_changed: boolean;
  diff_summary: string[];
}

export interface ProductToolResult {
  call_id: string;
  output: string;
  mutations?: ToolMutation[];
  metadata: ProductToolExecutionMetadata;
  /** Rich result detail, validated when the server sends it. */
  envelope?: ToolOutputEnvelope;
  /** Procedures applied during this tool call */
  procedure_applications?: ProcedureApplication[];
  /** Procedure deviations during this tool call */
  procedure_deviations?: ProcedureDeviation[];
}

/**
 * What one bounded compaction request elided, as the runtime recorded it.
 *
 * These facts describe the shrink request the compaction probe sent, not the
 * turn's own prompt: the runtime builds the probe's material, prunes it, and
 * stamps what it pruned onto the same `PromptBuilt` metadata the turn publishes.
 *
 * This is the shared decoded type rather than a product copy of it, because one
 * reader decodes the record for both arrival paths. Its member types describe the
 * decoded shape, not the wire shape: on the wire the runtime omits each member
 * while it is zero or empty and omits `pruning` entirely when the turn trimmed
 * nothing, so a member that arrived as nothing decodes to the default. A filled
 * zero is therefore "the runtime reported nothing here", never a measurement the
 * runtime published; compare `pruning` against `undefined` to tell a turn that
 * pruned nothing from one whose facts were not carried.
 *
 * Nothing here is partial once `pruning` exists. Both paths judge each carried
 * member by the same predicates, and a member that fails them withholds the whole
 * fact set — the restored path by refusing the record, the live path by answering
 * `undefined` — so a decoding that returns facts returns all of them, and a
 * shorter digest list is never presented as a complete one.
 */
export type ProductPromptPruning = PromptPruningFacts;

export interface ProductPromptBuildMetadata {
  prompt_hash: string;
  stable_prefix_hash: string;
  workspace_fingerprint: string;
  tool_signature: string;
  token_estimate: number;
  included_history_messages: number;
  dropped_history_messages: number;
  prompt_cache_key?: string;
  pruning?: ProductPromptPruning;
}

type ProductUnchangedStreamEvent = Exclude<
  StreamEvent,
  | { type: "tool_call_completed" }
  | { type: "tool_call_failed" }
  | { type: "prompt_built" }
>;

export type ProductStreamEvent =
  | ProductUnchangedStreamEvent
  | {
      type: "tool_call_completed";
      call_id: string;
      result: ProductToolResult;
    }
  | {
      type: "tool_call_failed";
      call_id: string;
      error: ToolError;
      metadata: ProductToolExecutionMetadata;
    }
  | {
      type: "prompt_built";
      metadata: ProductPromptBuildMetadata;
    };

export type ProductControlId = string;

export const PRODUCT_CONTROL_KINDS = ["steer", "followup"] as const;
export type ProductControlKind = (typeof PRODUCT_CONTROL_KINDS)[number];

export const PRODUCT_CONTROL_STATUSES = [
  "pending",
  "accepted",
  "applied",
  "dropped",
  "abandoned",
  "revoked",
] as const;
export type ProductControlStatus = (typeof PRODUCT_CONTROL_STATUSES)[number];

export interface ProductControl {
  id: ProductControlId;
  product_session_id: ProductSessionId;
  kind: ProductControlKind;
  idempotency_key?: string;
  content: string;
  status: ProductControlStatus;
  run_id?: string;
  seq: number;
  created_at: string;
  applied_at?: string;
}

export interface CreateProductControlRequest {
  content: string;
  idempotency_key?: string;
  /**
   * Attachment references the message carries, in the order the user added
   * them. Additive: absent serialises exactly as before, and a server older
   * than the field refuses it rather than ignoring it.
   */
  attachments?: ProductMessageAttachmentRequest[];
}

/** Lifecycle of an attachment payload, from the durable row's own state. */
export const PRODUCT_ATTACHMENT_AVAILABILITIES = [
  "available",
  "missing",
  "corrupt",
  "expired",
] as const;
export type ProductAttachmentAvailability =
  (typeof PRODUCT_ATTACHMENT_AVAILABILITIES)[number];

/** Lifecycle of an attachment row; a fresh upload is `staged`. */
export const PRODUCT_ATTACHMENT_STATUSES = ["staged", "referenced", "expired"] as const;
export type ProductAttachmentStatus = (typeof PRODUCT_ATTACHMENT_STATUSES)[number];

/**
 * One attachment reference on a durable message. `content_type`, `size`, and
 * `sha256` are the server-verified values, never the client's claim, so a
 * transcript cannot report a type the bytes do not have. `name` is display
 * metadata and is never a path.
 */
export interface ProductMessageAttachmentRef {
  attachment_id: string;
  content_type: string;
  size: number;
  sha256: string;
  name?: string;
  availability: ProductAttachmentAvailability;
  /**
   * Why a reference with fine bytes still did not reach the model. `undefined`
   * for every reference that was projected as sent, for every non-image
   * reference, and for every response a pre-image server sends.
   */
  degradation?: string;
}

/** The reference a client sends with a message: an id and a display name. */
export interface ProductMessageAttachmentRequest {
  attachment_id: string;
  name?: string;
}

/** The `201` body of one upload. Every field is server-verified. */
export interface ProductAttachmentUpload {
  attachment_id: string;
  product_session_id: string;
  /** The locally verified type; the client's `Content-Type` is never echoed. */
  content_type: string;
  size: number;
  sha256: string;
  name?: string;
  status: ProductAttachmentStatus;
  expires_at?: string;
  /** Always present, always an array: "no warnings" is not "no answer". */
  warnings: string[];
}

/** Upper bound the server accepts for one message's attachment set. */
export const MAX_MESSAGE_ATTACHMENTS = 32;

export interface ProductControlsResponse {
  controls: ProductControl[];
}

export const PRODUCT_MESSAGE_DELIVERIES = ["successor", "current_run"] as const;
export type ProductMessageDelivery = (typeof PRODUCT_MESSAGE_DELIVERIES)[number];
export const PRODUCT_MESSAGE_STATUSES = [
  "queued",
  "intervention_requested",
  "applied_current_run",
  "claimed_successor",
  "needs_attention",
  "revoked",
] as const;
export type ProductMessageStatus = (typeof PRODUCT_MESSAGE_STATUSES)[number];

export interface ProductMessage {
  id: ProductControlId;
  product_session_id: ProductSessionId;
  content: string;
  requested_delivery: ProductMessageDelivery;
  actual_delivery?: ProductMessageDelivery;
  status: ProductMessageStatus;
  seq: number;
  run_id?: string;
  successor_run_id?: string;
  created_at: string;
  applied_at?: string;
  /**
   * Explicit successor-queue position (R4). `undefined` means the message has
   * never been moved and keeps its `seq` (creation) order, so a client sorts
   * the queue by `queue_order ?? seq`. Additive: an older payload parses
   * unchanged.
   */
  queue_order?: number;
  reason?: string;
  /**
   * Attachments this message references. Additive and omitted when empty, so a
   * message with no attachments parses and serialises exactly as before.
   */
  attachments?: ProductMessageAttachmentRef[];
}

export type CreateProductMessageRequest = CreateProductControlRequest;
export interface ProductMessagesResponse {
  messages: ProductMessage[];
  next_after_seq?: number;
  next_before_seq?: number;
}

/** Optional body of a message promotion; absent keeps the historical delivery. */
export interface PromoteProductMessageRequest {
  delivery?: ProductMessageDelivery;
}

/**
 * Atomic successor-queue reorder (R4). `ordered_ids` must name exactly the
 * session's current queued successor messages, in the wanted order: a stale or
 * partial list is refused instead of merged, which is what turns a concurrent
 * promote, revoke, or drain into a typed `product_control_conflict`.
 */
export interface ReorderProductMessagesRequest {
  ordered_ids: ProductControlId[];
}

/** The authoritative successor queue after a reorder or a promotion. */
export interface ProductQueueResponse {
  messages: ProductMessage[];
}

/** Upper bound the server accepts for one reorder list. */
export const MAX_PENDING_PRODUCT_MESSAGES = 64;
/** Longest search term the server accepts, in UTF-8 bytes. */
export const MAX_PRODUCT_SEARCH_QUERY_BYTES = 128;
/** Search page bounds the server accepts. */
export const MAX_PRODUCT_SEARCH_LIMIT = 100;

/**
 * One session-scoped message search hit (R7 step 1). The excerpt is bounded and
 * already redacted server-side, so it is rendered verbatim rather than
 * re-truncated or re-highlighted.
 */
export interface ProductMessageSearchHit {
  /** Ledger sequence, the same `seq` the message listing publishes. */
  message_seq: number;
  snippet: string;
  created_at: string;
}

export interface ProductMessagesSearchResponse {
  hits: ProductMessageSearchHit[];
  /** Token for the next page; absent on the last page. Bound to the term. */
  next_cursor?: string;
}

export const PRODUCT_SEARCH_SOURCES = ["message", "trace"] as const;
export type ProductSearchSource = (typeof PRODUCT_SEARCH_SOURCES)[number];

/**
 * One hit of the unified product search (R7 step 2). `scope` on the response is
 * the server's canonical echo of the requested scope. A trace hit's `seq` is the
 * trace record's own sequence and must never be used to locate a message;
 * `created_at` is absent for legacy trace lines and is never invented.
 */
export interface ProductSearchHit {
  session_id: ProductSessionId;
  source: ProductSearchSource;
  seq: number;
  run_id?: string;
  run_ordinal?: number;
  snippet: string;
  created_at?: string;
}

export interface ProductSearchResponse {
  scope: string;
  hits: ProductSearchHit[];
  /** Token for the next page; absent when the search is exhausted. */
  next_cursor?: string;
}

export type ProductControlStatusFilter = ProductControlStatus | "all";

export interface ProductJobStreamEvent {
  seq: number;
  event: ProductStreamEvent;
}

export class ProductApiSchemaError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProductApiSchemaError";
  }
}

type UnknownRecord = Record<string, unknown>;

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function schemaError(path: string, expectation: string): never {
  throw new ProductApiSchemaError(`${path} must be ${expectation}`);
}

function expectRecord(value: unknown, path: string): UnknownRecord {
  if (!isRecord(value)) {
    return schemaError(path, "an object");
  }
  return value;
}

function expectOnlyKeys(
  value: UnknownRecord,
  allowed: readonly string[],
  path: string,
): void {
  const allowedKeys = new Set(allowed);
  const unknown = Object.keys(value).find((key) => !allowedKeys.has(key));
  if (unknown) {
    schemaError(`${path}.${unknown}`, "a known field");
  }
}

function expectString(
  value: unknown,
  path: string,
  options: {
    nonEmpty?: boolean;
    maxBytes?: number;
    noControlCharacters?: boolean;
  } = {},
): string {
  if (typeof value !== "string") {
    return schemaError(path, "a string");
  }
  if (options.nonEmpty && value.trim().length === 0) {
    return schemaError(path, "a non-empty string");
  }
  if (
    options.maxBytes !== undefined &&
    new TextEncoder().encode(value).length > options.maxBytes
  ) {
    return schemaError(path, `at most ${options.maxBytes} UTF-8 bytes`);
  }
  if (
    options.noControlCharacters &&
    /[\u0000-\u001f\u007f-\u009f]/u.test(value)
  ) {
    return schemaError(path, "free of control characters");
  }
  return value;
}

function optionalString(
  value: UnknownRecord,
  key: string,
  path: string,
  options: {
    nonEmpty?: boolean;
    maxBytes?: number;
    noControlCharacters?: boolean;
  } = {},
): string | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectString(candidate, `${path}.${key}`, options);
}

const RFC3339_TIMESTAMP_PATTERN =
  /^(\d{4})-(\d{2})-(\d{2})[Tt](\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:[Zz]|[+-](\d{2}):(\d{2}))$/u;

function isLeapYear(year: number): boolean {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

function expectRfc3339Timestamp(value: unknown, path: string): string {
  const timestamp = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControlCharacters: true,
  });
  const match = RFC3339_TIMESTAMP_PATTERN.exec(timestamp);
  if (match === null) {
    return schemaError(path, "an RFC3339 timestamp");
  }
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const hour = Number(match[4]);
  const minute = Number(match[5]);
  const second = Number(match[6]);
  const offsetHour = match[7] === undefined ? 0 : Number(match[7]);
  const offsetMinute = match[8] === undefined ? 0 : Number(match[8]);
  const monthDays = [
    31,
    isLeapYear(year) ? 29 : 28,
    31,
    30,
    31,
    30,
    31,
    31,
    30,
    31,
    30,
    31,
  ];
  if (
    month < 1 ||
    month > 12 ||
    day < 1 ||
    day > monthDays[month - 1]! ||
    hour > 23 ||
    minute > 59 ||
    second > 60 ||
    offsetHour > 23 ||
    offsetMinute > 59
  ) {
    return schemaError(path, "an RFC3339 timestamp");
  }
  return timestamp;
}

function expectM1MigrationIdempotencyKey(
  value: unknown,
  path: string,
): string {
  const key = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES,
  });
  if (!/^[A-Za-z0-9_.:-]+$/u.test(key)) {
    return schemaError(path, "a valid migration idempotency key");
  }
  return key;
}

function optionalNullableString(
  value: UnknownRecord,
  key: string,
  path: string,
): string | null | undefined {
  const candidate = value[key];
  if (candidate === undefined) {
    return undefined;
  }
  if (candidate === null) {
    return null;
  }
  return expectString(candidate, `${path}.${key}`);
}

function expectBoolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    return schemaError(path, "a boolean");
  }
  return value;
}

function optionalBoolean(
  value: UnknownRecord,
  key: string,
  path: string,
): boolean | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectBoolean(candidate, `${path}.${key}`);
}

function expectInteger(
  value: unknown,
  path: string,
  options: { min?: number; max?: number } = {},
): number {
  if (!Number.isSafeInteger(value)) {
    return schemaError(path, "a safe integer");
  }
  const numberValue = Number(value);
  if (options.min !== undefined && numberValue < options.min) {
    return schemaError(path, `at least ${options.min}`);
  }
  if (options.max !== undefined && numberValue > options.max) {
    return schemaError(path, `at most ${options.max}`);
  }
  return numberValue;
}

function optionalInteger(
  value: UnknownRecord,
  key: string,
  path: string,
  options: { min?: number; max?: number } = {},
): number | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectInteger(candidate, `${path}.${key}`, options);
}

function optionalNumber(
  value: UnknownRecord,
  key: string,
  path: string,
): number | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  if (typeof candidate !== "number" || !Number.isFinite(candidate)) {
    return schemaError(`${path}.${key}`, "a finite number");
  }
  return candidate;
}

function optionalArray<T>(
  value: UnknownRecord,
  key: string,
  path: string,
  parseItem: (item: unknown, itemPath: string) => T,
  maxLength?: number,
): T[] | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectArray(candidate, `${path}.${key}`, parseItem, maxLength);
}

function optionalId(
  value: UnknownRecord,
  key: string,
  path: string,
): string | undefined {
  const candidate = value[key];
  if (candidate === undefined || candidate === null) {
    return undefined;
  }
  return expectId(candidate, `${path}.${key}`);
}

function expectArray<T>(
  value: unknown,
  path: string,
  parseItem: (item: unknown, itemPath: string) => T,
  maxLength?: number,
): T[] {
  if (!Array.isArray(value)) {
    return schemaError(path, "an array");
  }
  if (maxLength !== undefined && value.length > maxLength) {
    return schemaError(path, `an array with at most ${maxLength} items`);
  }
  return value.map((item, index) => parseItem(item, `${path}[${index}]`));
}

function isEnumValue<const T extends readonly string[]>(
  value: unknown,
  values: T,
): value is T[number] {
  return (
    typeof value === "string" &&
    values.some((candidate) => candidate === value)
  );
}

function expectEnum<const T extends readonly string[]>(
  value: unknown,
  values: T,
  path: string,
): T[number] {
  if (!isEnumValue(value, values)) {
    return schemaError(path, `one of ${values.join(", ")}`);
  }
  return value;
}

function assignOptional<T extends object, K extends keyof T>(
  target: T,
  key: K,
  value: T[K] | undefined,
): void {
  if (value !== undefined) {
    target[key] = value;
  }
}

function expectId(value: unknown, path: string): string {
  return expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControlCharacters: true,
  });
}

function expectRunStatus(value: unknown, path: string): RunStatus {
  return expectEnum(
    value,
    ["init", "running", "done", "error", "cancelled", "interrupted"] as const,
    path,
  );
}

export function isSafeEnvironmentVariableName(value: string): boolean {
  return /^[A-Z_][A-Z0-9_]{0,255}$/.test(value);
}

export function assertSafeProductProviderConfiguration(
  providerType: ProductProviderType,
  apiBase: string,
  apiKeyEnv: string | undefined,
  path = "provider",
): void {
  if (
    apiKeyEnv !== undefined &&
    !isSafeEnvironmentVariableName(apiKeyEnv)
  ) {
    schemaError(`${path}.api_key_env`, "a valid environment variable name");
  }

  if (providerType === "fake") {
    if (apiKeyEnv !== undefined) {
      schemaError(`${path}.api_key_env`, "absent for the fake provider");
    }
    if (apiBase.trim() !== "") {
      schemaError(`${path}.api_base`, "empty for the fake provider");
    }
    return;
  }

  let parsed: URL;
  try {
    parsed = new URL(apiBase);
  } catch {
    return schemaError(`${path}.api_base`, "an absolute HTTP(S) URL");
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    schemaError(`${path}.api_base`, "an HTTP(S) URL");
  }
  if (parsed.username || parsed.password) {
    schemaError(`${path}.api_base`, "a URL without user information");
  }
  if (parsed.search || parsed.hash) {
    schemaError(`${path}.api_base`, "a URL without query parameters or fragments");
  }
}

function parseProductRuntimeBinding(
  value: unknown,
  path: string,
): ProductRuntimeBinding {
  const record = expectRecord(value, path);
  return {
    ordinal: expectInteger(record.ordinal, `${path}.ordinal`, { min: 1 }),
    runtime_session_id: expectId(
      record.runtime_session_id,
      `${path}.runtime_session_id`,
    ),
    latest_job_id: expectId(record.latest_job_id, `${path}.latest_job_id`),
    latest_run_id: expectId(record.latest_run_id, `${path}.latest_run_id`),
  };
}

export function parseProductWorkspace(
  value: unknown,
  path = "product workspace",
): ProductWorkspace {
  const record = expectRecord(value, path);
  return {
    id: expectId(record.id, `${path}.id`),
    canonical_root: expectString(record.canonical_root, `${path}.canonical_root`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
    }),
    kind: expectEnum(record.kind, PRODUCT_WORKSPACE_KINDS, `${path}.kind`),
    display_name: expectString(record.display_name, `${path}.display_name`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    pinned: expectBoolean(record.pinned, `${path}.pinned`),
    last_opened_at: expectString(
      record.last_opened_at,
      `${path}.last_opened_at`,
      { nonEmpty: true },
    ),
    created_at: expectString(record.created_at, `${path}.created_at`, {
      nonEmpty: true,
    }),
    updated_at: expectString(record.updated_at, `${path}.updated_at`, {
      nonEmpty: true,
    }),
  };
}

export function parseProductSession(
  value: unknown,
  path = "product session",
): ProductSession {
  const record = expectRecord(value, path);
  const session: ProductSession = {
    id: expectId(record.id, `${path}.id`),
    workspace_id: expectId(record.workspace_id, `${path}.workspace_id`),
    title: expectString(record.title, `${path}.title`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    status: expectEnum(
      record.status,
      PRODUCT_SESSION_STATUSES,
      `${path}.status`,
    ),
    created_at: expectString(record.created_at, `${path}.created_at`, {
      nonEmpty: true,
    }),
    updated_at: expectString(record.updated_at, `${path}.updated_at`, {
      nonEmpty: true,
    }),
  };
  if (record.runtime_binding !== undefined && record.runtime_binding !== null) {
    session.runtime_binding = parseProductRuntimeBinding(
      record.runtime_binding,
      `${path}.runtime_binding`,
    );
  }
  assignOptional(
    session,
    "parent_session_id",
    optionalId(record, "parent_session_id", path),
  );
  assignOptional(
    session,
    "fork_point_run_id",
    optionalId(record, "fork_point_run_id", path),
  );
  assignOptional(
    session,
    "fork_point_seq",
    optionalInteger(record, "fork_point_seq", path, { min: 1 }),
  );
  const hasForkParent = session.parent_session_id !== undefined;
  const hasForkRun = session.fork_point_run_id !== undefined;
  const hasForkSeq = session.fork_point_seq !== undefined;
  if (hasForkParent !== hasForkRun || hasForkParent !== hasForkSeq) {
    schemaError(
      path,
      "complete fork provenance fields or no fork provenance fields",
    );
  }
  const lastOutcome = record.last_outcome;
  if (lastOutcome !== undefined && lastOutcome !== null) {
    session.last_outcome = expectEnum(
      lastOutcome,
      PRODUCT_SESSION_OUTCOMES,
      `${path}.last_outcome`,
    );
  }
  assignOptional(
    session,
    "last_outcome_at",
    optionalString(record, "last_outcome_at", path, { nonEmpty: true }),
  );
  // The store writes the outcome and its timestamp in one statement, so a lone
  // field means the payload is not one this client understands. Fail closed
  // instead of guessing which half is authoritative.
  if ((session.last_outcome === undefined) !== (session.last_outcome_at === undefined)) {
    schemaError(path, "both last_outcome and last_outcome_at, or neither");
  }
  return session;
}

function parseProductExecutionCapabilities(
  value: unknown,
  path: string,
): ProductExecutionCapabilities {
  const record = expectRecord(value, path);
  return {
    filesystem_read: expectBoolean(
      record.filesystem_read,
      `${path}.filesystem_read`,
    ),
    filesystem_write: expectBoolean(
      record.filesystem_write,
      `${path}.filesystem_write`,
    ),
    process_run: expectBoolean(record.process_run, `${path}.process_run`),
    process_stdio: expectBoolean(record.process_stdio, `${path}.process_stdio`),
    observations: expectBoolean(record.observations, `${path}.observations`),
    process_background: expectBoolean(
      record.process_background,
      `${path}.process_background`,
    ),
    process_pty: expectBoolean(record.process_pty, `${path}.process_pty`),
    workspace_checkpoints: expectBoolean(
      record.workspace_checkpoints,
      `${path}.workspace_checkpoints`,
    ),
    artifact_projection: expectBoolean(
      record.artifact_projection,
      `${path}.artifact_projection`,
    ),
  };
}

export function parseProductRuntimeInfo(
  value: unknown,
  path = "product runtime info",
): ProductRuntimeInfo {
  const record = expectRecord(value, path);
  const environment = expectRecord(
    record.execution_environment,
    `${path}.execution_environment`,
  );
  const agent = expectRecord(record.agent, `${path}.agent`);
  const info: ProductRuntimeInfo = {
    api_version: expectString(record.api_version, `${path}.api_version`, {
      nonEmpty: true,
    }),
    connection: expectEnum(
      record.connection,
      PRODUCT_CONNECTION_STATUSES,
      `${path}.connection`,
    ),
    product_store: expectEnum(
      record.product_store,
      PRODUCT_STORE_STATUSES,
      `${path}.product_store`,
    ),
    execution_environment: {
      adapter: expectEnum(
        environment.adapter,
        PRODUCT_EXECUTION_ADAPTERS,
        `${path}.execution_environment.adapter`,
      ),
      workspace_kind: expectEnum(
        environment.workspace_kind,
        PRODUCT_EXECUTION_WORKSPACE_KINDS,
        `${path}.execution_environment.workspace_kind`,
      ),
      workspace_digest: expectString(
        environment.workspace_digest,
        `${path}.execution_environment.workspace_digest`,
      ),
      capabilities: parseProductExecutionCapabilities(
        environment.capabilities,
        `${path}.execution_environment.capabilities`,
      ),
    },
    agent: {
      selector: expectString(agent.selector, `${path}.agent.selector`),
      workspace_source_authorized: expectBoolean(
        agent.workspace_source_authorized,
        `${path}.agent.workspace_source_authorized`,
      ),
      workspace_instructions_enabled: expectBoolean(
        agent.workspace_instructions_enabled,
        `${path}.agent.workspace_instructions_enabled`,
      ),
      allow_remediation_procedures: expectBoolean(
        agent.allow_remediation_procedures,
        `${path}.agent.allow_remediation_procedures`,
      ),
      max_procedure_selections: expectInteger(
        agent.max_procedure_selections,
        `${path}.agent.max_procedure_selections`,
        { min: 0 },
      ),
    },
  };
  if (record.resume_health !== undefined && record.resume_health !== null) {
    const health = expectRecord(record.resume_health, `${path}.resume_health`);
    info.resume_health = {
      status: expectEnum(
        health.status,
        PRODUCT_RESUME_HEALTH_STATUSES,
        `${path}.resume_health.status`,
      ),
      workspace_count: expectInteger(
        health.workspace_count,
        `${path}.resume_health.workspace_count`,
        { min: 0 },
      ),
      session_count: expectInteger(
        health.session_count,
        `${path}.resume_health.session_count`,
        { min: 0 },
      ),
      bound_session_count: expectInteger(
        health.bound_session_count,
        `${path}.resume_health.bound_session_count`,
        { min: 0 },
      ),
      running_session_count: expectInteger(
        health.running_session_count,
        `${path}.resume_health.running_session_count`,
        { min: 0 },
      ),
      needs_attention_session_count: expectInteger(
        health.needs_attention_session_count,
        `${path}.resume_health.needs_attention_session_count`,
        { min: 0 },
      ),
    };
  }
  return info;
}

function parseProductReviewTargetSpec(
  value: unknown,
  path: string,
): ProductReviewTargetSpec {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["kind", "revision"], path);
  const kind = expectEnum(
    record.kind,
    PRODUCT_REVIEW_TARGET_KINDS,
    `${path}.kind`,
  );
  const revision = optionalString(record, "revision", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControlCharacters: true,
  });
  if (kind === "uncommitted" && revision !== undefined) {
    schemaError(`${path}.revision`, "omitted for an uncommitted target");
  }
  if ((kind === "base" || kind === "commit") && revision === undefined) {
    schemaError(`${path}.revision`, "present for a base or commit target");
  }
  return revision === undefined ? { kind } : { kind, revision };
}

function parseProductReviewTargetSummary(
  value: unknown,
  path: string,
): ProductReviewTargetSummary {
  const record = expectRecord(value, path);
  const summary: ProductReviewTargetSummary = {
    schema_version: expectInteger(record.schema_version, `${path}.schema_version`, {
      min: 1,
    }),
    spec: parseProductReviewTargetSpec(record.spec, `${path}.spec`),
    workspace_kind: expectEnum(
      record.workspace_kind,
      PRODUCT_WORKSPACE_KINDS,
      `${path}.workspace_kind`,
    ),
    workspace_digest: expectString(record.workspace_digest, `${path}.workspace_digest`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    captured_at: expectRfc3339Timestamp(record.captured_at, `${path}.captured_at`),
    entries: expectInteger(record.entries, `${path}.entries`, { min: 0 }),
    entries_truncated: expectInteger(
      record.entries_truncated ?? 0,
      `${path}.entries_truncated`,
      { min: 0 },
    ),
    digest: expectString(record.digest, `${path}.digest`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  };
  assignOptional(
    summary,
    "resolved_base",
    optionalString(record, "resolved_base", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  return summary;
}

function parseProductReviewFinding(
  value: unknown,
  path: string,
): ProductReviewFinding {
  const record = expectRecord(value, path);
  const locationRecord = expectRecord(record.location, `${path}.location`);
  const evidence = expectArray(
    record.evidence ?? [],
    `${path}.evidence`,
    (item, itemPath) => {
      const evidenceRecord = expectRecord(item, itemPath);
      const parsed: ProductReviewEvidence = {
        snippet: expectString(evidenceRecord.snippet, `${itemPath}.snippet`, {
          maxBytes: 2 * 1024,
        }),
        source: expectString(evidenceRecord.source, `${itemPath}.source`, {
          maxBytes: 64,
        }),
      };
      assignOptional(
        parsed,
        "reference",
        optionalString(evidenceRecord, "reference", itemPath, {
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
        }),
      );
      return parsed;
    },
    3,
  );
  return {
    finding_id: expectString(record.finding_id, `${path}.finding_id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    severity: expectEnum(
      record.severity,
      PRODUCT_REVIEW_SEVERITIES,
      `${path}.severity`,
    ),
    confidence: expectEnum(
      record.confidence,
      PRODUCT_REVIEW_CONFIDENCES,
      `${path}.confidence`,
    ),
    category: expectString(record.category, `${path}.category`, {
      maxBytes: 128,
    }),
    path: expectString(record.path, `${path}.path`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
    }),
    location: {
      start_line: expectInteger(locationRecord.start_line, `${path}.location.start_line`, {
        min: 0,
      }),
      start_col: expectInteger(locationRecord.start_col, `${path}.location.start_col`, {
        min: 0,
      }),
      end_line: expectInteger(locationRecord.end_line, `${path}.location.end_line`, {
        min: 0,
      }),
      end_col: expectInteger(locationRecord.end_col, `${path}.location.end_col`, {
        min: 0,
      }),
    },
    location_status: expectEnum(
      record.location_status,
      PRODUCT_REVIEW_LOCATION_STATUSES,
      `${path}.location_status`,
    ),
    title: expectString(record.title, `${path}.title`, {
      nonEmpty: true,
      maxBytes: 256,
    }),
    explanation: expectString(record.explanation, `${path}.explanation`, {
      maxBytes: 4 * 1024,
    }),
    evidence,
    rule: expectString(record.rule, `${path}.rule`, { maxBytes: 256 }),
    suggestion: expectString(record.suggestion, `${path}.suggestion`, {
      maxBytes: 2 * 1024,
    }),
    status: expectString(record.status, `${path}.status`, { nonEmpty: true, maxBytes: 64 }),
  };
}

function parseProductReviewResult(
  value: unknown,
  path = "product review result",
): ProductReviewResult {
  const record = expectRecord(value, path);
  const stats = expectRecord(record.stats, `${path}.stats`);
  const unchecked = expectArray(
    record.unchecked ?? [],
    `${path}.unchecked`,
    (item, itemPath) => {
      const itemRecord = expectRecord(item, itemPath);
      return {
        reason: expectString(itemRecord.reason, `${itemPath}.reason`, {
          nonEmpty: true,
          maxBytes: 128,
        }),
        paths: expectArray(
          itemRecord.paths ?? [],
          `${itemPath}.paths`,
          (pathValue, pathPath) =>
            expectString(pathValue, pathPath, {
              maxBytes: MAX_PRODUCT_PATH_BYTES,
            }),
        ),
      } satisfies ProductReviewUnchecked;
    },
    512,
  );
  return {
    schema_version: expectInteger(record.schema_version, `${path}.schema_version`, {
      min: 1,
    }),
    review_id: expectString(record.review_id, `${path}.review_id`, { nonEmpty: true }),
    run_id: expectString(record.run_id, `${path}.run_id`, { nonEmpty: true }),
    session_id: expectString(record.session_id, `${path}.session_id`, { nonEmpty: true }),
    target: parseProductReviewTargetSummary(record.target, `${path}.target`),
    conclusion: expectEnum(
      record.conclusion,
      PRODUCT_REVIEW_CONCLUSIONS,
      `${path}.conclusion`,
    ),
    findings: expectArray(
      record.findings ?? [],
      `${path}.findings`,
      parseProductReviewFinding,
      512,
    ),
    stats: {
      files_scanned: expectInteger(stats.files_scanned, `${path}.stats.files_scanned`, {
        min: 0,
      }),
      bytes_scanned: expectInteger(stats.bytes_scanned, `${path}.stats.bytes_scanned`, {
        min: 0,
      }),
      duration_ms: expectInteger(stats.duration_ms, `${path}.stats.duration_ms`, {
        min: 0,
      }),
      concurrency_limit: expectInteger(
        stats.concurrency_limit,
        `${path}.stats.concurrency_limit`,
        { min: 1 },
      ),
      findings_total: expectInteger(stats.findings_total, `${path}.stats.findings_total`, {
        min: 0,
      }),
      truncated_findings: expectInteger(
        stats.truncated_findings,
        `${path}.stats.truncated_findings`,
        { min: 0 },
      ),
    },
    unchecked,
    warnings: expectArray(
      record.warnings ?? [],
      `${path}.warnings`,
      (item, itemPath) => expectString(item, itemPath, { maxBytes: 256 }),
      256,
    ),
  };
}

export function parseProductReview(
  value: unknown,
  path = "product review",
): ProductReview {
  const record = expectRecord(value, path);
  const review: ProductReview = {
    id: expectId(record.id, `${path}.id`),
    product_session_id: expectId(record.product_session_id, `${path}.product_session_id`),
    workspace_id: expectId(record.workspace_id, `${path}.workspace_id`),
    target: parseProductReviewTargetSummary(record.target, `${path}.target`),
    status: expectEnum(record.status, PRODUCT_REVIEW_STATUSES, `${path}.status`),
    findings_count: expectInteger(record.findings_count, `${path}.findings_count`, { min: 0 }),
    unchecked_count: expectInteger(record.unchecked_count, `${path}.unchecked_count`, { min: 0 }),
    warnings_count: expectInteger(record.warnings_count, `${path}.warnings_count`, { min: 0 }),
    created_at: expectRfc3339Timestamp(record.created_at, `${path}.created_at`),
    updated_at: expectRfc3339Timestamp(record.updated_at, `${path}.updated_at`),
    captured_at: expectRfc3339Timestamp(record.captured_at, `${path}.captured_at`),
  };
  const conclusion = record.conclusion;
  if (conclusion !== undefined && conclusion !== null) {
    review.conclusion = expectEnum(
      conclusion,
      PRODUCT_REVIEW_CONCLUSIONS,
      `${path}.conclusion`,
    );
  }
  assignOptional(review, "runtime_session_id", optionalString(record, "runtime_session_id", path, { nonEmpty: true }));
  assignOptional(review, "job_id", optionalString(record, "job_id", path, { nonEmpty: true }));
  assignOptional(review, "run_id", optionalString(record, "run_id", path, { nonEmpty: true }));
  if (record.result !== undefined && record.result !== null) {
    review.result = parseProductReviewResult(record.result, `${path}.result`);
  }
  const finalizedAt = optionalString(record, "finalized_at", path, { nonEmpty: true });
  if (finalizedAt !== undefined) {
    review.finalized_at = expectRfc3339Timestamp(finalizedAt, `${path}.finalized_at`);
  }
  return review;
}

export function parseCreateProductReviewRequest(
  value: CreateProductReviewRequest,
): CreateProductReviewRequest {
  const record = expectRecord(value, "create product review request");
  expectOnlyKeys(record, ["target", "idempotency_key", "max_steps"], "create product review request");
  const request: CreateProductReviewRequest = {
    target: parseProductReviewTargetSpec(record.target, "create product review request.target"),
  };
  const key = optionalString(record, "idempotency_key", "create product review request", {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES,
    noControlCharacters: true,
  });
  if (key !== undefined) {
    if (!/^[A-Za-z0-9_.:-]+$/u.test(key)) {
      schemaError("create product review request.idempotency_key", "a valid idempotency key");
    }
    request.idempotency_key = key;
  }
  const maxSteps = record.max_steps;
  if (maxSteps !== undefined && maxSteps !== null) {
    request.max_steps = expectInteger(maxSteps, "create product review request.max_steps", {
      min: 1,
      max: MAX_PRODUCT_MAX_STEPS,
    });
  }
  return request;
}

export function parseProductReviewsResponse(value: unknown): ProductReviewsResponse {
  const record = expectRecord(value, "product reviews response");
  return {
    reviews: expectArray(
      record.reviews,
      "product reviews response.reviews",
      parseProductReview,
      MAX_PRODUCT_SESSIONS,
    ),
  };
}

export function parseProductReviewFindingsResponse(
  value: unknown,
): ProductReviewFindingsResponse {
  const record = expectRecord(value, "product review findings response");
  const response: ProductReviewFindingsResponse = {
    findings: expectArray(
      record.findings,
      "product review findings response.findings",
      (item, path) => {
        const itemRecord = expectRecord(item, path);
        return {
          finding: parseProductReviewFinding(itemRecord.finding, `${path}.finding`),
          sort_key: expectString(itemRecord.sort_key, `${path}.sort_key`, {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
          }),
        };
      },
      512,
    ),
  };
  assignOptional(
    response,
    "next_cursor",
    optionalInteger(record, "next_cursor", "product review findings response", {
      min: 0,
    }),
  );
  return response;
}

export function parseProductFork(
  value: unknown,
  path = "product fork",
): ProductFork {
  const record = expectRecord(value, path);
  const fork: ProductFork = {
    id: expectId(record.id, `${path}.id`),
    parent_product_session_id: expectId(
      record.parent_product_session_id,
      `${path}.parent_product_session_id`,
    ),
    child_product_session_id: expectId(
      record.child_product_session_id,
      `${path}.child_product_session_id`,
    ),
    parent_workspace_id: expectId(
      record.parent_workspace_id,
      `${path}.parent_workspace_id`,
    ),
    parent_title: expectString(record.parent_title, `${path}.parent_title`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    source_runtime_session_id: expectId(
      record.source_runtime_session_id,
      `${path}.source_runtime_session_id`,
    ),
    source_runtime_job_id: expectId(
      record.source_runtime_job_id,
      `${path}.source_runtime_job_id`,
    ),
    source_runtime_run_id: expectId(
      record.source_runtime_run_id,
      `${path}.source_runtime_run_id`,
    ),
    fork_at_event_seq: expectInteger(record.fork_at_event_seq, `${path}.fork_at_event_seq`, {
      min: 1,
    }),
    idempotency_key: expectString(record.idempotency_key, `${path}.idempotency_key`, {
      nonEmpty: true,
      maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES,
      noControlCharacters: true,
    }),
    created_at: expectString(record.created_at, `${path}.created_at`, {
      nonEmpty: true,
    }),
  };
  assignOptional(
    fork,
    "truncate_after_message_seq",
    optionalInteger(record, "truncate_after_message_seq", path, { min: 1 }),
  );
  assignOptional(
    fork,
    "truncate_after_message_id",
    optionalString(record, "truncate_after_message_id", path, { nonEmpty: true }),
  );
  return fork;
}

function parseProductSessionRunBinding(
  value: unknown,
  path: string,
): ProductSessionRunBinding {
  const record = expectRecord(value, path);
  const binding: ProductSessionRunBinding = {
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    ordinal: expectInteger(record.ordinal, `${path}.ordinal`, { min: 1 }),
    runtime_session_id: expectId(
      record.runtime_session_id,
      `${path}.runtime_session_id`,
    ),
    runtime_job_id: expectId(record.runtime_job_id, `${path}.runtime_job_id`),
    runtime_run_id: expectId(record.runtime_run_id, `${path}.runtime_run_id`),
    bound_at: expectString(record.bound_at, `${path}.bound_at`, {
      nonEmpty: true,
    }),
  };
  assignOptional(
    binding,
    "resumed_from_run_id",
    optionalString(record, "resumed_from_run_id", path, { nonEmpty: true }),
  );
  return binding;
}

export function parseProductProviderProfile(
  value: unknown,
  path = "product provider profile",
): ProductProviderProfile {
  const record = expectRecord(value, path);
  const providerType = expectEnum(
    record.provider_type,
    PRODUCT_PROVIDER_TYPES,
    `${path}.provider_type`,
  );
  const apiBase = expectString(record.api_base, `${path}.api_base`, {
    maxBytes: MAX_PRODUCT_API_BASE_BYTES,
  });
  const apiKeyEnv = optionalString(record, "api_key_env", path, {
    nonEmpty: true,
    maxBytes: 256,
  });
  const credentialSource =
    record.credential_source === undefined
      ? apiKeyEnv
        ? ({ source: "env", name: apiKeyEnv } as const)
        : ({ source: "none" } as const)
      : parseProductProviderCredentialSource(
          record.credential_source,
          `${path}.credential_source`,
        );
  assertSafeProductProviderConfiguration(
    providerType,
    apiBase,
    apiKeyEnv,
    path,
  );
  const profile: ProductProviderProfile = {
    id: expectId(record.id, `${path}.id`),
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    provider_type: providerType,
    api_base: apiBase,
    credential_source: credentialSource,
    created_at: expectString(record.created_at, `${path}.created_at`, {
      nonEmpty: true,
    }),
    updated_at: expectString(record.updated_at, `${path}.updated_at`, {
      nonEmpty: true,
    }),
    catalog_revision: expectString(
      record.catalog_revision,
      `${path}.catalog_revision`,
      { nonEmpty: true },
    ),
  };
  assignOptional(profile, "api_key_env", apiKeyEnv);
  assignOptional(
    profile,
    "default_model",
    optionalString(record, "default_model", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  return profile;
}

function parseProductProviderCredentialSource(
  value: unknown,
  path: string,
): ProductProviderCredentialSource {
  const record = expectRecord(value, path);
  const source = expectEnum(
    record.source,
    ["env", "file", "keyring", "none"] as const,
    `${path}.source`,
  );
  switch (source) {
    case "env":
      return {
        source,
        name: expectString(record.name, `${path}.name`, {
          nonEmpty: true,
          maxBytes: 256,
        }),
      };
    case "file":
      return {
        source,
        path: expectString(record.path, `${path}.path`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_PATH_BYTES,
        }),
      };
    case "keyring":
      return {
        source,
        service: expectString(record.service, `${path}.service`, {
          nonEmpty: true,
          maxBytes: 256,
        }),
        account: expectString(record.account, `${path}.account`, {
          nonEmpty: true,
          maxBytes: 256,
        }),
      };
    case "none":
      return { source };
  }
}

function parseProductSessionModelConfig(
  value: unknown,
  path = "product session model config",
): ProductSessionModelConfig {
  const record = expectRecord(value, path);
  const config: ProductSessionModelConfig = {
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    reasoning: expectEnum(
      record.reasoning,
      PRODUCT_REASONING_PREFERENCES,
      `${path}.reasoning`,
    ),
    max_steps: expectInteger(record.max_steps, `${path}.max_steps`, {
      min: 1,
      max: MAX_PRODUCT_MAX_STEPS,
    }),
    revision: expectInteger(record.revision, `${path}.revision`, { min: 0 }),
    updated_at: expectString(record.updated_at, `${path}.updated_at`, {
      nonEmpty: true,
    }),
  };
  assignOptional(
    config,
    "profile_id",
    optionalString(record, "profile_id", path, { nonEmpty: true }),
  );
  return config;
}

export function parseProductSessionModelConfigResponse(
  value: unknown,
): ProductSessionModelConfig {
  return parseProductSessionModelConfig(value);
}

export function parseUpdateProductSessionModelConfigRequest(
  value: unknown,
  path = "update product session model config request",
): UpdateProductSessionModelConfigRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["profile_id", "model", "reasoning", "max_steps", "expected_revision"],
    path,
  );
  const request: UpdateProductSessionModelConfigRequest = {
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    reasoning:
      record.reasoning === undefined || record.reasoning === null
        ? "default"
        : expectEnum(
            record.reasoning,
            PRODUCT_REASONING_PREFERENCES,
            `${path}.reasoning`,
          ),
    max_steps:
      record.max_steps === undefined || record.max_steps === null
        ? 8
        : expectInteger(record.max_steps, `${path}.max_steps`, {
            min: 1,
            max: MAX_PRODUCT_MAX_STEPS,
          }),
  };
  assignOptional(
    request,
    "profile_id",
    optionalString(record, "profile_id", path, { nonEmpty: true }),
  );
  assignOptional(
    request,
    "expected_revision",
    optionalInteger(record, "expected_revision", path, { min: 0 }),
  );
  return request;
}

function parseProductModelDescriptor(
  value: unknown,
  path: string,
): ProductModelDescriptor {
  const record = expectRecord(value, path);
  const descriptor: ProductModelDescriptor = {
    id: expectString(record.id, `${path}.id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    supports_reasoning: expectBoolean(
      record.supports_reasoning,
      `${path}.supports_reasoning`,
    ),
    supported_reasoning: expectArray(
      record.supported_reasoning,
      `${path}.supported_reasoning`,
      (item, itemPath) =>
        expectEnum(item, PRODUCT_REASONING_PREFERENCES, itemPath),
      PRODUCT_REASONING_PREFERENCES.length,
    ),
  };
  assignOptional(
    descriptor,
    "context_window",
    optionalInteger(record, "context_window", path, { min: 1 }),
  );
  assignOptional(
    descriptor,
    "reasoning_unavailable_reason",
    optionalString(record, "reasoning_unavailable_reason", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  return descriptor;
}

export function parseProductProviderModelsResponse(
  value: unknown,
): ProductProviderModelsResponse {
  const record = expectRecord(value, "product provider models response");
  const response: ProductProviderModelsResponse = {
    profile_id: expectId(
      record.profile_id,
      "product provider models response.profile_id",
    ),
    models: expectArray(
      record.models,
      "product provider models response.models",
      parseProductModelDescriptor,
      4_096,
    ),
  };
  assignOptional(
    response,
    "default_model",
    optionalString(record, "default_model", "product provider models response", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  return response;
}

function parseProductSessionRunModelView(
  value: unknown,
  path: string,
): ProductSessionRunModelView {
  const record = expectRecord(value, path);
  const run: ProductSessionRunModelView = {
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    ordinal: expectInteger(record.ordinal, `${path}.ordinal`, { min: 1 }),
    runtime_run_id: expectId(record.runtime_run_id, `${path}.runtime_run_id`),
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    reasoning: expectEnum(
      record.reasoning,
      PRODUCT_REASONING_PREFERENCES,
      `${path}.reasoning`,
    ),
    max_steps: expectInteger(record.max_steps, `${path}.max_steps`, {
      min: 1,
      max: MAX_PRODUCT_MAX_STEPS,
    }),
  };
  assignOptional(
    run,
    "profile_id",
    optionalString(record, "profile_id", path, { nonEmpty: true }),
  );
  assignOptional(
    run,
    "context_window",
    optionalInteger(record, "context_window", path, { min: 1 }),
  );
  assignOptional(
    run,
    "pricing_source",
    optionalString(record, "pricing_source", path, { nonEmpty: true }),
  );
  assignOptional(
    run,
    "pricing_version",
    optionalString(record, "pricing_version", path, { nonEmpty: true }),
  );
  assignOptional(
    run,
    "pricing_currency",
    optionalString(record, "pricing_currency", path, { nonEmpty: true }),
  );
  if (
    record.pricing_availability !== undefined &&
    record.pricing_availability !== null
  ) {
    run.pricing_availability = expectEnum(
      record.pricing_availability,
      PRODUCT_PRICING_AVAILABILITIES,
      `${path}.pricing_availability`,
    );
  }
  assignOptional(
    run,
    "per_mtok_prompt",
    optionalNumber(record, "per_mtok_prompt", path),
  );
  assignOptional(
    run,
    "per_mtok_completion",
    optionalNumber(record, "per_mtok_completion", path),
  );
  assignOptional(
    run,
    "per_mtok_cache_read",
    optionalNumber(record, "per_mtok_cache_read", path),
  );
  return run;
}

export function parseProductSessionRunModelsResponse(
  value: unknown,
): ProductSessionRunModelsResponse {
  const record = expectRecord(value, "product session run models response");
  return {
    runs: expectArray(
      record.runs,
      "product session run models response.runs",
      parseProductSessionRunModelView,
      MAX_PRODUCT_SESSIONS,
    ),
  };
}

function parseProductUsage(value: unknown, path: string): ProductUsage {
  const record = expectRecord(value, path);
  return {
    prompt_tokens: expectInteger(record.prompt_tokens, `${path}.prompt_tokens`, {
      min: 0,
    }),
    completion_tokens: expectInteger(
      record.completion_tokens,
      `${path}.completion_tokens`,
      { min: 0 },
    ),
    total_tokens: expectInteger(record.total_tokens, `${path}.total_tokens`, {
      min: 0,
    }),
    cached_tokens: expectInteger(record.cached_tokens, `${path}.cached_tokens`, {
      min: 0,
    }),
  };
}

function parseProductCostBreakdown(
  value: unknown,
  path: string,
): ProductCostBreakdown {
  const record = expectRecord(value, path);
  const cost: ProductCostBreakdown = {
    currency: expectString(record.currency, `${path}.currency`, {
      nonEmpty: true,
      maxBytes: 16,
    }),
    availability: expectEnum(
      record.availability,
      PRODUCT_PRICING_AVAILABILITIES,
      `${path}.availability`,
    ),
  };
  assignOptional(cost, "total_usd", optionalNumber(record, "total_usd", path));
  assignOptional(cost, "prompt_usd", optionalNumber(record, "prompt_usd", path));
  assignOptional(
    cost,
    "completion_usd",
    optionalNumber(record, "completion_usd", path),
  );
  assignOptional(
    cost,
    "cache_read_usd",
    optionalNumber(record, "cache_read_usd", path),
  );
  assignOptional(
    cost,
    "pricing_source",
    optionalString(record, "pricing_source", path, { nonEmpty: true }),
  );
  assignOptional(
    cost,
    "pricing_version",
    optionalString(record, "pricing_version", path, { nonEmpty: true }),
  );
  return cost;
}

function parseProductContextOccupancy(
  value: unknown,
  path: string,
): ProductContextOccupancy {
  const record = expectRecord(value, path);
  const context: ProductContextOccupancy = {
    token_estimate: expectInteger(
      record.token_estimate,
      `${path}.token_estimate`,
      { min: 0 },
    ),
    estimate_kind: expectString(record.estimate_kind, `${path}.estimate_kind`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    included_history_messages: expectInteger(
      record.included_history_messages,
      `${path}.included_history_messages`,
      { min: 0 },
    ),
    dropped_history_messages: expectInteger(
      record.dropped_history_messages,
      `${path}.dropped_history_messages`,
      { min: 0 },
    ),
    compaction_degraded: expectBoolean(
      record.compaction_degraded ?? false,
      `${path}.compaction_degraded`,
    ),
    compaction_auto_triggered: expectBoolean(
      record.compaction_auto_triggered ?? false,
      `${path}.compaction_auto_triggered`,
    ),
    compacted_history_messages: expectInteger(
      record.compacted_history_messages ?? 0,
      `${path}.compacted_history_messages`,
      { min: 0 },
    ),
    compaction_source_messages: expectInteger(
      record.compaction_source_messages ?? 0,
      `${path}.compaction_source_messages`,
      { min: 0 },
    ),
  };
  assignOptional(
    context,
    "context_window",
    optionalInteger(record, "context_window", path, { min: 1 }),
  );
  assignOptional(
    context,
    "compaction_mode",
    optionalString(record, "compaction_mode", path, { nonEmpty: true }),
  );
  assignOptional(
    context,
    "compaction_prompt_version",
    optionalString(record, "compaction_prompt_version", path, {
      nonEmpty: true,
    }),
  );
  assignOptional(
    context,
    "prompt_hash",
    optionalString(record, "prompt_hash", path, { nonEmpty: true }),
  );
  return context;
}

function parseProductRunUsage(value: unknown, path: string): ProductRunUsage {
  const record = expectRecord(value, path);
  const run: ProductRunUsage = {
    runtime_run_id: expectId(record.runtime_run_id, `${path}.runtime_run_id`),
    ordinal: expectInteger(record.ordinal, `${path}.ordinal`, { min: 1 }),
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    usage: parseProductUsage(record.usage, `${path}.usage`),
    steps: expectInteger(record.steps, `${path}.steps`, { min: 0 }),
    tool_calls: expectInteger(record.tool_calls, `${path}.tool_calls`, {
      min: 0,
    }),
  };
  if (record.cost !== undefined && record.cost !== null) {
    run.cost = parseProductCostBreakdown(record.cost, `${path}.cost`);
  }
  if (record.context !== undefined && record.context !== null) {
    run.context = parseProductContextOccupancy(
      record.context,
      `${path}.context`,
    );
  }
  return run;
}

export function parseProductSessionUsageResponse(
  value: unknown,
): ProductSessionUsageResponse {
  const record = expectRecord(value, "product session usage response");
  const response: ProductSessionUsageResponse = {
    product_session_id: expectId(
      record.product_session_id,
      "product session usage response.product_session_id",
    ),
    totals: parseProductUsage(
      record.totals,
      "product session usage response.totals",
    ),
    runs: expectArray(
      record.runs,
      "product session usage response.runs",
      parseProductRunUsage,
      MAX_PRODUCT_SESSIONS,
    ),
    partial_reasons: expectArray(
      record.partial_reasons,
      "product session usage response.partial_reasons",
      (item, path) =>
        expectString(item, path, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
        }),
      512,
    ),
  };
  if (record.totals_cost !== undefined && record.totals_cost !== null) {
    response.totals_cost = parseProductCostBreakdown(
      record.totals_cost,
      "product session usage response.totals_cost",
    );
  }
  if (record.latest_context !== undefined && record.latest_context !== null) {
    response.latest_context = parseProductContextOccupancy(
      record.latest_context,
      "product session usage response.latest_context",
    );
  }
  return response;
}

function parseProductProviderSelection(
  value: unknown,
  path: string,
  strict = false,
): ProductProviderSelection {
  const record = expectRecord(value, path);
  if (strict) {
    expectOnlyKeys(record, ["profile_id", "model", "approval", "max_steps"], path);
  }
  const selection: ProductProviderSelection = {
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    approval: expectEnum(
      record.approval,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.approval`,
    ),
    max_steps: expectInteger(record.max_steps, `${path}.max_steps`, {
      min: 1,
      max: 4_096,
    }),
  };
  assignOptional(
    selection,
    "profile_id",
    optionalString(record, "profile_id", path, { nonEmpty: true }),
  );
  return selection;
}

export function parseProductPreferences(
  value: unknown,
  path = "product preferences",
): ProductPreferences {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "schema_version",
      "revision",
      "theme",
      "default_approval_policy",
      "active_workspace_id",
      "active_session_id",
      "provider_selection",
    ],
    path,
  );
  const preferences: ProductPreferences = {
    schema_version: expectInteger(
      record.schema_version,
      `${path}.schema_version`,
      { min: 1 },
    ),
    revision: expectInteger(record.revision, `${path}.revision`, { min: 0 }),
    theme: expectEnum(
      record.theme,
      PRODUCT_THEME_PREFERENCES,
      `${path}.theme`,
    ),
    default_approval_policy: expectEnum(
      record.default_approval_policy,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.default_approval_policy`,
    ),
  };
  assignOptional(
    preferences,
    "active_workspace_id",
    optionalString(record, "active_workspace_id", path, { nonEmpty: true }),
  );
  assignOptional(
    preferences,
    "active_session_id",
    optionalString(record, "active_session_id", path, { nonEmpty: true }),
  );
  if (
    record.provider_selection !== undefined &&
    record.provider_selection !== null
  ) {
    preferences.provider_selection = parseProductProviderSelection(
      record.provider_selection,
      `${path}.provider_selection`,
      true,
    );
  }
  return preferences;
}

export function parseCreateProductWorkspaceRequest(
  value: unknown,
  path = "create product workspace request",
): CreateProductWorkspaceRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["root", "kind", "display_name", "pinned"], path);
  const request: CreateProductWorkspaceRequest = {
    root: expectString(record.root, `${path}.root`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
      noControlCharacters: true,
    }),
    kind: expectEnum(record.kind, PRODUCT_WORKSPACE_KINDS, `${path}.kind`),
  };
  assignOptional(
    request,
    "display_name",
    optionalString(record, "display_name", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  assignOptional(request, "pinned", optionalBoolean(record, "pinned", path));
  return request;
}

export function parseCreateProductSessionRequest(
  value: unknown,
  path = "create product session request",
): CreateProductSessionRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["workspace_id", "title"], path);
  const request: CreateProductSessionRequest = {
    workspace_id: expectId(record.workspace_id, `${path}.workspace_id`),
  };
  assignOptional(
    request,
    "title",
    optionalString(record, "title", path, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  return request;
}

export function parseCreateProductForkRequest(
  value: unknown,
  path = "create product fork request",
): CreateProductForkRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["fork_at_run_id", "title", "idempotency_key", "truncate_after_message_seq"],
    path,
  );
  const request: CreateProductForkRequest = {
    fork_at_run_id: expectId(record.fork_at_run_id, `${path}.fork_at_run_id`),
    idempotency_key: expectString(record.idempotency_key, `${path}.idempotency_key`, {
      nonEmpty: true,
      maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES,
      noControlCharacters: true,
    }),
  };
  assignOptional(
    request,
    "title",
    optionalString(record, "title", path, { maxBytes: MAX_PRODUCT_TEXT_BYTES }),
  );
  assignOptional(
    request,
    "truncate_after_message_seq",
    optionalInteger(record, "truncate_after_message_seq", path, { min: 1 }),
  );
  return request;
}

export function parseUpdateProductSessionRequest(
  value: unknown,
  path = "update product session request",
): UpdateProductSessionRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["title", "archived"], path);
  const request: UpdateProductSessionRequest = {};
  assignOptional(
    request,
    "title",
    optionalString(record, "title", path, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  assignOptional(
    request,
    "archived",
    optionalBoolean(record, "archived", path),
  );
  return request;
}

export function parseCreateProductControlRequest(
  value: unknown,
  path = "create product control request",
): CreateProductControlRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["content", "idempotency_key", "attachments"], path);
  const content = expectString(record.content, `${path}.content`, {
    maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
  }).trim();
  if (!content) {
    return schemaError(`${path}.content`, "a non-empty string");
  }
  const request: CreateProductControlRequest = { content };
  assignOptional(
    request,
    "idempotency_key",
    optionalString(record, "idempotency_key", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES,
    }),
  );
  const attachments = optionalArray(
    record,
    "attachments",
    path,
    parseProductMessageAttachmentRequest,
    MAX_MESSAGE_ATTACHMENTS,
  );
  if (attachments !== undefined && attachments.length > 0) {
    request.attachments = attachments;
  }
  return request;
}

export function parseProductProviderProfileRequest(
  value: unknown,
  path = "product provider profile request",
): CreateProductProviderProfileRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "label",
      "provider_type",
      "api_base",
      "api_key_env",
      "default_model",
      "expected_revision",
    ],
    path,
  );
  const providerType = expectEnum(
    record.provider_type,
    PRODUCT_PROVIDER_TYPES,
    `${path}.provider_type`,
  );
  const apiBase = expectString(record.api_base, `${path}.api_base`, {
    maxBytes: MAX_PRODUCT_API_BASE_BYTES,
  });
  const apiKeyEnv = optionalString(record, "api_key_env", path, {
    nonEmpty: true,
    maxBytes: 256,
  });
  assertSafeProductProviderConfiguration(
    providerType,
    apiBase,
    apiKeyEnv,
    path,
  );
  const request: CreateProductProviderProfileRequest = {
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    provider_type: providerType,
    api_base: apiBase,
  };
  assignOptional(request, "api_key_env", apiKeyEnv);
  assignOptional(
    request,
    "default_model",
    optionalString(record, "default_model", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  assignOptional(
    request,
    "expected_revision",
    optionalString(record, "expected_revision", path, { nonEmpty: true }),
  );
  return request;
}

/**
 * Validate the onboarding body before it is serialized — including the
 * credential's bound — but never inspect, transform, or retain its content.
 */
export function parseOnboardProductProviderRequest(
  value: unknown,
  path = "provider onboarding request",
): OnboardProductProviderRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "profile_id",
      "label",
      "provider_type",
      "api_base",
      "model",
      "make_default",
      "expected_revision",
      "credential",
    ],
    path,
  );
  const request: OnboardProductProviderRequest = {
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    provider_type: expectEnum(
      record.provider_type,
      PRODUCT_PROVIDER_TYPES,
      `${path}.provider_type`,
    ),
    api_base: expectString(record.api_base, `${path}.api_base`, {
      maxBytes: MAX_PRODUCT_API_BASE_BYTES,
    }),
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES * 2,
    }),
    credential: expectString(record.credential, `${path}.credential`, {
      nonEmpty: true,
      maxBytes: 16 * 1024,
    }),
  };
  assignOptional(
    request,
    "profile_id",
    optionalString(record, "profile_id", path, { nonEmpty: true }),
  );
  assignOptional(
    request,
    "make_default",
    optionalBoolean(record, "make_default", path),
  );
  assignOptional(
    request,
    "expected_revision",
    optionalString(record, "expected_revision", path, { nonEmpty: true }),
  );
  return request;
}

export function parseProductProviderOnboardingReceipt(
  value: unknown,
  path = "provider onboarding receipt",
): ProductProviderOnboardingReceipt {
  const record = expectRecord(value, path);
  const probe = expectRecord(record.probe, `${path}.probe`);
  return {
    profile_id: expectId(record.profile_id, `${path}.profile_id`),
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    provider_type: expectEnum(
      record.provider_type,
      PRODUCT_PROVIDER_TYPES,
      `${path}.provider_type`,
    ),
    api_base: expectString(record.api_base, `${path}.api_base`, {
      maxBytes: MAX_PRODUCT_API_BASE_BYTES,
    }),
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES * 2,
    }),
    catalog_revision: expectString(
      record.catalog_revision,
      `${path}.catalog_revision`,
      { nonEmpty: true },
    ),
    credential_source: expectString(
      record.credential_source,
      `${path}.credential_source`,
      { nonEmpty: true, maxBytes: 64 },
    ),
    probe: {
      inventory_count: expectInteger(
        probe.inventory_count,
        `${path}.probe.inventory_count`,
        { min: 0 },
      ),
      streaming_supported: expectBoolean(
        probe.streaming_supported,
        `${path}.probe.streaming_supported`,
      ),
      native_tool_calls_supported: expectBoolean(
        probe.native_tool_calls_supported,
        `${path}.probe.native_tool_calls_supported`,
      ),
      usage_supported: expectBoolean(
        probe.usage_supported,
        `${path}.probe.usage_supported`,
      ),
    },
    selected: expectBoolean(record.selected, `${path}.selected`),
  };
}

export function parseUpdateProductPreferencesRequest(
  value: unknown,
  path = "update product preferences request",
): UpdateProductPreferencesRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "schema_version",
      "expected_revision",
      "theme",
      "default_approval_policy",
      "active_workspace_id",
      "active_session_id",
      "provider_selection",
    ],
    path,
  );
  const request: UpdateProductPreferencesRequest = {
    schema_version: expectInteger(
      record.schema_version,
      `${path}.schema_version`,
      { min: 1 },
    ),
    theme: expectEnum(
      record.theme,
      PRODUCT_THEME_PREFERENCES,
      `${path}.theme`,
    ),
  };
  assignOptional(
    request,
    "expected_revision",
    optionalInteger(record, "expected_revision", path, { min: 0 }),
  );
  if (
    record.default_approval_policy !== undefined &&
    record.default_approval_policy !== null
  ) {
    request.default_approval_policy = expectEnum(
      record.default_approval_policy,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.default_approval_policy`,
    );
  }
  assignOptional(
    request,
    "active_workspace_id",
    optionalString(record, "active_workspace_id", path, { nonEmpty: true }),
  );
  assignOptional(
    request,
    "active_session_id",
    optionalString(record, "active_session_id", path, { nonEmpty: true }),
  );
  if (
    record.provider_selection !== undefined &&
    record.provider_selection !== null
  ) {
    request.provider_selection = parseProductProviderSelection(
      record.provider_selection,
      `${path}.provider_selection`,
      true,
    );
  }
  return request;
}

export function parseProductWorkspacesResponse(
  value: unknown,
): ProductWorkspacesResponse {
  const record = expectRecord(value, "product workspaces response");
  return {
    workspaces: expectArray(
      record.workspaces,
      "product workspaces response.workspaces",
      parseProductWorkspace,
      MAX_PRODUCT_WORKSPACES,
    ),
  };
}

export function parseProductSessionsResponse(
  value: unknown,
): ProductSessionsResponse {
  const record = expectRecord(value, "product sessions response");
  const response: ProductSessionsResponse = {
    sessions: expectArray(
      record.sessions,
      "product sessions response.sessions",
      parseProductSession,
      MAX_PRODUCT_SESSIONS,
    ),
  };
  assignOptional(
    response,
    "next_cursor",
    optionalString(record, "next_cursor", "product sessions response", {
      nonEmpty: true,
    }),
  );
  return response;
}

export function parseProductForkResponse(
  value: unknown,
  path = "product fork response",
): ProductForkResponse {
  const record = expectRecord(value, path);
  const response = {
    fork: parseProductFork(record.fork, `${path}.fork`),
    session: parseProductSession(record.session, `${path}.session`),
  };
  if (response.fork.child_product_session_id !== response.session.id) {
    schemaError(`${path}.fork.child_product_session_id`, "the returned child session id");
  }
  if (response.session.parent_session_id !== response.fork.parent_product_session_id) {
    schemaError(`${path}.session.parent_session_id`, "the fork parent session id");
  }
  if (response.session.fork_point_run_id !== response.fork.source_runtime_run_id) {
    schemaError(`${path}.session.fork_point_run_id`, "the fork source runtime run id");
  }
  if (response.session.fork_point_seq !== response.fork.fork_at_event_seq) {
    schemaError(`${path}.session.fork_point_seq`, "the fork terminal event sequence");
  }
  return response;
}

export function parseProductForksResponse(
  value: unknown,
): ProductForksResponse {
  const record = expectRecord(value, "product forks response");
  return {
    forks: expectArray(
      record.forks,
      "product forks response.forks",
      parseProductFork,
      MAX_PRODUCT_SESSIONS,
    ),
  };
}

export function parseProductProviderProfilesResponse(
  value: unknown,
): ProductProviderProfilesResponse {
  const record = expectRecord(value, "product provider profiles response");
  return {
    catalog_revision: expectString(
      record.catalog_revision,
      "product provider profiles response.catalog_revision",
      { nonEmpty: true },
    ),
    provider_profiles: expectArray(
      record.provider_profiles,
      "product provider profiles response.provider_profiles",
      parseProductProviderProfile,
      MAX_PRODUCT_PROVIDER_PROFILES,
    ),
  };
}

export function parseProductControl(
  value: unknown,
  path = "product control",
): ProductControl {
  const record = expectRecord(value, path);
  const control: ProductControl = {
    id: expectId(record.id, `${path}.id`),
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    kind: expectEnum(record.kind, PRODUCT_CONTROL_KINDS, `${path}.kind`),
    content: expectString(record.content, `${path}.content`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
    }),
    status: expectEnum(
      record.status,
      PRODUCT_CONTROL_STATUSES,
      `${path}.status`,
    ),
    seq: expectInteger(record.seq, `${path}.seq`, { min: 1 }),
    created_at: expectRfc3339Timestamp(
      record.created_at,
      `${path}.created_at`,
    ),
  };
  assignOptional(
    control,
    "idempotency_key",
    optionalString(record, "idempotency_key", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_CONTROL_IDEMPOTENCY_KEY_BYTES,
    }),
  );
  assignOptional(
    control,
    "run_id",
    optionalString(record, "run_id", path, { nonEmpty: true }),
  );
  const appliedAt = optionalString(record, "applied_at", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
  });
  if (appliedAt !== undefined) {
    control.applied_at = expectRfc3339Timestamp(
      appliedAt,
      `${path}.applied_at`,
    );
  }
  return control;
}

export function parseProductControlsResponse(
  value: unknown,
): ProductControlsResponse {
  const record = expectRecord(value, "product controls response");
  return {
    controls: expectArray(
      record.controls,
      "product controls response.controls",
      parseProductControl,
    ),
  };
}

/** The server emits three codes today; the bound keeps the array finite. */
const MAX_PRODUCT_ATTACHMENT_WARNINGS = 16;

export function parseProductMessageAttachmentRef(
  value: unknown,
  path = "product message attachment",
): ProductMessageAttachmentRef {
  const record = expectRecord(value, path);
  const reference: ProductMessageAttachmentRef = {
    attachment_id: expectId(record.attachment_id, `${path}.attachment_id`),
    content_type: expectString(record.content_type, `${path}.content_type`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    size: expectInteger(record.size, `${path}.size`, { min: 0 }),
    sha256: expectString(record.sha256, `${path}.sha256`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    availability: expectEnum(
      record.availability,
      PRODUCT_ATTACHMENT_AVAILABILITIES,
      `${path}.availability`,
    ),
  };
  assignOptional(
    reference,
    "name",
    optionalString(record, "name", path, { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES }),
  );
  assignOptional(
    reference,
    "degradation",
    optionalString(record, "degradation", path, { maxBytes: MAX_PRODUCT_TEXT_BYTES }),
  );
  return reference;
}

export function parseProductMessageAttachmentRequest(
  value: unknown,
  path = "product message attachment request",
): ProductMessageAttachmentRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["attachment_id", "name"], path);
  const request: ProductMessageAttachmentRequest = {
    attachment_id: expectId(record.attachment_id, `${path}.attachment_id`),
  };
  assignOptional(
    request,
    "name",
    optionalString(record, "name", path, { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES }),
  );
  return request;
}

export function parseProductAttachmentUpload(
  value: unknown,
  path = "product attachment upload",
): ProductAttachmentUpload {
  const record = expectRecord(value, path);
  const upload: ProductAttachmentUpload = {
    attachment_id: expectId(record.attachment_id, `${path}.attachment_id`),
    product_session_id: expectId(record.product_session_id, `${path}.product_session_id`),
    content_type: expectString(record.content_type, `${path}.content_type`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    size: expectInteger(record.size, `${path}.size`, { min: 0 }),
    sha256: expectString(record.sha256, `${path}.sha256`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    status: expectEnum(record.status, PRODUCT_ATTACHMENT_STATUSES, `${path}.status`),
    warnings: expectArray(
      record.warnings,
      `${path}.warnings`,
      (item, itemPath) =>
        expectString(item, itemPath, { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES }),
      MAX_PRODUCT_ATTACHMENT_WARNINGS,
    ),
  };
  assignOptional(
    upload,
    "name",
    optionalString(record, "name", path, { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES }),
  );
  assignOptional(
    upload,
    "expires_at",
    optionalString(record, "expires_at", path, { nonEmpty: true }),
  );
  return upload;
}

export function parseProductMessage(
  value: unknown,
  path = "product message",
): ProductMessage {
  const record = expectRecord(value, path);
  const message: ProductMessage = {
    id: expectId(record.id, `${path}.id`),
    product_session_id: expectId(record.product_session_id, `${path}.product_session_id`),
    content: expectString(record.content, `${path}.content`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
    }),
    requested_delivery: expectEnum(
      record.requested_delivery,
      PRODUCT_MESSAGE_DELIVERIES,
      `${path}.requested_delivery`,
    ),
    status: expectEnum(record.status, PRODUCT_MESSAGE_STATUSES, `${path}.status`),
    seq: expectInteger(record.seq, `${path}.seq`, { min: 1 }),
    created_at: expectRfc3339Timestamp(record.created_at, `${path}.created_at`),
  };
  if (record.actual_delivery !== undefined && record.actual_delivery !== null) {
    message.actual_delivery = expectEnum(
      record.actual_delivery,
      PRODUCT_MESSAGE_DELIVERIES,
      `${path}.actual_delivery`,
    );
  }
  assignOptional(message, "run_id", optionalString(record, "run_id", path, { nonEmpty: true }));
  assignOptional(message, "successor_run_id", optionalString(record, "successor_run_id", path, { nonEmpty: true }));
  assignOptional(
    message,
    "queue_order",
    optionalInteger(record, "queue_order", path),
  );
  assignOptional(message, "reason", optionalString(record, "reason", path, { maxBytes: MAX_PRODUCT_TEXT_BYTES }));
  const attachments = optionalArray(
    record,
    "attachments",
    path,
    parseProductMessageAttachmentRef,
    MAX_MESSAGE_ATTACHMENTS,
  );
  if (attachments !== undefined && attachments.length > 0) {
    message.attachments = attachments;
  }
  const appliedAt = optionalString(record, "applied_at", path, { nonEmpty: true });
  if (appliedAt !== undefined) {
    message.applied_at = expectRfc3339Timestamp(appliedAt, `${path}.applied_at`);
  }
  return message;
}

export function parseProductMessagesResponse(value: unknown): ProductMessagesResponse {
  const record = expectRecord(value, "product messages response");
  const response: ProductMessagesResponse = {
    messages: expectArray(
      record.messages,
      "product messages response.messages",
      parseProductMessage,
    ),
  };
  assignOptional(
    response,
    "next_after_seq",
    optionalInteger(record, "next_after_seq", "product messages response", { min: 1 }),
  );
  assignOptional(
    response,
    "next_before_seq",
    optionalInteger(record, "next_before_seq", "product messages response", { min: 1 }),
  );
  return response;
}

export function parsePromoteProductMessageRequest(
  value: unknown,
  path = "promote product message request",
): PromoteProductMessageRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["delivery"], path);
  const request: PromoteProductMessageRequest = {};
  const delivery = record.delivery;
  if (delivery !== undefined && delivery !== null) {
    request.delivery = expectEnum(delivery, PRODUCT_MESSAGE_DELIVERIES, `${path}.delivery`);
  }
  return request;
}

export function parseReorderProductMessagesRequest(
  value: unknown,
  path = "reorder product messages request",
): ReorderProductMessagesRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["ordered_ids"], path);
  const orderedIds = expectArray(
    record.ordered_ids,
    `${path}.ordered_ids`,
    (item, itemPath) => expectId(item, itemPath),
    MAX_PENDING_PRODUCT_MESSAGES,
  );
  // The server refuses a duplicate with 400 instead of merging it, so the client
  // refuses it before the request leaves: a duplicate list is never meaningful.
  const seen = new Set(orderedIds);
  if (seen.size !== orderedIds.length) {
    return schemaError(`${path}.ordered_ids`, "a list of unique ids");
  }
  return { ordered_ids: orderedIds };
}

export function parseProductQueueResponse(value: unknown): ProductQueueResponse {
  const record = expectRecord(value, "product queue response");
  return {
    messages: expectArray(
      record.messages,
      "product queue response.messages",
      parseProductMessage,
    ),
  };
}

export function parseProductMessageSearchHit(
  value: unknown,
  path = "product message search hit",
): ProductMessageSearchHit {
  const record = expectRecord(value, path);
  return {
    message_seq: expectInteger(record.message_seq, `${path}.message_seq`, { min: 1 }),
    snippet: expectString(record.snippet, `${path}.snippet`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES * 8,
      noControlCharacters: false,
    }),
    created_at: expectRfc3339Timestamp(record.created_at, `${path}.created_at`),
  };
}

export function parseProductMessagesSearchResponse(
  value: unknown,
): ProductMessagesSearchResponse {
  const record = expectRecord(value, "product messages search response");
  const response: ProductMessagesSearchResponse = {
    hits: expectArray(
      record.hits,
      "product messages search response.hits",
      parseProductMessageSearchHit,
    ),
  };
  assignOptional(
    response,
    "next_cursor",
    optionalString(record, "next_cursor", "product messages search response", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  return response;
}

export function parseProductSearchHit(
  value: unknown,
  path = "product search hit",
): ProductSearchHit {
  const record = expectRecord(value, path);
  const hit: ProductSearchHit = {
    session_id: expectId(record.session_id, `${path}.session_id`),
    source: expectEnum(record.source, PRODUCT_SEARCH_SOURCES, `${path}.source`),
    seq: expectInteger(record.seq, `${path}.seq`),
    snippet: expectString(record.snippet, `${path}.snippet`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES * 8,
      noControlCharacters: false,
    }),
  };
  assignOptional(
    hit,
    "run_id",
    optionalString(record, "run_id", path, { nonEmpty: true }),
  );
  assignOptional(
    hit,
    "run_ordinal",
    optionalInteger(record, "run_ordinal", path, { min: 0 }),
  );
  // A legacy trace line carries no timestamp; an invented one would be a
  // fabricated fact, so absence stays absence.
  const createdAt = optionalString(record, "created_at", path, { nonEmpty: true });
  if (createdAt !== undefined) {
    hit.created_at = expectRfc3339Timestamp(createdAt, `${path}.created_at`);
  }
  return hit;
}

export function parseProductSearchResponse(value: unknown): ProductSearchResponse {
  const record = expectRecord(value, "product search response");
  const response: ProductSearchResponse = {
    scope: expectString(record.scope, "product search response.scope", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_SEARCH_QUERY_BYTES * 2,
      noControlCharacters: true,
    }),
    hits: expectArray(
      record.hits,
      "product search response.hits",
      parseProductSearchHit,
    ),
  };
  assignOptional(
    response,
    "next_cursor",
    optionalString(record, "next_cursor", "product search response", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  return response;
}

const PRODUCT_COMPACTION_MODES = [
  "none",
  "deterministic",
  "model_generated",
  "automatic",
  "degraded",
  "disabled",
] as const;

export function parseProductSessionCompaction(
  value: unknown,
  path = "product session compaction",
): ProductSessionCompaction {
  const record = expectRecord(value, path);
  const compaction: ProductSessionCompaction = {
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    triggered: expectBoolean(record.triggered, `${path}.triggered`),
    // Kept a plain string: a newer mode must not fail an older bundle's parse,
    // and the UI shows the mode verbatim rather than guessing a translation.
    mode: expectString(record.mode, `${path}.mode`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    degraded: expectBoolean(record.degraded, `${path}.degraded`),
    consecutive_failures: expectInteger(
      record.consecutive_failures,
      `${path}.consecutive_failures`,
      { min: 0 },
    ),
    circuit_open: expectBoolean(record.circuit_open, `${path}.circuit_open`),
    source_message_count: expectInteger(
      record.source_message_count,
      `${path}.source_message_count`,
      { min: 0 },
    ),
    summary_truncated: expectBoolean(
      record.summary_truncated,
      `${path}.summary_truncated`,
    ),
    token_estimate: expectInteger(record.token_estimate, `${path}.token_estimate`, {
      min: 0,
    }),
  };
  assignOptional(
    compaction,
    "runtime_run_id",
    optionalString(record, "runtime_run_id", path, { nonEmpty: true }),
  );
  assignOptional(
    compaction,
    "model",
    optionalString(record, "model", path, { maxBytes: MAX_PRODUCT_TEXT_BYTES }),
  );
  assignOptional(
    compaction,
    "prompt_version",
    optionalString(record, "prompt_version", path, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
  );
  assignOptional(
    compaction,
    "summary",
    optionalString(record, "summary", path, {
      maxBytes: MAX_PRODUCT_COMPACTION_SUMMARY_CHARS * 4,
    }),
  );
  assignOptional(
    compaction,
    "failure_code",
    optionalString(record, "failure_code", path, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  const nextAttemptAfter = optionalString(record, "next_attempt_after", path, {
    nonEmpty: true,
  });
  if (nextAttemptAfter !== undefined) {
    compaction.next_attempt_after = expectRfc3339Timestamp(
      nextAttemptAfter,
      `${path}.next_attempt_after`,
    );
  }
  return compaction;
}

/**
 * The compaction modes this build names in the UI. An unknown mode is still
 * shown verbatim; this list only decides whether a translated label exists.
 */
export function isKnownProductCompactionMode(
  mode: string,
): mode is (typeof PRODUCT_COMPACTION_MODES)[number] {
  return (PRODUCT_COMPACTION_MODES as readonly string[]).includes(mode);
}

function parseStringArray(value: unknown, path: string): string[] {
  return expectArray(value, path, (item, itemPath) =>
    expectString(item, itemPath),
  );
}

function parseUsage(value: unknown, path: string): Usage {
  const record = expectRecord(value, path);
  const usage: Usage = {
    prompt_tokens: expectInteger(record.prompt_tokens, `${path}.prompt_tokens`, {
      min: 0,
    }),
    completion_tokens: expectInteger(
      record.completion_tokens,
      `${path}.completion_tokens`,
      { min: 0 },
    ),
    total_tokens: expectInteger(record.total_tokens, `${path}.total_tokens`, {
      min: 0,
    }),
  };
  assignOptional(
    usage,
    "cached_tokens",
    optionalInteger(record, "cached_tokens", path, { min: 0 }),
  );
  return usage;
}

function parsePlanStep(value: unknown, path: string): PlanStep {
  const record = expectRecord(value, path);
  return {
    id: expectString(record.id, `${path}.id`, { nonEmpty: true }),
    title: expectString(record.title, `${path}.title`),
    done: expectBoolean(record.done, `${path}.done`),
  };
}

function parseTaskPlan(value: unknown, path: string): TaskPlan {
  const record = expectRecord(value, path);
  return {
    goal: expectString(record.goal, `${path}.goal`),
    steps: expectArray(record.steps, `${path}.steps`, parsePlanStep),
    current_step: expectInteger(record.current_step, `${path}.current_step`, {
      min: 0,
    }),
  };
}

function parseToolMutation(value: unknown, path: string): ToolMutation {
  const record = expectRecord(value, path);
  const mutation: ToolMutation = {
    path: expectString(record.path, `${path}.path`),
    operation: expectEnum(
      record.operation,
      ["create", "update", "delete", "unknown"] as const satisfies readonly ToolMutationOperation[],
      `${path}.operation`,
    ),
  };
  const diff = optionalNullableString(record, "diff", path);
  if (diff !== undefined) {
    mutation.diff = diff;
  }
  return mutation;
}

function defaultToolExecutionMetadata(): ProductToolExecutionMetadata {
  return {
    status: "ok",
    risk_level: "low",
    read_only: false,
    affected_paths: [],
    workspace_changed: false,
    diff_summary: [],
  };
}

function parseToolExecutionMetadata(
  value: unknown,
  path: string,
): ProductToolExecutionMetadata {
  if (value === undefined) {
    return defaultToolExecutionMetadata();
  }
  const record = expectRecord(value, path);
  const metadata: ProductToolExecutionMetadata = {
    status: expectEnum(
      record.status,
      ["ok", "error", "rejected", "partial_success"] as const,
      `${path}.status`,
    ),
    risk_level: expectEnum(
      record.risk_level,
      ["low", "high"] as const,
      `${path}.risk_level`,
    ),
    read_only: expectBoolean(record.read_only, `${path}.read_only`),
    affected_paths:
      record.affected_paths === undefined
        ? []
        : parseStringArray(record.affected_paths, `${path}.affected_paths`),
    workspace_changed: expectBoolean(
      record.workspace_changed,
      `${path}.workspace_changed`,
    ),
    diff_summary:
      record.diff_summary === undefined
        ? []
        : parseStringArray(record.diff_summary, `${path}.diff_summary`),
  };
  assignOptional(
    metadata,
    "error_code",
    optionalString(record, "error_code", path),
  );
  assignOptional(
    metadata,
    "security_event_type",
    optionalString(record, "security_event_type", path),
  );
  return metadata;
}

function parseProcedureReference(
  value: unknown,
  path: string,
): ProcedureReference {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["id", "version", "trust", "source_path", "content_hash"],
    path,
  );
  return {
    id: expectString(record.id, `${path}.id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    version: expectString(record.version, `${path}.version`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    trust: expectEnum(
      record.trust,
      [
        "builtin_trusted",
        "workspace_trusted",
        "user_installed",
        "external_untrusted",
      ] as const,
      `${path}.trust`,
    ),
    source_path: expectString(record.source_path, `${path}.source_path`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
      noControlCharacters: true,
    }),
    content_hash: expectString(record.content_hash, `${path}.content_hash`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  };
}

function parseProcedureCapabilityBinding(value: unknown, path: string): ProcedureCapabilityBinding {
  const record = expectRecord(value, path);
  const binding: ProcedureCapabilityBinding = {
    capability_id: expectString(record.capability_id, `${path}.capability_id`, { nonEmpty: true }),
    available: expectBoolean(record.available, `${path}.available`),
    approval_required: expectBoolean(record.approval_required, `${path}.approval_required`),
  };
  if (record.required !== undefined && record.required !== null) {
    binding.required = expectBoolean(record.required, `${path}.required`);
  }
  if (record.tool_name !== undefined && record.tool_name !== null) {
    binding.tool_name = expectString(record.tool_name, `${path}.tool_name`, { nonEmpty: true });
  }
  if (record.mutation_class !== undefined && record.mutation_class !== null) {
    binding.mutation_class = expectEnum(record.mutation_class, ["read_only", "mutating"] as const, `${path}.mutation_class`);
  }
  return binding;
}

function parseProcedureApplication(value: unknown, path: string): ProcedureApplication {
  const record = expectRecord(value, path);
  const application: ProcedureApplication = {
    application_id: expectString(record.application_id, `${path}.application_id`, { nonEmpty: true }),
    reference: parseProcedureReference(record.reference, `${path}.reference`),
    hydration_hash: expectString(record.hydration_hash, `${path}.hydration_hash`, { nonEmpty: true }),
    capability_snapshot_id: expectString(record.capability_snapshot_id, `${path}.capability_snapshot_id`, { nonEmpty: true }),
    risk_level: expectEnum(record.risk_level, ["low", "medium", "high"] as const, `${path}.risk_level`),
    boundary: expectString(record.boundary, `${path}.boundary`, { nonEmpty: true }),
  };
  if (record.section_ids !== undefined && record.section_ids !== null) {
    application.section_ids = parseStringArray(record.section_ids, `${path}.section_ids`);
  }
  if (record.side_effects !== undefined && record.side_effects !== null) {
    application.side_effects = parseStringArray(record.side_effects, `${path}.side_effects`);
  }
  if (record.truncated !== undefined && record.truncated !== null) {
    application.truncated = expectBoolean(record.truncated, `${path}.truncated`);
  }
  if (record.step_id !== undefined && record.step_id !== null) {
    application.step_id = expectString(record.step_id, `${path}.step_id`, { nonEmpty: true });
  }
  if (record.capability_bindings !== undefined && record.capability_bindings !== null) {
    application.capability_bindings = expectArray(
      record.capability_bindings,
      `${path}.capability_bindings`,
      parseProcedureCapabilityBinding,
    );
  }
  return application;
}

function parseProcedureDeviation(value: unknown, path: string): ProcedureDeviation {
  const record = expectRecord(value, path);
  const deviation: ProcedureDeviation = {
    deviation_id: expectString(record.deviation_id, `${path}.deviation_id`, { nonEmpty: true }),
    reference: parseProcedureReference(record.reference, `${path}.reference`),
    reason: expectEnum(
      record.reason,
      [
        "evidence_contradiction",
        "capability_unavailable",
        "preconditions_unsatisfied",
        "user_constraint",
        "procedure_stale",
        "safer_alternative",
        "runtime_failure",
      ] as const,
      `${path}.reason`,
    ),
    safe_summary: expectString(record.safe_summary, `${path}.safe_summary`),
  };
  if (record.application_id !== undefined && record.application_id !== null) {
    deviation.application_id = expectString(record.application_id, `${path}.application_id`, { nonEmpty: true });
  }
  if (record.material !== undefined && record.material !== null) {
    deviation.material = expectBoolean(record.material, `${path}.material`);
  }
  if (record.evidence_refs !== undefined && record.evidence_refs !== null) {
    deviation.evidence_refs = parseStringArray(record.evidence_refs, `${path}.evidence_refs`);
  }
  return deviation;
}

function parseToolResult(value: unknown, path: string): ProductToolResult {
  const record = expectRecord(value, path);
  const result: ProductToolResult = {
    call_id: expectString(record.call_id, `${path}.call_id`, { nonEmpty: true }),
    output: expectString(record.output, `${path}.output`),
    metadata: parseToolExecutionMetadata(
      record.metadata,
      `${path}.metadata`,
    ),
  };
  if (record.mutations !== undefined && record.mutations !== null) {
    result.mutations = expectArray(
      record.mutations,
      `${path}.mutations`,
      parseToolMutation,
    );
  }
  if (record.procedure_applications !== undefined && record.procedure_applications !== null) {
    result.procedure_applications = expectArray(
      record.procedure_applications,
      `${path}.procedure_applications`,
      parseProcedureApplication,
    );
  }
  if (record.procedure_deviations !== undefined && record.procedure_deviations !== null) {
    result.procedure_deviations = expectArray(
      record.procedure_deviations,
      `${path}.procedure_deviations`,
      parseProcedureDeviation,
    );
  }
  if (record.envelope !== undefined && record.envelope !== null) {
    result.envelope = parseToolOutputEnvelope(
      record.envelope,
      `${path}.envelope`,
    );
  }
  return result;
}

function parseToolArtifactSource(
  value: unknown,
  path: string,
): ToolArtifactSource {
  const record = expectRecord(value, path);
  const source: ToolArtifactSource = {
    run_id: expectString(record.run_id, `${path}.run_id`, { nonEmpty: true }),
    call_id: expectString(record.call_id, `${path}.call_id`, {
      nonEmpty: true,
    }),
    block_ordinal: expectInteger(record.block_ordinal, `${path}.block_ordinal`, {
      min: 0,
    }),
    captured_at: expectString(record.captured_at, `${path}.captured_at`, {
      nonEmpty: true,
    }),
  };
  for (const key of [
    "server_config_id",
    "server_identity_hash",
    "session_hash",
    "remote_tool_name",
  ] as const) {
    if (record[key] !== undefined && record[key] !== null) {
      source[key] = expectString(record[key], `${path}.${key}`);
    }
  }
  return source;
}

function parseToolArtifactRef(value: unknown, path: string): ToolArtifactRef {
  const record = expectRecord(value, path);
  const artifact: ToolArtifactRef = {
    artifact_id: expectString(record.artifact_id, `${path}.artifact_id`, {
      nonEmpty: true,
    }),
    kind: expectEnum(record.kind, TOOL_ARTIFACT_KINDS, `${path}.kind`),
    byte_length: expectInteger(record.byte_length, `${path}.byte_length`, {
      min: 0,
    }),
    sha256: expectString(record.sha256, `${path}.sha256`, { nonEmpty: true }),
    storage_ref: expectString(record.storage_ref, `${path}.storage_ref`, {
      nonEmpty: true,
    }),
    source: parseToolArtifactSource(record.source, `${path}.source`),
  };
  if (record.mime_type !== undefined && record.mime_type !== null) {
    artifact.mime_type = expectString(record.mime_type, `${path}.mime_type`);
  }
  if (record.original_uri !== undefined && record.original_uri !== null) {
    artifact.original_uri = expectString(
      record.original_uri,
      `${path}.original_uri`,
    );
  }
  if (record.validation !== undefined && record.validation !== null) {
    artifact.validation = expectEnum(
      record.validation,
      ARTIFACT_VALIDATION_STATES,
      `${path}.validation`,
    );
  }
  if (record.sensitivity !== undefined && record.sensitivity !== null) {
    artifact.sensitivity = expectEnum(
      record.sensitivity,
      ["normal", "sensitive"] as const,
      `${path}.sensitivity`,
    );
  }
  if (record.trust !== undefined && record.trust !== null) {
    artifact.trust = expectEnum(
      record.trust,
      ["untrusted", "local_tool"] as const,
      `${path}.trust`,
    );
  }
  return artifact;
}

/**
 * Protocol identity and timing a tool result may carry. Only the fields the
 * shell renders are kept; an unrecognized key is dropped rather than forwarded.
 */
function parseToolProtocolMetadata(
  value: unknown,
  path: string,
): ToolProtocolMetadata {
  const record = expectRecord(value, path);
  const metadata: ToolProtocolMetadata = {};
  for (const key of [
    "protocol",
    "server_config_id",
    "protocol_version",
    "remote_tool_name",
  ] as const) {
    const candidate = record[key];
    if (candidate !== undefined && candidate !== null) {
      metadata[key] = expectString(candidate, `${path}.${key}`, {
        maxBytes: 512,
        noControlCharacters: true,
      });
    }
  }
  for (const key of ["attempt_count", "duration_ms"] as const) {
    const candidate = record[key];
    if (candidate !== undefined && candidate !== null) {
      metadata[key] = expectInteger(candidate, `${path}.${key}`, { min: 0 });
    }
  }
  return metadata;
}

function parseToolOutputEnvelope(
  value: unknown,
  path: string,
): ToolOutputEnvelope {
  const record = expectRecord(value, path);
  const envelope: ToolOutputEnvelope = {
    summary_text: expectString(record.summary_text, `${path}.summary_text`),
  };
  if (record.outcome !== undefined && record.outcome !== null) {
    envelope.outcome = expectEnum(
      record.outcome,
      TOOL_RESULT_OUTCOMES,
      `${path}.outcome`,
    );
  }
  if (record.artifacts !== undefined && record.artifacts !== null) {
    envelope.artifacts = expectArray(
      record.artifacts,
      `${path}.artifacts`,
      parseToolArtifactRef,
    );
  }
  // Content blocks, structured content, effects, and diagnostics are passed
  // through as validated-shape records rather than re-modelled here: the server
  // is the authority on their contents, and the UI reads them through narrow
  // accessors. Protocol metadata is narrowed to the fields the shell reads, so
  // an unrecognized key cannot reach a component.
  if (record.protocol_metadata !== undefined && record.protocol_metadata !== null) {
    envelope.protocol_metadata = parseToolProtocolMetadata(
      record.protocol_metadata,
      `${path}.protocol_metadata`,
    );
  }
  if (record.content_blocks !== undefined && record.content_blocks !== null) {
    envelope.content_blocks = expectArray(
      record.content_blocks,
      `${path}.content_blocks`,
      (block, blockPath) =>
        parseToolContentBlock(block, blockPath),
    );
  }
  if (record.diagnostics !== undefined && record.diagnostics !== null) {
    envelope.diagnostics = expectArray(
      record.diagnostics,
      `${path}.diagnostics`,
      (diagnostic, diagnosticPath) => {
        const entry = expectRecord(diagnostic, diagnosticPath);
        return {
          domain: expectString(entry.domain, `${diagnosticPath}.domain`),
          code: expectString(entry.code, `${diagnosticPath}.code`),
          message: expectString(entry.message, `${diagnosticPath}.message`),
        };
      },
    );
  }
  if (
    record.external_effects !== undefined &&
    record.external_effects !== null
  ) {
    envelope.external_effects = expectArray(
      record.external_effects,
      `${path}.external_effects`,
      (effect, effectPath) => {
        const entry = expectRecord(effect, effectPath);
        return {
          kind: expectString(entry.kind, `${effectPath}.kind`),
          target: expectString(entry.target, `${effectPath}.target`),
          indeterminate:
            entry.indeterminate === undefined || entry.indeterminate === null
              ? undefined
              : expectBoolean(
                  entry.indeterminate,
                  `${effectPath}.indeterminate`,
                ),
        };
      },
    );
  }
  return envelope;
}

function parseToolContentBlockMeta(
  value: unknown,
  path: string,
): ToolContentBlockMeta {
  const record = expectRecord(value, path);
  const meta: ToolContentBlockMeta = {
    ordinal: expectInteger(record.ordinal, `${path}.ordinal`, { min: 0 }),
  };
  if (record.mime_type !== undefined && record.mime_type !== null) {
    meta.mime_type = expectString(record.mime_type, `${path}.mime_type`);
  }
  if (record.truncated !== undefined && record.truncated !== null) {
    meta.truncated = expectBoolean(record.truncated, `${path}.truncated`);
  }
  if (record.validation !== undefined && record.validation !== null) {
    meta.validation = expectEnum(
      record.validation,
      ARTIFACT_VALIDATION_STATES,
      `${path}.validation`,
    );
  }
  return meta;
}

function parseToolContentBlock(
  value: unknown,
  path: string,
): ToolContentBlock {
  const record = expectRecord(value, path);
  const type = expectEnum(
    record.type,
    [
      "text",
      "image",
      "audio",
      "resource_link",
      "embedded_resource",
      "unknown",
    ] as const,
    `${path}.type`,
  );
  const meta = parseToolContentBlockMeta(record.meta, `${path}.meta`);
  switch (type) {
    case "text":
      return { type, meta, text: expectString(record.text, `${path}.text`) };
    case "image":
    case "audio":
      return {
        type,
        meta,
        artifact: parseToolArtifactRef(record.artifact, `${path}.artifact`),
      };
    case "resource_link":
      return {
        type,
        meta,
        uri: expectString(record.uri, `${path}.uri`, { nonEmpty: true }),
        name:
          record.name === undefined || record.name === null
            ? undefined
            : expectString(record.name, `${path}.name`),
      };
    case "embedded_resource":
      return {
        type,
        meta,
        artifact: parseToolArtifactRef(record.artifact, `${path}.artifact`),
        preview:
          record.preview === undefined || record.preview === null
            ? undefined
            : expectString(record.preview, `${path}.preview`),
      };
    case "unknown":
      return {
        type,
        meta,
        declared_type: expectString(
          record.declared_type,
          `${path}.declared_type`,
        ),
        retained:
          record.retained === undefined || record.retained === null
            ? undefined
            : expectString(record.retained, `${path}.retained`),
      };
  }
}

function parseToolCallRef(value: unknown, path: string): ToolCallRef {
  const record = expectRecord(value, path);
  return {
    id: expectString(record.id, `${path}.id`, { nonEmpty: true }),
    name: expectString(record.name, `${path}.name`, { nonEmpty: true }),
    args: record.args,
  };
}

function parseToolError(value: unknown, path: string): ToolError {
  const record = expectRecord(value, path);
  const error: ToolError = {
    code: expectString(record.code, `${path}.code`, { nonEmpty: true }),
  };
  assignOptional(error, "reason", optionalString(record, "reason", path));
  assignOptional(
    error,
    "timeout_ms",
    optionalInteger(record, "timeout_ms", path, { min: 0 }),
  );
  assignOptional(error, "name", optionalString(record, "name", path));
  return error;
}

/** An additive counter that older payloads may omit entirely. */
function optionalCounter(value: unknown, path: string): number {
  if (value === undefined || value === null) {
    return 0;
  }
  return expectInteger(value, path, { min: 0 });
}

function parseExecutionBudgetUsage(
  value: unknown,
  path: string,
): ExecutionBudgetUsage {
  const record = expectRecord(value, path);
  return {
    plan_steps: expectInteger(record.plan_steps, `${path}.plan_steps`, { min: 0 }),
    step_attempts: expectInteger(record.step_attempts, `${path}.step_attempts`, {
      min: 0,
    }),
    model_turns: expectInteger(record.model_turns, `${path}.model_turns`, {
      min: 0,
    }),
    tool_calls: expectInteger(record.tool_calls, `${path}.tool_calls`, { min: 0 }),
    plan_revisions: expectInteger(
      record.plan_revisions,
      `${path}.plan_revisions`,
      { min: 0 },
    ),
    // Additive per-phase counters. Older payloads omit them, so they default to
    // zero rather than failing a snapshot that is otherwise valid.
    model_repairs: optionalCounter(record.model_repairs, `${path}.model_repairs`),
    planner_turns: optionalCounter(record.planner_turns, `${path}.planner_turns`),
    evaluator_turns: optionalCounter(
      record.evaluator_turns,
      `${path}.evaluator_turns`,
    ),
    replanner_turns: optionalCounter(
      record.replanner_turns,
      `${path}.replanner_turns`,
    ),
    finalization_turns: optionalCounter(
      record.finalization_turns,
      `${path}.finalization_turns`,
    ),
    wall_time_ms: expectInteger(record.wall_time_ms, `${path}.wall_time_ms`, {
      min: 0,
    }),
    total_tokens: expectInteger(record.total_tokens, `${path}.total_tokens`, {
      min: 0,
    }),
    cost_microunits: expectInteger(
      record.cost_microunits,
      `${path}.cost_microunits`,
      { min: 0 },
    ),
  };
}

function parsePlanRevision(value: unknown, path: string): PlanRevision {
  const record = expectRecord(value, path);
  const revision: PlanRevision = {
    plan_id: expectString(record.plan_id, `${path}.plan_id`, { nonEmpty: true }),
    revision_id: expectString(record.revision_id, `${path}.revision_id`, {
      nonEmpty: true,
    }),
    revision: expectInteger(record.revision, `${path}.revision`, { min: 0 }),
    created_at: expectString(record.created_at, `${path}.created_at`, {
      nonEmpty: true,
    }),
    decision_id: expectString(record.decision_id, `${path}.decision_id`, {
      nonEmpty: true,
    }),
    budget_snapshot: parseExecutionBudgetUsage(
      record.budget_snapshot,
      `${path}.budget_snapshot`,
    ),
  };
  assignOptional(
    revision,
    "parent_revision_id",
    optionalString(record, "parent_revision_id", path, { nonEmpty: true }),
  );
  assignOptional(
    revision,
    "trigger_step_record_id",
    optionalString(record, "trigger_step_record_id", path, { nonEmpty: true }),
  );
  assignOptional(
    revision,
    "capability_snapshot_id",
    optionalString(record, "capability_snapshot_id", path, { nonEmpty: true }),
  );
  if (record.safe_reason_codes !== undefined && record.safe_reason_codes !== null) {
    revision.safe_reason_codes = parseStringArray(
      record.safe_reason_codes,
      `${path}.safe_reason_codes`,
    );
  }
  if (record.retained_step_ids !== undefined && record.retained_step_ids !== null) {
    revision.retained_step_ids = parseStringArray(
      record.retained_step_ids,
      `${path}.retained_step_ids`,
    );
  }
  if (
    record.superseded_remaining_step_ids !== undefined &&
    record.superseded_remaining_step_ids !== null
  ) {
    revision.superseded_remaining_step_ids = parseStringArray(
      record.superseded_remaining_step_ids,
      `${path}.superseded_remaining_step_ids`,
    );
  }
  if (record.remaining_steps !== undefined && record.remaining_steps !== null) {
    revision.remaining_steps = expectArray(
      record.remaining_steps,
      `${path}.remaining_steps`,
      parsePlanStep,
    );
  }
  return revision;
}

function parseStepRecord(value: unknown, path: string): StepRecord {
  const record = expectRecord(value, path);
  const stepRecord: StepRecord = {
    record_id: expectString(record.record_id, `${path}.record_id`, {
      nonEmpty: true,
    }),
    plan_id: expectString(record.plan_id, `${path}.plan_id`, { nonEmpty: true }),
    plan_revision_id: expectString(
      record.plan_revision_id,
      `${path}.plan_revision_id`,
      { nonEmpty: true },
    ),
    step_id: expectString(record.step_id, `${path}.step_id`, { nonEmpty: true }),
    attempt: expectInteger(record.attempt, `${path}.attempt`, { min: 0 }),
    status: expectEnum(
      record.status,
      [
        "succeeded",
        "partial",
        "failed",
        "blocked",
        "rejected",
        "skipped",
        "budget_exhausted",
        "cancelled",
        "interrupted",
        "indeterminate",
      ] as const,
      `${path}.status`,
    ),
    started_at: expectString(record.started_at, `${path}.started_at`, {
      nonEmpty: true,
    }),
    finished_at: expectString(record.finished_at, `${path}.finished_at`, {
      nonEmpty: true,
    }),
    summary: expectString(record.summary, `${path}.summary`),
    completion_basis: expectEnum(
      record.completion_basis,
      [
        "model_conclusion",
        "deterministic_rule",
        "user_decision",
        "runtime_failure",
      ] as const,
      `${path}.completion_basis`,
    ),
    model_turns_used: expectInteger(
      record.model_turns_used,
      `${path}.model_turns_used`,
      { min: 0 },
    ),
    tool_calls_used: expectInteger(
      record.tool_calls_used,
      `${path}.tool_calls_used`,
      { min: 0 },
    ),
    token_usage: parseUsage(record.token_usage, `${path}.token_usage`),
  };
  for (const [key, field] of [
    ["evidence_refs", "evidence_refs"],
    ["tool_call_ids", "tool_call_ids"],
    ["artifact_refs", "artifact_refs"],
  ] as const) {
    if (record[key] !== undefined && record[key] !== null) {
      stepRecord[field] = parseStringArray(record[key], `${path}.${key}`);
    }
  }
  if (record.mutations !== undefined && record.mutations !== null) {
    stepRecord.mutations = expectArray(
      record.mutations,
      `${path}.mutations`,
      parseToolMutation,
    );
  }
  assignOptional(
    stepRecord,
    "error_code",
    optionalString(record, "error_code", path),
  );
  assignOptional(
    stepRecord,
    "safe_error_summary",
    optionalString(record, "safe_error_summary", path),
  );
  assignOptional(
    stepRecord,
    "supersedes_record_id",
    optionalString(record, "supersedes_record_id", path),
  );
  if (record.ambiguity !== undefined && record.ambiguity !== null) {
    stepRecord.ambiguity = parsePlanAmbiguity(
      record.ambiguity,
      `${path}.ambiguity`,
    );
  }
  return stepRecord;
}

function parsePlanAmbiguity(value: unknown, path: string): PlanAmbiguity {
  const record = expectRecord(value, path);
  const ambiguity: PlanAmbiguity = {
    kind: expectEnum(
      record.kind,
      [
        "remaining_work_may_be_unnecessary",
        "plan_assumption_may_be_invalid",
        "recoverable_alternative_may_exist",
        "goal_may_be_partially_satisfied",
        "remaining_dependencies_may_need_reordering",
      ] as const,
      `${path}.kind`,
    ),
    safe_summary: expectString(record.safe_summary, `${path}.safe_summary`),
  };
  if (record.evidence_refs !== undefined && record.evidence_refs !== null) {
    ambiguity.evidence_refs = parseStringArray(
      record.evidence_refs,
      `${path}.evidence_refs`,
    );
  }
  return ambiguity;
}

const EXECUTION_PHASES = [
  "planner",
  "step",
  "evaluator",
  "replanner",
  "finalizer",
  "run",
] as const;

const EXECUTION_BUDGET_DIMENSIONS = [
  "plan_steps",
  "step_attempts",
  "model_turns",
  "model_turns_per_step",
  "tool_calls",
  "tool_calls_per_step",
  "plan_revisions",
  "model_repairs",
  "finalization_turns",
  "wall_time",
  "total_tokens",
  "cost",
] as const;

const PLAN_FINISH_REASONS = [
  "completed",
  "partial",
  "blocked",
  "budget_exhausted",
  "failed",
  "cancelled",
  "interrupted",
  "rejected",
  "indeterminate",
] as const;

function parseExecutionBudgetLimits(
  value: unknown,
  path: string,
): ExecutionBudgetLimits {
  const record = expectRecord(value, path);
  const limits: ExecutionBudgetLimits = {};
  for (const key of [
    "max_plan_steps",
    "max_step_attempts",
    "max_model_turns",
    "max_model_turns_per_step",
    "max_tool_calls",
    "max_tool_calls_per_step",
    "max_plan_revisions",
    "max_model_repairs",
    "max_finalization_turns",
    "max_wall_time_ms",
    "max_total_tokens",
    "max_cost_microunits",
  ] as const) {
    // An unset dimension stays absent rather than becoming a fabricated bound.
    if (record[key] !== undefined && record[key] !== null) {
      limits[key] = expectInteger(record[key], `${path}.${key}`, { min: 1 });
    }
  }
  return limits;
}

function parseExecutionBudgetExhaustion(
  value: unknown,
  path: string,
): ExecutionBudgetExhaustion {
  const record = expectRecord(value, path);
  return {
    dimension: expectEnum(
      record.dimension,
      EXECUTION_BUDGET_DIMENSIONS,
      `${path}.dimension`,
    ),
    phase: expectEnum(record.phase, EXECUTION_PHASES, `${path}.phase`),
    limit: expectInteger(record.limit, `${path}.limit`, { min: 0 }),
    consumed: expectInteger(record.consumed, `${path}.consumed`, { min: 0 }),
    safe_summary: expectString(record.safe_summary, `${path}.safe_summary`),
  };
}

function parseExecutionBudgetSnapshot(
  value: unknown,
  path: string,
): ExecutionBudgetSnapshot {
  const record = expectRecord(value, path);
  const snapshot: ExecutionBudgetSnapshot = {
    limits: parseExecutionBudgetLimits(record.limits ?? {}, `${path}.limits`),
    consumed: parseExecutionBudgetUsage(record.consumed, `${path}.consumed`),
    cost_enforced: optionalBoolean(record, "cost_enforced", path) ?? false,
  };
  if (record.exhausted !== undefined && record.exhausted !== null) {
    snapshot.exhausted = parseExecutionBudgetExhaustion(
      record.exhausted,
      `${path}.exhausted`,
    );
  }
  return snapshot;
}

function parseExecutionPolicy(value: unknown, path: string): ExecutionPolicy {
  const record = expectRecord(value, path);
  const policy: ExecutionPolicy = {
    version: expectInteger(record.version, `${path}.version`, { min: 0 }),
    strategy: expectEnum(
      record.strategy,
      ["react", "plan_react"] as const,
      `${path}.strategy`,
    ),
    selection_source: expectEnum(
      record.selection_source,
      [
        "request",
        "session",
        "config",
        "compatibility_default",
        "max_steps_and_plan_flag",
      ] as const,
      `${path}.selection_source`,
    ),
    budgets: parseExecutionBudgetLimits(
      record.budgets ?? {},
      `${path}.budgets`,
    ),
  };
  if (record.evaluator_mode !== undefined && record.evaluator_mode !== null) {
    policy.evaluator_mode = expectEnum(
      record.evaluator_mode,
      ["rule_only", "rule_first_model_on_ambiguity"] as const,
      `${path}.evaluator_mode`,
    );
  }
  if (record.finalizer_policy !== undefined && record.finalizer_policy !== null) {
    policy.finalizer_policy = expectEnum(
      record.finalizer_policy,
      ["deterministic", "model_preferred"] as const,
      `${path}.finalizer_policy`,
    );
  }
  return policy;
}

function parseExecutionDegradation(
  value: unknown,
  path: string,
): ExecutionDegradation {
  const record = expectRecord(value, path);
  return {
    degradation_id: expectString(
      record.degradation_id,
      `${path}.degradation_id`,
      { nonEmpty: true },
    ),
    phase: expectEnum(record.phase, EXECUTION_PHASES, `${path}.phase`),
    code: expectString(record.code, `${path}.code`, { nonEmpty: true }),
    safe_summary: expectString(record.safe_summary, `${path}.safe_summary`),
    occurred_at: expectString(record.occurred_at, `${path}.occurred_at`, {
      nonEmpty: true,
    }),
  };
}

function parseFinalizationRecord(
  value: unknown,
  path: string,
): FinalizationRecord {
  const record = expectRecord(value, path);
  const finalization: FinalizationRecord = {
    finalization_id: expectString(
      record.finalization_id,
      `${path}.finalization_id`,
      { nonEmpty: true },
    ),
    phase: expectEnum(
      record.phase,
      ["started", "completed"] as const,
      `${path}.phase`,
    ),
    finish_reason: expectEnum(
      record.finish_reason,
      PLAN_FINISH_REASONS,
      `${path}.finish_reason`,
    ),
    mode: expectEnum(
      record.mode,
      ["direct", "model", "deterministic", "deterministic_fallback"] as const,
      `${path}.mode`,
    ),
    started_at: expectString(record.started_at, `${path}.started_at`, {
      nonEmpty: true,
    }),
  };
  if (record.outcome !== undefined && record.outcome !== null) {
    finalization.outcome = expectEnum(
      record.outcome,
      [
        "success",
        "partial",
        "blocked",
        "rejected",
        "cancelled",
        "interrupted",
        "exhausted",
        "indeterminate",
        "failed",
      ] as const,
      `${path}.outcome`,
    );
  }
  assignOptional(
    finalization,
    "completed_at",
    optionalString(record, "completed_at", path, { nonEmpty: true }),
  );
  const output = optionalNullableString(record, "output", path);
  if (output !== undefined) {
    finalization.output = output;
  }
  for (const key of ["evidence_refs", "incomplete_step_ids"] as const) {
    if (record[key] !== undefined && record[key] !== null) {
      finalization[key] = parseStringArray(record[key], `${path}.${key}`);
    }
  }
  for (const key of ["budget_before", "budget_after"] as const) {
    if (record[key] !== undefined && record[key] !== null) {
      finalization[key] = parseExecutionBudgetUsage(
        record[key],
        `${path}.${key}`,
      );
    }
  }
  return finalization;
}

function parsePlanDecision(value: unknown, path: string): PlanDecision {
  const record = expectRecord(value, path);
  const decision: PlanDecision = {
    decision_id: expectString(record.decision_id, `${path}.decision_id`, {
      nonEmpty: true,
    }),
    kind: expectEnum(
      record.kind,
      ["continue", "replace_remaining", "finish"] as const,
      `${path}.kind`,
    ),
    safe_summary: expectString(record.safe_summary, `${path}.safe_summary`),
  };
  if (record.safe_reason_codes !== undefined && record.safe_reason_codes !== null) {
    decision.safe_reason_codes = parseStringArray(
      record.safe_reason_codes,
      `${path}.safe_reason_codes`,
    );
  }
  if (
    record.remaining_work_requirements !== undefined &&
    record.remaining_work_requirements !== null
  ) {
    decision.remaining_work_requirements = parseStringArray(
      record.remaining_work_requirements,
      `${path}.remaining_work_requirements`,
    );
  }
  if (record.finish_reason !== undefined && record.finish_reason !== null) {
    decision.finish_reason = expectEnum(
      record.finish_reason,
      [
        "completed",
        "partial",
        "blocked",
        "budget_exhausted",
        "failed",
        "cancelled",
        "interrupted",
      ] as const,
      `${path}.finish_reason`,
    );
  }
  return decision;
}

function parsePlanDecisionRecord(
  value: unknown,
  path: string,
): PlanDecisionRecord {
  const record = expectRecord(value, path);
  return {
    trigger_step_record_id: expectString(
      record.trigger_step_record_id,
      `${path}.trigger_step_record_id`,
      { nonEmpty: true },
    ),
    decided_at: expectString(record.decided_at, `${path}.decided_at`, {
      nonEmpty: true,
    }),
    decision: parsePlanDecision(record.decision, `${path}.decision`),
  };
}

function parsePromptCompactionState(
  value: unknown,
  path: string,
): PromptCompactionState {
  const record = expectRecord(value, path);
  const state: PromptCompactionState = {
    mode: expectEnum(
      record.mode,
      [
        "none",
        "deterministic",
        "model_generated",
        "automatic",
        "degraded",
        "disabled",
      ] as const,
      `${path}.mode`,
    ),
    auto_triggered: expectBoolean(record.auto_triggered, `${path}.auto_triggered`),
    degraded: expectBoolean(record.degraded, `${path}.degraded`),
    consecutive_failures: expectInteger(
      record.consecutive_failures,
      `${path}.consecutive_failures`,
      { min: 0 },
    ),
    circuit_open: expectBoolean(record.circuit_open, `${path}.circuit_open`),
    source_message_count: expectInteger(
      record.source_message_count,
      `${path}.source_message_count`,
      { min: 0 },
    ),
  };
  assignOptional(state, "model", optionalString(record, "model", path));
  assignOptional(
    state,
    "prompt_version",
    optionalString(record, "prompt_version", path),
  );
  assignOptional(
    state,
    "last_error",
    optionalString(record, "last_error", path),
  );
  return state;
}

/**
 * Validate the flat pruning members of one prompt-build record and decode them.
 *
 * This function validates; `readPromptPruningFacts` decides which members exist
 * and what an omitted one means. The split is deliberate: the restored path has a
 * schema and refuses a malformed member, while the live path (`lib/rove-state.ts`)
 * has only a frame and reads the same record leniently — both must agree on the
 * predicates, the presence rule and the defaults, which is why those live in one
 * place and not in two. The bounds themselves are `lib/rove-types.ts` constants
 * (`MAX_PRUNED_PAYLOAD_DIGESTS`, `MAX_PRUNING_DIGEST_BYTES`,
 * `MAX_PRUNING_POLICY_BYTES`), deliberately looser than the runtime's own policy
 * constants: the runtime caps the digest list at 8, but a policy version bump must
 * not turn a transcript read into a schema error, so these bounds only stop an
 * unbounded or nonsensical payload.
 */

function parsePromptBuildPruning(
  record: UnknownRecord,
  path: string,
): ProductPromptPruning | undefined {
  const validated: UnknownRecord = {};
  // Validation follows the order the runtime writes these facts in — counts,
  // then the digest list, then the policy — so the first thing a refused payload
  // reports is the first thing the record carries. The counts come from the
  // shared key list rather than a second copy of it.
  for (const key of PRUNING_WIRE_COUNT_KEYS) {
    const value = optionalInteger(record, key, path, { min: 0 });
    if (value !== undefined) {
      validated[key] = value;
    }
  }
  const digests = optionalArray(
    record,
    "pruned_payload_digests",
    path,
    (item, itemPath) =>
      expectString(item, itemPath, {
        nonEmpty: true,
        maxBytes: MAX_PRUNING_DIGEST_BYTES,
        noControlCharacters: true,
      }),
    MAX_PRUNED_PAYLOAD_DIGESTS,
  );
  if (digests !== undefined) {
    validated.pruned_payload_digests = digests;
  }
  const policy = optionalString(record, "pruning_policy", path, {
    nonEmpty: true,
    maxBytes: MAX_PRUNING_POLICY_BYTES,
    noControlCharacters: true,
  });
  if (policy !== undefined) {
    validated.pruning_policy = policy;
  }
  return readPromptPruningFacts(validated);
}

function parsePromptBuildMetadata(
  value: unknown,
  path: string,
): ProductPromptBuildMetadata {
  const record = expectRecord(value, path);
  const metadata: ProductPromptBuildMetadata = {
    prompt_hash: expectString(record.prompt_hash, `${path}.prompt_hash`, {
      nonEmpty: true,
    }),
    stable_prefix_hash: expectString(
      record.stable_prefix_hash,
      `${path}.stable_prefix_hash`,
      { nonEmpty: true },
    ),
    workspace_fingerprint: expectString(
      record.workspace_fingerprint,
      `${path}.workspace_fingerprint`,
      { nonEmpty: true },
    ),
    tool_signature: expectString(record.tool_signature, `${path}.tool_signature`, {
      nonEmpty: true,
    }),
    token_estimate: expectInteger(record.token_estimate, `${path}.token_estimate`, {
      min: 0,
    }),
    included_history_messages: expectInteger(
      record.included_history_messages,
      `${path}.included_history_messages`,
      { min: 0 },
    ),
    dropped_history_messages: expectInteger(
      record.dropped_history_messages,
      `${path}.dropped_history_messages`,
      { min: 0 },
    ),
  };
  assignOptional(
    metadata,
    "prompt_cache_key",
    optionalString(record, "prompt_cache_key", path, { nonEmpty: true }),
  );
  assignOptional(metadata, "pruning", parsePromptBuildPruning(record, path));
  return metadata;
}

function parseAgentProfileIdentity(
  value: unknown,
  path: string,
): AgentProfileIdentity {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "selector",
      "agent_id",
      "display_name",
      "definition_version",
      "manifest_hash",
      "package_hash",
      "profile_hash",
      "instruction_bundle_hash",
      "procedures",
    ],
    path,
  );
  const selectorRecord = expectRecord(record.selector, `${path}.selector`);
  expectOnlyKeys(selectorRecord, ["source", "agent_id"], `${path}.selector`);
  const identity: AgentProfileIdentity = {
    selector: {
      source: expectEnum(
        selectorRecord.source,
        ["builtin", "workspace"] as const,
        `${path}.selector.source`,
      ),
      agent_id: expectString(
        selectorRecord.agent_id,
        `${path}.selector.agent_id`,
        {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        },
      ),
    },
    agent_id: expectString(record.agent_id, `${path}.agent_id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    display_name: expectString(record.display_name, `${path}.display_name`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    definition_version: expectString(
      record.definition_version,
      `${path}.definition_version`,
      {
        nonEmpty: true,
        maxBytes: MAX_PRODUCT_TEXT_BYTES,
        noControlCharacters: true,
      },
    ),
    manifest_hash: expectString(record.manifest_hash, `${path}.manifest_hash`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    package_hash: expectString(record.package_hash, `${path}.package_hash`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    profile_hash: expectString(record.profile_hash, `${path}.profile_hash`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  };
  assignOptional(
    identity,
    "instruction_bundle_hash",
    optionalString(record, "instruction_bundle_hash", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  if (record.procedures !== undefined && record.procedures !== null) {
    identity.procedures = expectArray(
      record.procedures,
      `${path}.procedures`,
      parseProcedureReference,
    );
  }
  return identity;
}

function parseAgentDiagnostic(value: unknown, path: string): AgentDiagnostic {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["code", "subject", "message"], path);
  return {
    code: expectString(record.code, `${path}.code`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    subject: expectString(record.subject, `${path}.subject`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    message: expectString(record.message, `${path}.message`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  };
}

export function parseStreamEvent(
  value: unknown,
  path = "stream event",
): ProductStreamEvent {
  const record = expectRecord(value, path);
  const type = expectEnum(record.type, STREAM_EVENT_NAMES, `${path}.type`);
  switch (type) {
    case "run_started":
      return {
        type,
        run_id: expectId(record.run_id, `${path}.run_id`),
        job_id: expectId(record.job_id, `${path}.job_id`),
        user_message: expectString(record.user_message, `${path}.user_message`),
      };
    case "agent_profile_activated": {
      const event: Extract<
        ProductStreamEvent,
        { type: "agent_profile_activated" }
      > = {
        type,
        identity: parseAgentProfileIdentity(record.identity, `${path}.identity`),
        resumed_from_snapshot: expectBoolean(
          record.resumed_from_snapshot,
          `${path}.resumed_from_snapshot`,
        ),
      };
      if (record.diagnostics !== undefined && record.diagnostics !== null) {
        event.diagnostics = expectArray(
          record.diagnostics,
          `${path}.diagnostics`,
          parseAgentDiagnostic,
        );
      }
      return event;
    }
    case "workspace_instructions_resolved":
      return {
        type,
        bundle_hash: expectString(record.bundle_hash, `${path}.bundle_hash`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        }),
        layer_count: expectInteger(record.layer_count, `${path}.layer_count`, {
          min: 0,
        }),
        rejected_count: expectInteger(
          record.rejected_count,
          `${path}.rejected_count`,
          { min: 0 },
        ),
        truncated: expectBoolean(record.truncated, `${path}.truncated`),
      };
    case "instruction_overlay_applied": {
      const event: Extract<
        ProductStreamEvent,
        { type: "instruction_overlay_applied" }
      > = {
        type,
        target_path: expectString(record.target_path, `${path}.target_path`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_PATH_BYTES,
          noControlCharacters: true,
        }),
        scope: expectString(record.scope, `${path}.scope`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_PATH_BYTES,
          noControlCharacters: true,
        }),
        source_path: expectString(record.source_path, `${path}.source_path`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_PATH_BYTES,
          noControlCharacters: true,
        }),
        content_hash: expectString(record.content_hash, `${path}.content_hash`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        }),
        boundary: expectString(record.boundary, `${path}.boundary`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        }),
      };
      if (record.call_id !== undefined && record.call_id !== null) {
        event.call_id = expectId(record.call_id, `${path}.call_id`);
      }
      return event;
    }
    case "procedures_selected": {
      const event: Extract<
        ProductStreamEvent,
        { type: "procedures_selected" }
      > = {
        type,
        profile_hash: expectString(
          record.profile_hash,
          `${path}.profile_hash`,
          {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
            noControlCharacters: true,
          },
        ),
        considered_count: expectInteger(
          record.considered_count,
          `${path}.considered_count`,
          { min: 0 },
        ),
        excluded_count: expectInteger(
          record.excluded_count,
          `${path}.excluded_count`,
          { min: 0 },
        ),
      };
      if (record.selected !== undefined && record.selected !== null) {
        event.selected = expectArray(
          record.selected,
          `${path}.selected`,
          parseProcedureReference,
        );
      }
      return event;
    }
    case "procedure_hydrated":
      {
        const event: Extract<ProductStreamEvent, { type: "procedure_hydrated" }> = {
        type,
        reference: parseProcedureReference(
          record.reference,
          `${path}.reference`,
        ),
        truncated: expectBoolean(record.truncated, `${path}.truncated`),
        dropped_bytes: expectInteger(
          record.dropped_bytes,
          `${path}.dropped_bytes`,
          { min: 0 },
        ),
        };
        if (record.step_id !== undefined && record.step_id !== null) {
          event.step_id = expectString(record.step_id, `${path}.step_id`, { nonEmpty: true });
        }
        if (record.hydration_hash !== undefined && record.hydration_hash !== null) {
          event.hydration_hash = expectString(record.hydration_hash, `${path}.hydration_hash`, { nonEmpty: true });
        }
        return event;
      }
    case "procedure_applied":
      return {
        type,
        application: parseProcedureApplication(record.application, `${path}.application`),
      };
    case "procedure_deviation":
      return {
        type,
        record_id: expectString(record.record_id, `${path}.record_id`, { nonEmpty: true }),
        deviation: parseProcedureDeviation(record.deviation, `${path}.deviation`),
      };
    case "llm_chunk":
      return { type, delta: expectString(record.delta, `${path}.delta`) };
    case "model_status":
      return {
        type,
        status: expectString(record.status, `${path}.status`),
        message: expectString(record.message, `${path}.message`),
      };
    case "provider_retry":
      return {
        type,
        attempt: expectInteger(record.attempt, `${path}.attempt`, { min: 1 }),
        max_attempts: expectInteger(
          record.max_attempts,
          `${path}.max_attempts`,
          { min: 1 },
        ),
        delay_ms: expectInteger(record.delay_ms, `${path}.delay_ms`, { min: 0 }),
        reason: expectString(record.reason, `${path}.reason`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        }),
        phase: expectString(record.phase, `${path}.phase`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        }),
      };
    case "llm_message": {
      const event: Extract<ProductStreamEvent, { type: "llm_message" }> = {
        type,
        full: expectString(record.full, `${path}.full`),
        usage: parseUsage(record.usage, `${path}.usage`),
      };
      if (record.tool_calls !== undefined && record.tool_calls !== null) {
        event.tool_calls = expectArray(
          record.tool_calls,
          `${path}.tool_calls`,
          parseToolCallRef,
        );
      }
      event.aborted = optionalBoolean(record, "aborted", path);
      return event;
    }
    case "tool_call_started": {
      const event: Extract<ProductStreamEvent, { type: "tool_call_started" }> = {
        type,
        call_id: expectId(record.call_id, `${path}.call_id`),
        name: expectString(record.name, `${path}.name`, { nonEmpty: true }),
        args: record.args,
      };
      const toolUseId = optionalNullableString(record, "tool_use_id", path);
      if (toolUseId !== undefined) {
        event.tool_use_id = toolUseId;
      }
      return event;
    }
    case "tool_call_approval_needed":
      return {
        type,
        call_id: expectId(record.call_id, `${path}.call_id`),
        name: expectString(record.name, `${path}.name`, { nonEmpty: true }),
        args: record.args,
        reason: expectString(record.reason, `${path}.reason`),
      };
    case "tool_call_completed":
      return {
        type,
        call_id: expectId(record.call_id, `${path}.call_id`),
        result: parseToolResult(record.result, `${path}.result`),
      };
    case "tool_artifact_stored":
      return {
        type,
        call_id: expectId(record.call_id, `${path}.call_id`),
        artifact: parseToolArtifactRef(record.artifact, `${path}.artifact`),
      };
    case "tool_artifact_rejected":
      return {
        type,
        call_id: expectId(record.call_id, `${path}.call_id`),
        block_ordinal: expectInteger(
          record.block_ordinal,
          `${path}.block_ordinal`,
          { min: 0 },
        ),
        reason: expectString(record.reason, `${path}.reason`, {
          nonEmpty: true,
        }),
        observed_bytes: expectInteger(
          record.observed_bytes,
          `${path}.observed_bytes`,
          { min: 0 },
        ),
      };
    case "mcp_server_degraded":
      return {
        type,
        server_config_id: expectString(
          record.server_config_id,
          `${path}.server_config_id`,
          {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
            noControlCharacters: true,
          },
        ),
        required: expectBoolean(record.required, `${path}.required`),
        failure_code: expectString(record.failure_code, `${path}.failure_code`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        }),
      };
    case "mcp_capabilities_refreshed": {
      const names = (key: "added" | "removed" | "changed") =>
        expectArray(
          record[key],
          `${path}.${key}`,
          (value, itemPath) =>
            expectString(value, itemPath, {
              nonEmpty: true,
              maxBytes: MAX_PRODUCT_TEXT_BYTES,
              noControlCharacters: true,
            }),
          128,
        );
      return {
        type,
        server_config_id: expectString(
          record.server_config_id,
          `${path}.server_config_id`,
          {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
            noControlCharacters: true,
          },
        ),
        snapshot_id: expectString(record.snapshot_id, `${path}.snapshot_id`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
          noControlCharacters: true,
        }),
        added: names("added"),
        removed: names("removed"),
        changed: names("changed"),
      };
    }
    case "tool_call_failed":
      return {
        type,
        call_id: expectId(record.call_id, `${path}.call_id`),
        error: parseToolError(record.error, `${path}.error`),
        metadata: parseToolExecutionMetadata(
          record.metadata,
          `${path}.metadata`,
        ),
      };
    case "input_needed":
      return {
        type,
        input_id: expectId(record.input_id, `${path}.input_id`),
        prompt: expectString(record.prompt, `${path}.prompt`),
      };
    case "plan_created": {
      const event: Extract<ProductStreamEvent, { type: "plan_created" }> = {
        type,
        plan: parseTaskPlan(record.plan, `${path}.plan`),
      };
      assignOptional(event, "plan_id", optionalString(record, "plan_id", path));
      assignOptional(
        event,
        "plan_revision_id",
        optionalString(record, "plan_revision_id", path),
      );
      assignOptional(
        event,
        "revision",
        optionalInteger(record, "revision", path, { min: 0 }),
      );
      if (record.plan_revision !== undefined && record.plan_revision !== null) {
        event.plan_revision = parsePlanRevision(
          record.plan_revision,
          `${path}.plan_revision`,
        );
      }
      return event;
    }
    case "plan_step_started": {
      const event: Extract<ProductStreamEvent, { type: "plan_step_started" }> = {
        type,
        step: parsePlanStep(record.step, `${path}.step`),
        index: expectInteger(record.index, `${path}.index`, { min: 0 }),
      };
      assignOptional(event, "plan_id", optionalString(record, "plan_id", path));
      assignOptional(
        event,
        "plan_revision_id",
        optionalString(record, "plan_revision_id", path),
      );
      assignOptional(event, "step_id", optionalString(record, "step_id", path));
      assignOptional(
        event,
        "attempt",
        optionalInteger(record, "attempt", path, { min: 0 }),
      );
      assignOptional(
        event,
        "started_at",
        optionalString(record, "started_at", path),
      );
      if (record.budget !== undefined && record.budget !== null) {
        event.budget = parseExecutionBudgetSnapshot(
          record.budget,
          `${path}.budget`,
        );
      }
      return event;
    }
    case "step_result":
      return {
        type,
        record: parseStepRecord(record.record, `${path}.record`),
      };
    case "plan_decision":
      return {
        type,
        record: parsePlanDecisionRecord(record.record, `${path}.record`),
      };
    case "plan_revised":
      return {
        type,
        plan: parseTaskPlan(record.plan, `${path}.plan`),
        revision: parsePlanRevision(record.revision, `${path}.revision`),
      };
    case "execution_strategy_selected":
      return {
        type,
        policy: parseExecutionPolicy(record.policy, `${path}.policy`),
      };
    case "execution_budget_updated":
      return {
        type,
        phase: expectEnum(record.phase, EXECUTION_PHASES, `${path}.phase`),
        snapshot: parseExecutionBudgetSnapshot(
          record.snapshot,
          `${path}.snapshot`,
        ),
      };
    case "execution_degraded":
      return {
        type,
        record: parseExecutionDegradation(record.record, `${path}.record`),
      };
    case "finalization_started":
    case "finalization_completed":
      return {
        type,
        record: parseFinalizationRecord(record.record, `${path}.record`),
      };
    case "prompt_compacted": {
      const event: Extract<ProductStreamEvent, { type: "prompt_compacted" }> = {
        type,
        state: parsePromptCompactionState(record.state, `${path}.state`),
      };
      const summary = optionalNullableString(record, "summary", path);
      if (summary !== undefined) {
        event.summary = summary;
      }
      return event;
    }
    case "memory_flushed":
      return {
        type,
        notes: parseStringArray(record.notes, `${path}.notes`),
      };
    case "prompt_built":
      return {
        type,
        metadata: parsePromptBuildMetadata(record.metadata, `${path}.metadata`),
      };
    case "run_completed": {
      const event: Extract<ProductStreamEvent, { type: "run_completed" }> = {
        type,
        reason: expectString(record.reason, `${path}.reason`, { nonEmpty: true }),
      };
      const output = optionalNullableString(record, "output", path);
      if (output !== undefined) {
        event.output = output;
      }
      return event;
    }
    case "steer_accepted":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
        content: expectString(record.content, `${path}.content`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
        }),
      };
    case "steer_applied":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
      };
    case "steer_dropped":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
        reason: expectString(record.reason, `${path}.reason`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
        }),
      };
    case "followup_queued":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
        content: expectString(record.content, `${path}.content`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
        }),
      };
    case "followup_dequeued":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
      };
    case "followup_abandoned":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
        reason: expectString(record.reason, `${path}.reason`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
        }),
      };
    case "message_queued":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
        content: expectString(record.content, `${path}.content`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
        }),
      };
    case "message_intervention_requested":
    case "message_applied_current_run":
    case "message_claimed_successor":
    case "message_revoked":
      return { type, id: expectId(record.id, `${path}.id`) };
    case "message_needs_attention":
      return {
        type,
        id: expectId(record.id, `${path}.id`),
        reason: expectString(record.reason, `${path}.reason`, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
        }),
      };
  }
}

function parseJobStreamEvent(
  value: unknown,
  path: string,
): ProductJobStreamEvent {
  const record = expectRecord(value, path);
  return {
    seq: expectInteger(record.seq, `${path}.seq`, { min: 1 }),
    event: parseStreamEvent(record.event, `${path}.event`),
  };
}

function parseProductTranscriptPartialReason(
  value: unknown,
  path: string,
): ProductTranscriptPartialReason {
  const record = expectRecord(value, path);
  const reason: ProductTranscriptPartialReason = {
    code: expectEnum(
      record.code,
      PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES,
      `${path}.code`,
    ),
  };
  assignOptional(
    reason,
    "run_ordinal",
    optionalInteger(record, "run_ordinal", path, { min: 1 }),
  );
  assignOptional(
    reason,
    "run_id",
    optionalString(record, "run_id", path, { nonEmpty: true }),
  );
  assignOptional(
    reason,
    "expected_seq",
    optionalInteger(record, "expected_seq", path, { min: 0 }),
  );
  assignOptional(
    reason,
    "observed_seq",
    optionalInteger(record, "observed_seq", path, { min: 0 }),
  );
  return reason;
}

function parseProductTranscriptFallback(
  value: unknown,
  path: string,
): ProductTranscriptFallback {
  const record = expectRecord(value, path);
  const fallback: ProductTranscriptFallback = {
    source: expectEnum(record.source, ["report"] as const, `${path}.source`),
    status: expectString(record.status, `${path}.status`, { nonEmpty: true }),
  };
  assignOptional(
    fallback,
    "summary",
    optionalString(record, "summary", path),
  );
  return fallback;
}

function parseProductTranscriptRunSegment(
  value: unknown,
  path: string,
): ProductTranscriptRunSegment {
  const record = expectRecord(value, path);
  const segment: ProductTranscriptRunSegment = {
    binding: parseProductSessionRunBinding(record.binding, `${path}.binding`),
    inherited: expectBoolean(record.inherited, `${path}.inherited`),
    run_status: expectRunStatus(record.run_status, `${path}.run_status`),
    observed_through_seq: expectInteger(
      record.observed_through_seq,
      `${path}.observed_through_seq`,
      { min: 0 },
    ),
    last_event_seq: expectInteger(
      record.last_event_seq,
      `${path}.last_event_seq`,
      { min: 0 },
    ),
    events: expectArray(record.events, `${path}.events`, parseJobStreamEvent),
  };
  assignOptional(
    segment,
    "source_product_session_id",
    optionalId(record, "source_product_session_id", path),
  );
  assignOptional(
    segment,
    "answer_aborted",
    optionalBoolean(record, "answer_aborted", path),
  );
  if (record.fallback !== undefined && record.fallback !== null) {
    segment.fallback = parseProductTranscriptFallback(
      record.fallback,
      `${path}.fallback`,
    );
  }
  return segment;
}

export function parseProductTranscriptResponse(
  value: unknown,
): ProductTranscriptResponse {
  const path = "product transcript response";
  const record = expectRecord(value, path);
  const response: ProductTranscriptResponse = {
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
    workspace_id: expectId(record.workspace_id, `${path}.workspace_id`),
    status: expectEnum(
      record.status,
      PRODUCT_TRANSCRIPT_STATUSES,
      `${path}.status`,
    ),
    partial_reasons: expectArray(
      record.partial_reasons,
      `${path}.partial_reasons`,
      parseProductTranscriptPartialReason,
    ),
    segments: expectArray(
      record.segments,
      `${path}.segments`,
      parseProductTranscriptRunSegment,
    ),
  };
  assignOptional(
    response,
    "next_before_ordinal",
    optionalInteger(record, "next_before_ordinal", path, { min: 1 }),
  );
  assignOptional(response, "has_more", optionalBoolean(record, "has_more", path));
  if (response.next_before_ordinal !== undefined && response.has_more === undefined) {
    schemaError(
      `${path}.has_more`,
      "present on a response that carries next_before_ordinal",
    );
  }
  if (
    response.has_more !== undefined &&
    response.has_more !== (response.next_before_ordinal !== undefined)
  ) {
    schemaError(
      `${path}.has_more`,
      "equal to whether next_before_ordinal is present",
    );
  }
  if (
    (response.status === "complete" && response.partial_reasons.length !== 0) ||
    (response.status === "partial" && response.partial_reasons.length === 0)
  ) {
    schemaError(
      `${path}.status`,
      "consistent with whether partial_reasons is empty",
    );
  }
  // A cursor page starts wherever the cursor landed, so the ordinals below its
  // first segment wait for an older page instead of a partial reason. Legacy
  // responses still account for every leading gap.
  const cursorPage = response.has_more !== undefined;

  let previousOrdinal = 0;
  for (const [segmentIndex, segment] of response.segments.entries()) {
    const segmentPath = `${path}.segments[${segmentIndex}]`;
    if (
      !segment.inherited &&
      segment.binding.product_session_id !== response.product_session_id
    ) {
      schemaError(
        `${segmentPath}.binding.product_session_id`,
        "the transcript product_session_id",
      );
    }
    if (
      segment.inherited &&
      (!segment.source_product_session_id ||
        segment.binding.product_session_id !== segment.source_product_session_id)
    ) {
      schemaError(
        `${segmentPath}.source_product_session_id`,
        "the source product session id for inherited history",
      );
    }
    if (!segment.inherited && segment.source_product_session_id !== undefined) {
      schemaError(
        `${segmentPath}.source_product_session_id`,
        "absent for local session history",
      );
    }
    if (segment.binding.ordinal <= previousOrdinal) {
      schemaError(
        `${segmentPath}.binding.ordinal`,
        "strictly greater than the previous segment ordinal",
      );
    }
    for (
      let missingOrdinal =
        segmentIndex === 0 && cursorPage ? segment.binding.ordinal : previousOrdinal + 1;
      missingOrdinal < segment.binding.ordinal;
      missingOrdinal += 1
    ) {
      if (
        !response.partial_reasons.some(
          (reason) => reason.run_ordinal === missingOrdinal,
        )
      ) {
        schemaError(
          `${segmentPath}.binding.ordinal`,
          `covered by a partial reason for missing ordinal ${missingOrdinal}`,
        );
      }
    }
    let expectedSeq = 1;
    for (const [eventIndex, event] of segment.events.entries()) {
      if (
        event.seq !== expectedSeq &&
        !unknownEventTypeCoversStep(
          response.partial_reasons,
          segment.binding.ordinal,
          expectedSeq,
          event.seq,
          segment.binding.runtime_run_id,
        )
      ) {
        schemaError(
          `${segmentPath}.events[${eventIndex}].seq`,
          `the contiguous sequence ${expectedSeq}`,
        );
      }
      expectedSeq = event.seq + 1;
    }
    const observedThrough =
      segment.events[segment.events.length - 1]?.seq ?? 0;
    if (segment.observed_through_seq !== observedThrough) {
      schemaError(
        `${segmentPath}.observed_through_seq`,
        "the sequence of the final returned event",
      );
    }
    if (segment.observed_through_seq > segment.last_event_seq) {
      schemaError(
        `${segmentPath}.observed_through_seq`,
        "at most last_event_seq",
      );
    }
    if (
      segment.observed_through_seq < segment.last_event_seq &&
      !response.partial_reasons.some(
        (reason) =>
          partialReasonNamesRun(
            reason,
            segment.binding.ordinal,
            segment.binding.runtime_run_id,
          ) && PRODUCT_TRANSCRIPT_POSITIONLESS_GAP_REASON_CODES.has(reason.code),
      ) &&
      // A withheld-row reason claims exact durable positions, so it may only
      // explain a tail it actually reaches. Without this bound a response with
      // `observed_through_seq: 1` and `last_event_seq: 100` would parse clean
      // when a single withheld row at 2 is reported, and the client could never
      // notice that the server under-reported the rest of the gap. The server
      // stays consistent with this rule: a residue beyond the withheld range is
      // reported as `missing_event_range`.
      !unknownEventTypeCoversStep(
        response.partial_reasons,
        segment.binding.ordinal,
        segment.observed_through_seq + 1,
        segment.last_event_seq + 1,
        segment.binding.runtime_run_id,
      )
    ) {
      schemaError(
        `${segmentPath}.last_event_seq`,
        "equal to observed_through_seq or covered by a typed partial reason for this run ordinal",
      );
    }
    previousOrdinal = segment.binding.ordinal;
  }
  return response;
}

function parseM1WorkspaceImport(value: unknown, path: string): M1WorkspaceImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["source_id", "root", "kind", "display_name", "pinned", "last_opened_at"],
    path,
  );
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    root: expectString(record.root, `${path}.root`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
      noControlCharacters: true,
    }),
    kind: expectEnum(record.kind, PRODUCT_WORKSPACE_KINDS, `${path}.kind`),
    display_name: expectString(record.display_name, `${path}.display_name`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    pinned: expectBoolean(record.pinned, `${path}.pinned`),
    last_opened_at: expectRfc3339Timestamp(
      record.last_opened_at,
      `${path}.last_opened_at`,
    ),
  };
}

function parseM1SessionImport(value: unknown, path: string): M1SessionImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source_id",
      "source_workspace_id",
      "title",
      "created_at",
      "updated_at",
      "legacy_active_job_id",
      "legacy_active_run_id",
      "legacy_resumed_from_run_id",
      "legacy_has_durable_turn",
    ],
    path,
  );
  const session: M1SessionImport = {
    source_id: expectId(record.source_id, `${path}.source_id`),
    source_workspace_id: expectId(
      record.source_workspace_id,
      `${path}.source_workspace_id`,
    ),
    title: expectString(record.title, `${path}.title`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    created_at: expectRfc3339Timestamp(
      record.created_at,
      `${path}.created_at`,
    ),
    updated_at: expectRfc3339Timestamp(
      record.updated_at,
      `${path}.updated_at`,
    ),
  };
  assignOptional(
    session,
    "legacy_active_job_id",
    optionalString(record, "legacy_active_job_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    session,
    "legacy_active_run_id",
    optionalString(record, "legacy_active_run_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    session,
    "legacy_resumed_from_run_id",
    optionalString(record, "legacy_resumed_from_run_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    session,
    "legacy_has_durable_turn",
    optionalBoolean(record, "legacy_has_durable_turn", path),
  );
  return session;
}

function parseM1ProviderProfileImport(
  value: unknown,
  path: string,
): M1ProviderProfileImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source_id",
      "label",
      "provider_type",
      "api_base",
      "api_key_env",
      "default_model",
      "updated_at",
    ],
    path,
  );
  const providerType = expectEnum(
    record.provider_type,
    PRODUCT_PROVIDER_TYPES,
    `${path}.provider_type`,
  );
  const apiBase = expectString(record.api_base, `${path}.api_base`, {
    maxBytes: MAX_PRODUCT_API_BASE_BYTES,
  });
  const apiKeyEnv = optionalString(record, "api_key_env", path, {
    nonEmpty: true,
    maxBytes: 256,
  });
  assertSafeProductProviderConfiguration(
    providerType,
    apiBase,
    apiKeyEnv,
    path,
  );
  const profile: M1ProviderProfileImport = {
    source_id: expectId(record.source_id, `${path}.source_id`),
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    provider_type: providerType,
    api_base: apiBase,
    updated_at: expectRfc3339Timestamp(
      record.updated_at,
      `${path}.updated_at`,
    ),
  };
  assignOptional(profile, "api_key_env", apiKeyEnv);
  assignOptional(
    profile,
    "default_model",
    optionalString(record, "default_model", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  return profile;
}

function parseM1ProviderSelectionImport(
  value: unknown,
  path: string,
): M1ProviderSelectionImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["source_profile_id", "model", "approval", "max_steps"],
    path,
  );
  const selection: M1ProviderSelectionImport = {
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
    approval: expectEnum(
      record.approval,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.approval`,
    ),
    max_steps: expectInteger(record.max_steps, `${path}.max_steps`, {
      min: 1,
      max: 4_096,
    }),
  };
  assignOptional(
    selection,
    "source_profile_id",
    optionalString(record, "source_profile_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  return selection;
}

function parseM1SafePreferencesImport(
  value: unknown,
  path: string,
): M1SafePreferencesImport {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "theme",
      "source_active_workspace_id",
      "source_active_session_id",
      "provider_selection",
    ],
    path,
  );
  const preferences: M1SafePreferencesImport = {};
  if (record.theme !== undefined && record.theme !== null) {
    preferences.theme = expectEnum(
      record.theme,
      PRODUCT_THEME_PREFERENCES,
      `${path}.theme`,
    );
  }
  assignOptional(
    preferences,
    "source_active_workspace_id",
    optionalString(record, "source_active_workspace_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  assignOptional(
    preferences,
    "source_active_session_id",
    optionalString(record, "source_active_session_id", path, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControlCharacters: true,
    }),
  );
  if (
    record.provider_selection !== undefined &&
    record.provider_selection !== null
  ) {
    preferences.provider_selection = parseM1ProviderSelectionImport(
      record.provider_selection,
      `${path}.provider_selection`,
    );
  }
  return preferences;
}

export function parseM1BrowserMigrationRequest(
  value: unknown,
  path = "M1 browser migration request",
): M1BrowserMigrationRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source",
      "source_schema_version",
      "idempotency_key",
      "workspaces",
      "sessions",
      "provider_profiles",
      "safe_preferences",
    ],
    path,
  );
  return {
    source: expectEnum(
      record.source,
      ["web_m1_local_storage"] as const,
      `${path}.source`,
    ),
    source_schema_version: expectInteger(
      record.source_schema_version,
      `${path}.source_schema_version`,
      { min: 1 },
    ),
    idempotency_key: expectM1MigrationIdempotencyKey(
      record.idempotency_key,
      `${path}.idempotency_key`,
    ),
    workspaces: expectArray(
      record.workspaces,
      `${path}.workspaces`,
      parseM1WorkspaceImport,
      MAX_PRODUCT_WORKSPACES,
    ),
    sessions: expectArray(
      record.sessions,
      `${path}.sessions`,
      parseM1SessionImport,
      MAX_PRODUCT_SESSIONS,
    ),
    provider_profiles: expectArray(
      record.provider_profiles,
      `${path}.provider_profiles`,
      parseM1ProviderProfileImport,
      MAX_PRODUCT_PROVIDER_PROFILES,
    ),
    safe_preferences: parseM1SafePreferencesImport(
      record.safe_preferences,
      `${path}.safe_preferences`,
    ),
  };
}

function parseM1WorkspaceIdMapping(
  value: unknown,
  path: string,
): M1WorkspaceIdMapping {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["source_id", "workspace_id"], path);
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    workspace_id: expectId(record.workspace_id, `${path}.workspace_id`),
  };
}

function parseM1SessionIdMapping(
  value: unknown,
  path: string,
): M1SessionIdMapping {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["source_id", "product_session_id"], path);
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    product_session_id: expectId(
      record.product_session_id,
      `${path}.product_session_id`,
    ),
  };
}

function parseM1ProviderProfileIdMapping(
  value: unknown,
  path: string,
): M1ProviderProfileIdMapping {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["source_id", "provider_profile_id"], path);
  return {
    source_id: expectId(record.source_id, `${path}.source_id`),
    provider_profile_id: expectId(
      record.provider_profile_id,
      `${path}.provider_profile_id`,
    ),
  };
}

function parseM1MigrationIssue(value: unknown, path: string): M1MigrationIssue {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["code", "entity", "source_id"], path);
  const issue: M1MigrationIssue = {
    code: expectEnum(
      record.code,
      M1_MIGRATION_ISSUE_CODES,
      `${path}.code`,
    ),
    entity: expectString(record.entity, `${path}.entity`, { nonEmpty: true }),
  };
  assignOptional(
    issue,
    "source_id",
    optionalString(record, "source_id", path, { nonEmpty: true }),
  );
  return issue;
}

export function parseM1BrowserMigrationResponse(
  value: unknown,
): M1BrowserMigrationResponse {
  const path = "M1 browser migration response";
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "source_schema_version",
      "idempotency_key",
      "receipt_id",
      "disposition",
      "workspace_mappings",
      "session_mappings",
      "provider_profile_mappings",
      "issues",
      "applied_at",
    ],
    path,
  );
  return {
    source_schema_version: expectInteger(
      record.source_schema_version,
      `${path}.source_schema_version`,
      { min: 1 },
    ),
    idempotency_key: expectString(
      record.idempotency_key,
      `${path}.idempotency_key`,
      { nonEmpty: true, maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES },
    ),
    receipt_id: expectId(record.receipt_id, `${path}.receipt_id`),
    disposition: expectEnum(
      record.disposition,
      M1_MIGRATION_DISPOSITIONS,
      `${path}.disposition`,
    ),
    workspace_mappings: expectArray(
      record.workspace_mappings,
      `${path}.workspace_mappings`,
      parseM1WorkspaceIdMapping,
      MAX_PRODUCT_WORKSPACES,
    ),
    session_mappings: expectArray(
      record.session_mappings,
      `${path}.session_mappings`,
      parseM1SessionIdMapping,
      MAX_PRODUCT_SESSIONS,
    ),
    provider_profile_mappings: expectArray(
      record.provider_profile_mappings,
      `${path}.provider_profile_mappings`,
      parseM1ProviderProfileIdMapping,
      MAX_PRODUCT_PROVIDER_PROFILES,
    ),
    issues: expectArray(record.issues, `${path}.issues`, parseM1MigrationIssue),
    applied_at: expectString(record.applied_at, `${path}.applied_at`, {
      nonEmpty: true,
    }),
  };
}

export function parseApiErrorResponse(value: unknown): ApiErrorResponse | null {
  if (!isRecord(value)) {
    return null;
  }
  if (typeof value.code !== "string" || typeof value.error !== "string") {
    return null;
  }
  return { code: value.code, error: value.error };
}

export function parseProductFilesResponse(value: unknown): ProductFilesResponse {
  const record = expectRecord(value, "product files response");
  const response: ProductFilesResponse = {
    workspace_id: expectId(record.workspace_id, "product files response.workspace_id"),
    prefix: expectString(record.prefix ?? "", "product files response.prefix", {
      maxBytes: MAX_PRODUCT_PATH_BYTES,
    }),
    entries: expectArray(
      record.entries,
      "product files response.entries",
      (item, path) => {
        const entry = expectRecord(item, path);
        const parsed: ProductFileEntry = {
          path: expectString(entry.path, `${path}.path`, {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_PATH_BYTES,
          }),
          kind: expectEnum(entry.kind, PRODUCT_FILE_KINDS, `${path}.kind`),
          size: expectInteger(entry.size, `${path}.size`, { min: 0 }),
        };
        assignOptional(
          parsed,
          "modified",
          optionalString(entry, "modified", path, { nonEmpty: true }),
        );
        return parsed;
      },
      500,
    ),
    truncated: expectBoolean(record.truncated, "product files response.truncated"),
    scan_limit_reached: expectBoolean(
      record.scan_limit_reached ?? false,
      "product files response.scan_limit_reached",
    ),
  };
  assignOptional(
    response,
    "next_cursor",
    optionalString(record, "next_cursor", "product files response", {
      nonEmpty: true,
    }),
  );
  return response;
}

export function parseProductFileContentEnvelope(
  value: unknown,
): ProductFileContentEnvelope {
  const record = expectRecord(value, "product file content");
  const envelope: ProductFileContentEnvelope = {
    path: expectString(record.path, "product file content.path", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
    }),
    mime: expectString(record.mime, "product file content.mime", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    size: expectInteger(record.size, "product file content.size", { min: 0 }),
    truncated: expectBoolean(record.truncated, "product file content.truncated"),
    preview_allowed: expectBoolean(
      record.preview_allowed ?? false,
      "product file content.preview_allowed",
    ),
  };
  assignOptional(
    envelope,
    "text",
    optionalString(record, "text", "product file content"),
  );
  assignOptional(
    envelope,
    "encoding",
    optionalString(record, "encoding", "product file content", {
      nonEmpty: true,
    }),
  );
  if (record.image !== undefined && record.image !== null) {
    envelope.image = parseProductImageMetadata(
      record.image,
      "product file content.image",
    );
  }
  assignOptional(
    envelope,
    "validation_error",
    optionalString(record, "validation_error", "product file content", {
      nonEmpty: true,
    }),
  );
  return envelope;
}

function parseProductImageMetadata(
  value: unknown,
  path: string,
): ProductImageMetadata {
  const record = expectRecord(value, path);
  return {
    width: expectInteger(record.width, `${path}.width`, { min: 1 }),
    height: expectInteger(record.height, `${path}.height`, { min: 1 }),
    format: expectString(record.format, `${path}.format`, {
      nonEmpty: true,
      maxBytes: 16,
    }),
  };
}

export function parseProductArtifactsResponse(
  value: unknown,
): ProductArtifactsResponse {
  const record = expectRecord(value, "product artifacts response");
  return {
    session_id: expectId(
      record.session_id,
      "product artifacts response.session_id",
    ),
    artifacts: expectArray(
      record.artifacts,
      "product artifacts response.artifacts",
      (item, path) => {
        const artifact = expectRecord(item, path);
        const parsed: ProductArtifactView = {
          artifact_id: expectString(artifact.artifact_id, `${path}.artifact_id`, {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
          }),
          safe_name: expectString(artifact.safe_name, `${path}.safe_name`, {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
          }),
          mime: expectString(artifact.mime, `${path}.mime`, {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
          }),
          source_run_id: expectString(
            artifact.source_run_id,
            `${path}.source_run_id`,
            { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES },
          ),
          source_kind: expectEnum(
            artifact.source_kind,
            PRODUCT_ARTIFACT_SOURCE_KINDS,
            `${path}.source_kind`,
          ),
          availability: expectEnum(
            artifact.availability ?? "available",
            PRODUCT_ARTIFACT_AVAILABILITIES,
            `${path}.availability`,
          ),
          preview_kind: expectEnum(
            artifact.preview_kind ?? "download_only",
            PRODUCT_ARTIFACT_PREVIEW_KINDS,
            `${path}.preview_kind`,
          ),
        };
        assignOptional(
          parsed,
          "size",
          optionalInteger(artifact, "size", path, { min: 0 }),
        );
        assignOptional(
          parsed,
          "sha256",
          optionalString(artifact, "sha256", path, {
            nonEmpty: true,
            maxBytes: 64,
          }),
        );
        if (artifact.image !== undefined && artifact.image !== null) {
          parsed.image = parseProductImageMetadata(
            artifact.image,
            `${path}.image`,
          );
        }
        assignOptional(
          parsed,
          "validation_error",
          optionalString(artifact, "validation_error", path, {
            nonEmpty: true,
          }),
        );
        return parsed;
      },
      2048,
    ),
    partial_reasons: expectArray(
      record.partial_reasons,
      "product artifacts response.partial_reasons",
      (item, path) =>
        expectString(item, path, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
        }),
      512,
    ),
  };
}

export function parseProductArtifactContentEnvelope(
  value: unknown,
): ProductArtifactContentEnvelope {
  const record = expectRecord(value, "product artifact content");
  const envelope: ProductArtifactContentEnvelope = {
    artifact_id: expectString(
      record.artifact_id,
      "product artifact content.artifact_id",
      { nonEmpty: true, maxBytes: 64 },
    ),
    safe_name: expectString(
      record.safe_name,
      "product artifact content.safe_name",
      { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES },
    ),
    mime: expectString(record.mime, "product artifact content.mime", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    size: expectInteger(record.size, "product artifact content.size", {
      min: 0,
    }),
    truncated: expectBoolean(
      record.truncated,
      "product artifact content.truncated",
    ),
    preview_allowed: expectBoolean(
      record.preview_allowed ?? false,
      "product artifact content.preview_allowed",
    ),
  };
  assignOptional(
    envelope,
    "text",
    optionalString(record, "text", "product artifact content"),
  );
  assignOptional(
    envelope,
    "encoding",
    optionalString(record, "encoding", "product artifact content", {
      nonEmpty: true,
    }),
  );
  if (record.image !== undefined && record.image !== null) {
    envelope.image = parseProductImageMetadata(
      record.image,
      "product artifact content.image",
    );
  }
  assignOptional(
    envelope,
    "validation_error",
    optionalString(record, "validation_error", "product artifact content", {
      nonEmpty: true,
    }),
  );
  return envelope;
}

export function parseProductAuthorizationsResponse(
  value: unknown,
): ProductAuthorizationsResponse {
  const record = expectRecord(value, "product authorizations response");
  return {
    session_id: expectId(
      record.session_id,
      "product authorizations response.session_id",
    ),
    truncated: expectBoolean(
      record.truncated ?? false,
      "product authorizations response.truncated",
    ),
    authorizations: expectArray(
      record.authorizations,
      "product authorizations response.authorizations",
      (item, path) => {
        const entry = expectRecord(item, path);
        const parsed: ProductAuthorizationRecord = {
          call_id: expectId(entry.call_id, `${path}.call_id`),
          job_id: expectId(entry.job_id, `${path}.job_id`),
          run_id: expectId(entry.run_id, `${path}.run_id`),
          tool: expectString(entry.tool, `${path}.tool`, {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
          }),
          args: entry.args ?? null,
          reason: expectString(entry.reason ?? "", `${path}.reason`, {
            maxBytes: MAX_PRODUCT_CONTROL_CONTENT_BYTES,
          }),
          status: expectString(entry.status, `${path}.status`, {
            nonEmpty: true,
            maxBytes: 32,
          }),
          requested_at: expectString(entry.requested_at, `${path}.requested_at`, {
            nonEmpty: true,
            maxBytes: 64,
          }),
          updated_at: expectString(entry.updated_at, `${path}.updated_at`, {
            nonEmpty: true,
            maxBytes: 64,
          }),
        };
        assignOptional(
          parsed,
          "decided_via",
          optionalString(entry, "decided_via", path, {
            maxBytes: MAX_PRODUCT_TEXT_BYTES,
          }),
        );
        if (entry.outcome !== undefined && entry.outcome !== null) {
          const outcome = expectRecord(entry.outcome, `${path}.outcome`);
          parsed.outcome = {
            event: expectString(outcome.event, `${path}.outcome.event`, {
              nonEmpty: true,
              maxBytes: 64,
            }),
            seq: optionalNumber(outcome, "seq", `${path}.outcome`) ?? 0,
          };
        }
        return parsed;
      },
    ),
  };
}

export function parseCreateProductPreviewRequest(
  value: unknown,
): CreateProductPreviewRequest {
  const record = expectRecord(value, "create product preview request");
  return {
    path: expectString(record.path, "create product preview request.path", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
    }),
  };
}

export function parseProductPreviewSession(
  value: unknown,
): ProductPreviewSession {
  const record = expectRecord(value, "product preview session");
  return {
    preview_id: expectId(record.preview_id, "product preview session.preview_id"),
    workspace_id: expectId(
      record.workspace_id,
      "product preview session.workspace_id",
    ),
    entry: expectString(record.entry, "product preview session.entry", {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_PATH_BYTES,
    }),
    url: expectString(record.url, "product preview session.url", {
      nonEmpty: true,
      maxBytes: 2_048,
    }),
    created_at: expectString(
      record.created_at,
      "product preview session.created_at",
      { nonEmpty: true, maxBytes: 64 },
    ),
    expires_at: expectString(
      record.expires_at,
      "product preview session.expires_at",
      { nonEmpty: true, maxBytes: 64 },
    ),
  };
}

export function parseProductSessionDiffResponse(
  value: unknown,
): ProductSessionDiffResponse {
  const record = expectRecord(value, "product session diff response");
  return {
    session_id: expectId(
      record.session_id,
      "product session diff response.session_id",
    ),
    scope: expectString(record.scope, "product session diff response.scope", {
      nonEmpty: true,
      maxBytes: 16,
    }),
    entries: expectArray(
      record.entries,
      "product session diff response.entries",
      (item, path) => {
        const entry = expectRecord(item, path);
        const parsed: ProductDiffEntry = {
          path: expectString(entry.path, `${path}.path`, {
            nonEmpty: true,
            maxBytes: MAX_PRODUCT_PATH_BYTES,
          }),
          op: expectEnum(entry.op, PRODUCT_DIFF_OPS, `${path}.op`),
          source: expectEnum(
            entry.source ?? "run",
            PRODUCT_DIFF_SOURCES,
            `${path}.source`,
          ),
          binary: expectBoolean(entry.binary ?? false, `${path}.binary`),
          truncated: expectBoolean(entry.truncated ?? false, `${path}.truncated`),
          reconstructable: expectBoolean(
            entry.reconstructable ?? false,
            `${path}.reconstructable`,
          ),
        };
        assignOptional(
          parsed,
          "source_run_id",
          optionalString(entry, "source_run_id", path, { nonEmpty: true }),
        );
        assignOptional(
          parsed,
          "diff",
          optionalString(entry, "diff", path),
        );
        return parsed;
      },
      4096,
    ),
    partial_reasons: expectArray(
      record.partial_reasons,
      "product session diff response.partial_reasons",
      (item, path) =>
        expectString(item, path, {
          nonEmpty: true,
          maxBytes: MAX_PRODUCT_TEXT_BYTES,
        }),
      512,
    ),
  };
}
