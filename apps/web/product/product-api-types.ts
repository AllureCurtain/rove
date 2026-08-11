import {
  ARTIFACT_VALIDATION_STATES,
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
  created_at: string;
  updated_at: string;
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
}

export interface ProductProviderProfile {
  id: ProductProviderProfileId;
  label: string;
  provider_type: ProductProviderType;
  api_base: string;
  api_key_env?: string;
  default_model?: string;
  created_at: string;
  updated_at: string;
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
}

export type UpdateProductProviderProfileRequest =
  CreateProductProviderProfileRequest;

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
}

export interface ProductProviderProfilesResponse {
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
] as const;
export type ProductTranscriptPartialReasonCode =
  (typeof PRODUCT_TRANSCRIPT_PARTIAL_REASON_CODES)[number];

const PRODUCT_TRANSCRIPT_EVENT_GAP_REASON_CODES = new Set<
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
  fallback?: ProductTranscriptFallback;
}

export interface ProductTranscriptResponse {
  product_session_id: ProductSessionId;
  workspace_id: ProductWorkspaceId;
  status: ProductTranscriptStatus;
  partial_reasons: ProductTranscriptPartialReason[];
  segments: ProductTranscriptRunSegment[];
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

export interface ProductPromptBuildMetadata {
  prompt_hash: string;
  stable_prefix_hash: string;
  workspace_fingerprint: string;
  tool_signature: string;
  token_estimate: number;
  included_history_messages: number;
  dropped_history_messages: number;
  prompt_cache_key?: string;
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
}

export interface ProductControlsResponse {
  controls: ProductControl[];
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
  return session;
}

export function parseProductFork(
  value: unknown,
  path = "product fork",
): ProductFork {
  const record = expectRecord(value, path);
  return {
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
    created_at: expectString(record.created_at, `${path}.created_at`, {
      nonEmpty: true,
    }),
    updated_at: expectString(record.updated_at, `${path}.updated_at`, {
      nonEmpty: true,
    }),
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
  expectOnlyKeys(record, ["fork_at_run_id", "title", "idempotency_key"], path);
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
  expectOnlyKeys(record, ["content", "idempotency_key"], path);
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
  return request;
}

export function parseProductProviderProfileRequest(
  value: unknown,
  path = "product provider profile request",
): CreateProductProviderProfileRequest {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["label", "provider_type", "api_base", "api_key_env", "default_model"],
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
  return request;
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
  return {
    sessions: expectArray(
      record.sessions,
      "product sessions response.sessions",
      parseProductSession,
      MAX_PRODUCT_SESSIONS,
    ),
  };
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
  // Content blocks, structured content, protocol metadata, effects, and
  // diagnostics are passed through as validated-shape records rather than
  // re-modelled here: the server is the authority on their contents, and the
  // UI reads them through narrow accessors.
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
  if (
    (response.status === "complete" && response.partial_reasons.length !== 0) ||
    (response.status === "partial" && response.partial_reasons.length === 0)
  ) {
    schemaError(
      `${path}.status`,
      "consistent with whether partial_reasons is empty",
    );
  }

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
      let missingOrdinal = previousOrdinal + 1;
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
    for (const [eventIndex, event] of segment.events.entries()) {
      const expectedSeq = eventIndex + 1;
      if (event.seq !== expectedSeq) {
        schemaError(
          `${segmentPath}.events[${eventIndex}].seq`,
          `the contiguous sequence ${expectedSeq}`,
        );
      }
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
          reason.run_ordinal === segment.binding.ordinal &&
          PRODUCT_TRANSCRIPT_EVENT_GAP_REASON_CODES.has(reason.code),
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
