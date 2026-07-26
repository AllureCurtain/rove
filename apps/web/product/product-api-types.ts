import {
  STREAM_EVENT_NAMES,
  type ExecutionBudgetUsage,
  type PlanDecision,
  type PlanDecisionRecord,
  type PlanRevision,
  type PlanStep,
  type PromptCompactionState,
  type RunStatus,
  type StepRecord,
  type StreamEvent,
  type TaskPlan,
  type ToolCallRef,
  type ToolError,
  type ToolMutation,
  type ToolMutationOperation,
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

export type ProductWorkspaceId = string;
export type ProductSessionId = string;
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

export interface ProductPreferences {
  schema_version: number;
  theme: ProductThemePreference;
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
  theme: ProductThemePreference;
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
  "migration_idempotency_conflict",
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
  return session;
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
  const preferences: ProductPreferences = {
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
      "theme",
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
  return result;
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
        "skipped",
        "budget_exhausted",
        "cancelled",
        "interrupted",
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
  return stepRecord;
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
    if (segment.binding.product_session_id !== response.product_session_id) {
      schemaError(
        `${segmentPath}.binding.product_session_id`,
        "the transcript product_session_id",
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
