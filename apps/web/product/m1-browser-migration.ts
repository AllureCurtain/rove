import {
  M1_BROWSER_MIGRATION_STATE_KEY,
  M1_BROWSER_SOURCE_SCHEMA_VERSION,
  M1_BROWSER_STORAGE_KEYS,
} from "./m1-storage-keys";
import {
  MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES,
  MAX_PRODUCT_API_BASE_BYTES,
  MAX_PRODUCT_PATH_BYTES,
  MAX_PRODUCT_PROVIDER_PROFILES,
  MAX_PRODUCT_SESSIONS,
  MAX_PRODUCT_TEXT_BYTES,
  MAX_PRODUCT_WORKSPACES,
  PRODUCT_APPROVAL_PREFERENCES,
  PRODUCT_PROVIDER_TYPES,
  PRODUCT_THEME_PREFERENCES,
  PRODUCT_WORKSPACE_KINDS,
  ProductApiSchemaError,
  assertSafeProductProviderConfiguration,
  parseM1BrowserMigrationRequest,
  parseM1BrowserMigrationResponse,
  type M1BrowserMigrationRequest,
  type M1BrowserMigrationResponse,
  type M1MigrationIssue,
  type M1MigrationIssueCode,
  type M1ProviderProfileImport,
  type M1ProviderSelectionImport,
  type M1SafePreferencesImport,
  type M1SessionImport,
  type M1WorkspaceImport,
  type ProductProviderType,
  type ProductThemePreference,
} from "./product-api-types";
import {
  ProductApiError,
  createProductApiClient,
  type ProductApiClient,
} from "./product-client";

export interface MigrationStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export const M1_BROWSER_MIGRATION_LOCK_NAME =
  "rove.product.migration.web-m1.v1";
export const DEFAULT_M1_BROWSER_MIGRATION_TIMEOUT_MS = 30_000;
export const MAX_M1_BROWSER_MIGRATION_TIMEOUT_MS = 300_000;

export interface M1BrowserMigrationLock {
  runExclusive<T>(
    name: string,
    operation: () => Promise<T>,
  ): Promise<T>;
}

export interface M1BrowserMigrationDependencies {
  storage: MigrationStorage;
  fetch?: typeof globalThis.fetch;
  idGenerator?: () => string;
  now?: () => string;
  apiPrefix?: string;
  client?: ProductApiClient;
  requestTimeoutMs?: number;
  /** `null` explicitly selects the fail-closed no-lock path. */
  lock?: M1BrowserMigrationLock | null;
}

export interface PendingM1BrowserMigrationState {
  status: "pending";
  source_schema_version: typeof M1_BROWSER_SOURCE_SCHEMA_VERSION;
  idempotency_key: string;
  request: M1BrowserMigrationRequest;
  request_body: string;
  created_at: string;
}

export interface CompleteM1BrowserMigrationState {
  status: "complete";
  source_schema_version: typeof M1_BROWSER_SOURCE_SCHEMA_VERSION;
  idempotency_key: string;
  acknowledgement: M1BrowserMigrationResponse;
  completed_at: string;
}

export type M1BrowserMigrationState =
  | PendingM1BrowserMigrationState
  | CompleteM1BrowserMigrationState;

export interface M1BrowserMigrationFailure {
  code:
    | "invalid_legacy_state"
    | "invalid_migration_state"
    | "storage_write_failed"
    | "request_failed"
    | "request_rejected"
    | "invalid_acknowledgement"
    | "lock_unavailable"
    | "lock_failed";
  message: string;
}

export type M1BrowserMigrationRunResult =
  | {
      status: "not_needed";
    }
  | {
      status: "complete";
      state: CompleteM1BrowserMigrationState;
      reused: boolean;
    }
  | {
      status: "pending";
      state: PendingM1BrowserMigrationState;
      failure: M1BrowserMigrationFailure;
    }
  | {
      status: "rejected";
      failure: M1BrowserMigrationFailure & { code: "request_rejected" };
    }
  | {
      status: "superseded";
      acknowledgement: M1BrowserMigrationResponse;
    }
  | {
      status: "blocked";
      failure: M1BrowserMigrationFailure;
    };

export class M1BrowserMigrationDataError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "M1BrowserMigrationDataError";
  }
}

type UnknownRecord = Record<string, unknown>;

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function dataError(path: string, expectation: string): never {
  throw new M1BrowserMigrationDataError(`${path} must be ${expectation}`);
}

function expectRecord(value: unknown, path: string): UnknownRecord {
  if (!isRecord(value)) {
    return dataError(path, "an object");
  }
  return value;
}

function expectString(
  value: unknown,
  path: string,
  options: { nonEmpty?: boolean; maxBytes?: number } = {},
): string {
  if (typeof value !== "string") {
    return dataError(path, "a string");
  }
  if (options.nonEmpty && value.trim().length === 0) {
    return dataError(path, "a non-empty string");
  }
  if (
    options.maxBytes !== undefined &&
    new TextEncoder().encode(value).length > options.maxBytes
  ) {
    return dataError(path, `at most ${options.maxBytes} UTF-8 bytes`);
  }
  return value;
}

function optionalString(
  record: UnknownRecord,
  key: string,
  path: string,
  options: { nonEmpty?: boolean; maxBytes?: number } = {},
): string | undefined {
  const value = record[key];
  if (value === undefined || value === null) {
    return undefined;
  }
  return expectString(value, `${path}.${key}`, options);
}

function expectBoolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    return dataError(path, "a boolean");
  }
  return value;
}

function expectInteger(
  value: unknown,
  path: string,
  options: { min?: number; max?: number } = {},
): number {
  if (!Number.isSafeInteger(value)) {
    return dataError(path, "a safe integer");
  }
  const numberValue = Number(value);
  if (options.min !== undefined && numberValue < options.min) {
    return dataError(path, `at least ${options.min}`);
  }
  if (options.max !== undefined && numberValue > options.max) {
    return dataError(path, `at most ${options.max}`);
  }
  return numberValue;
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
    return dataError(path, `one of ${values.join(", ")}`);
  }
  return value;
}

function parseStoredJson(
  storage: MigrationStorage,
  key: string,
  label: string,
): unknown | undefined {
  const raw = storage.getItem(key);
  if (raw === null) {
    return undefined;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    return parsed;
  } catch {
    return dataError(label, "valid JSON");
  }
}

function parseLegacyArray<T>(
  value: unknown | undefined,
  path: string,
  maxLength: number,
  parseItem: (item: unknown, itemPath: string) => T,
): T[] {
  if (value === undefined) {
    return [];
  }
  if (!Array.isArray(value)) {
    return dataError(path, "an array");
  }
  if (value.length > maxLength) {
    return dataError(path, `an array with at most ${maxLength} items`);
  }
  return value.map((item, index) => parseItem(item, `${path}[${index}]`));
}

function assertUniqueSourceIds(
  entries: readonly { source_id: string }[],
  path: string,
): void {
  const seen = new Set<string>();
  for (const entry of entries) {
    if (seen.has(entry.source_id)) {
      dataError(path, "entries with unique ids");
    }
    seen.add(entry.source_id);
  }
}

function isAbsoluteWorkspacePath(value: string): boolean {
  return /^([a-zA-Z]:[\\/]|\\\\|\/)/.test(value.trim());
}

function parseLegacyWorkspace(value: unknown, path: string): M1WorkspaceImport {
  const record = expectRecord(value, path);
  const root = expectString(record.rootPath, `${path}.rootPath`, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_PATH_BYTES,
  });
  if (!isAbsoluteWorkspacePath(root)) {
    dataError(`${path}.rootPath`, "an absolute local path");
  }
  return {
    source_id: expectString(record.id, `${path}.id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    root,
    kind: expectEnum(
      record.kind,
      PRODUCT_WORKSPACE_KINDS,
      `${path}.kind`,
    ),
    display_name: expectString(record.displayName, `${path}.displayName`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    pinned: expectBoolean(record.pinned, `${path}.pinned`),
    last_opened_at: expectString(
      record.lastOpenedAt,
      `${path}.lastOpenedAt`,
      { nonEmpty: true },
    ),
  };
}

function parseLegacySession(value: unknown, path: string): M1SessionImport {
  const record = expectRecord(value, path);
  const session: M1SessionImport = {
    source_id: expectString(record.id, `${path}.id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    source_workspace_id: expectString(
      record.workspaceId,
      `${path}.workspaceId`,
      { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES },
    ),
    title: expectString(record.title, `${path}.title`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    created_at: expectString(record.createdAt, `${path}.createdAt`, {
      nonEmpty: true,
    }),
    updated_at: expectString(record.updatedAt, `${path}.updatedAt`, {
      nonEmpty: true,
    }),
  };

  const activeJobId = optionalString(record, "activeJobId", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
  });
  const activeRunId = optionalString(record, "activeRunId", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
  });
  const resumedFromRunId = optionalString(record, "resumedFromRunId", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
  });
  if (activeJobId !== undefined) {
    session.legacy_active_job_id = activeJobId;
  }
  if (activeRunId !== undefined) {
    session.legacy_active_run_id = activeRunId;
  }
  if (resumedFromRunId !== undefined) {
    session.legacy_resumed_from_run_id = resumedFromRunId;
  }
  if (record.hasDurableTurn !== undefined && record.hasDurableTurn !== null) {
    session.legacy_has_durable_turn = expectBoolean(
      record.hasDurableTurn,
      `${path}.hasDurableTurn`,
    );
  }
  return session;
}

function parseLegacyProviderProfile(
  value: unknown,
  path: string,
): M1ProviderProfileImport {
  const record = expectRecord(value, path);
  const providerType: ProductProviderType = expectEnum(
    record.providerType,
    PRODUCT_PROVIDER_TYPES,
    `${path}.providerType`,
  );
  const legacyApiBase = expectString(record.apiBase, `${path}.apiBase`, {
    maxBytes: MAX_PRODUCT_API_BASE_BYTES,
  });
  const apiBase =
    providerType === "fake" &&
    (legacyApiBase.trim() === "" ||
      legacyApiBase.trim().toLowerCase() === "local")
      ? ""
      : legacyApiBase;
  const apiKeyEnv = optionalString(record, "apiKeyEnv", path, {
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
    source_id: expectString(record.id, `${path}.id`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    label: expectString(record.label, `${path}.label`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    provider_type: providerType,
    api_base: apiBase,
    updated_at: expectString(record.updatedAt, `${path}.updatedAt`, {
      nonEmpty: true,
    }),
  };
  if (apiKeyEnv !== undefined) {
    profile.api_key_env = apiKeyEnv;
  }
  const defaultModel = optionalString(record, "defaultModel", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
  });
  if (defaultModel !== undefined) {
    profile.default_model = defaultModel;
  }
  return profile;
}

interface LegacyActiveSelection {
  workspaceId?: string;
  sessionId?: string;
}

function parseLegacyActiveSelection(
  value: unknown | undefined,
): LegacyActiveSelection {
  if (value === undefined) {
    return {};
  }
  const path = "M1 active selection";
  const record = expectRecord(value, path);
  const selection: LegacyActiveSelection = {};
  const workspaceId = optionalString(record, "workspaceId", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
  });
  const sessionId = optionalString(record, "sessionId", path, {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
  });
  if (workspaceId !== undefined) {
    selection.workspaceId = workspaceId;
  }
  if (sessionId !== undefined) {
    selection.sessionId = sessionId;
  }
  return selection;
}

function parseLegacyProviderSelection(
  value: unknown | undefined,
): M1ProviderSelectionImport | undefined {
  if (value === undefined) {
    return undefined;
  }
  const path = "M1 provider selection";
  const record = expectRecord(value, path);
  const mode = expectEnum(record.mode, ["default", "profile"] as const, `${path}.mode`);
  const selection: M1ProviderSelectionImport = {
    model: expectString(record.model, `${path}.model`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
    }),
    approval: expectEnum(
      record.approval,
      PRODUCT_APPROVAL_PREFERENCES,
      `${path}.approval`,
    ),
    max_steps: expectInteger(record.maxSteps, `${path}.maxSteps`, {
      min: 1,
      max: 4_096,
    }),
  };
  if (mode === "profile") {
    selection.source_profile_id = expectString(
      record.profileId,
      `${path}.profileId`,
      { nonEmpty: true, maxBytes: MAX_PRODUCT_TEXT_BYTES },
    );
  }
  if (
    mode === "default" &&
    selection.model === "fake" &&
    selection.approval === "ask" &&
    selection.max_steps === 8
  ) {
    return undefined;
  }
  return selection;
}

function parseLegacyTheme(
  storage: MigrationStorage,
): ProductThemePreference | undefined {
  const raw = storage.getItem(M1_BROWSER_STORAGE_KEYS.theme);
  if (raw === null) {
    return undefined;
  }
  if (isEnumValue(raw, PRODUCT_THEME_PREFERENCES)) {
    // M1 writes its default light theme on every shell mount, so this value
    // cannot prove an explicit user preference. Preserve the durable setting.
    return raw === "light" ? undefined : raw;
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    const theme = expectEnum(parsed, PRODUCT_THEME_PREFERENCES, "M1 theme");
    return theme === "light" ? undefined : theme;
  } catch (error) {
    if (error instanceof M1BrowserMigrationDataError) {
      throw error;
    }
    return dataError("M1 theme", "light, dark, or system");
  }
}

const SAFE_SECRET_REFERENCE_FIELDS = new Set([
  "api_key_env",
  "idempotency_key",
]);
const FORBIDDEN_SECRET_FIELDS = new Set([
  "apikey",
  "key",
  "token",
  "authorization",
  "auth",
  "secret",
  "password",
  "bearer",
]);

function assertNoRawSecretFields(value: unknown, path = "migration request"): void {
  if (Array.isArray(value)) {
    value.forEach((entry, index) =>
      assertNoRawSecretFields(entry, `${path}[${index}]`),
    );
    return;
  }
  if (!isRecord(value)) {
    return;
  }
  for (const [key, entry] of Object.entries(value)) {
    if (!SAFE_SECRET_REFERENCE_FIELDS.has(key)) {
      const normalized = key.replace(/[^a-zA-Z]/g, "").toLowerCase();
      if (FORBIDDEN_SECRET_FIELDS.has(normalized)) {
        dataError(`${path}.${key}`, "absent from the sanitized request");
      }
    }
    assertNoRawSecretFields(entry, `${path}.${key}`);
  }
}

interface M1BrowserMigrationPayload {
  workspaces: M1WorkspaceImport[];
  sessions: M1SessionImport[];
  provider_profiles: M1ProviderProfileImport[];
  safe_preferences: M1SafePreferencesImport;
}

function buildM1BrowserMigrationPayload(
  storage: MigrationStorage,
): M1BrowserMigrationPayload {
  const workspaces = parseLegacyArray(
    parseStoredJson(
      storage,
      M1_BROWSER_STORAGE_KEYS.workspaces,
      "M1 workspaces",
    ),
    "M1 workspaces",
    MAX_PRODUCT_WORKSPACES,
    parseLegacyWorkspace,
  );
  const sessions = parseLegacyArray(
    parseStoredJson(storage, M1_BROWSER_STORAGE_KEYS.sessions, "M1 sessions"),
    "M1 sessions",
    MAX_PRODUCT_SESSIONS,
    parseLegacySession,
  );
  const active = parseLegacyActiveSelection(
    parseStoredJson(
      storage,
      M1_BROWSER_STORAGE_KEYS.active,
      "M1 active selection",
    ),
  );
  const providerProfiles = parseLegacyArray(
    parseStoredJson(
      storage,
      M1_BROWSER_STORAGE_KEYS.providerProfiles,
      "M1 provider profiles",
    ),
    "M1 provider profiles",
    MAX_PRODUCT_PROVIDER_PROFILES,
    parseLegacyProviderProfile,
  );
  const providerSelection = parseLegacyProviderSelection(
    parseStoredJson(
      storage,
      M1_BROWSER_STORAGE_KEYS.providerSelection,
      "M1 provider selection",
    ),
  );

  assertUniqueSourceIds(workspaces, "M1 workspaces");
  assertUniqueSourceIds(sessions, "M1 sessions");
  assertUniqueSourceIds(providerProfiles, "M1 provider profiles");

  const safePreferences: M1SafePreferencesImport = {};
  const theme = parseLegacyTheme(storage);
  if (theme !== undefined) {
    safePreferences.theme = theme;
  }
  if (active.workspaceId !== undefined) {
    safePreferences.source_active_workspace_id = active.workspaceId;
  }
  if (active.sessionId !== undefined) {
    safePreferences.source_active_session_id = active.sessionId;
  }
  if (providerSelection !== undefined) {
    safePreferences.provider_selection = providerSelection;
  }

  return {
    workspaces,
    sessions,
    provider_profiles: providerProfiles,
    safe_preferences: safePreferences,
  };
}

function createM1BrowserMigrationRequest(
  payload: M1BrowserMigrationPayload,
  idempotencyKey: string,
): M1BrowserMigrationRequest {
  const request: M1BrowserMigrationRequest = {
    source: "web_m1_local_storage",
    source_schema_version: M1_BROWSER_SOURCE_SCHEMA_VERSION,
    idempotency_key: expectString(idempotencyKey, "migration idempotency key", {
      nonEmpty: true,
      maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES,
    }),
    ...payload,
  };
  assertNoRawSecretFields(request);
  return parseM1BrowserMigrationRequest(request);
}

export function buildM1BrowserMigrationRequest(
  storage: MigrationStorage,
  idempotencyKey: string,
): M1BrowserMigrationRequest {
  return createM1BrowserMigrationRequest(
    buildM1BrowserMigrationPayload(storage),
    idempotencyKey,
  );
}

function parseMigrationState(raw: string): M1BrowserMigrationState {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch {
    return dataError("browser migration state", "valid JSON");
  }
  const path = "browser migration state";
  const record = expectRecord(value, path);
  const status = expectEnum(record.status, ["pending", "complete"] as const, `${path}.status`);
  const schemaVersion = expectInteger(
    record.source_schema_version,
    `${path}.source_schema_version`,
    { min: 1 },
  );
  if (schemaVersion !== M1_BROWSER_SOURCE_SCHEMA_VERSION) {
    dataError(
      `${path}.source_schema_version`,
      `exactly ${M1_BROWSER_SOURCE_SCHEMA_VERSION}`,
    );
  }
  const idempotencyKey = expectString(
    record.idempotency_key,
    `${path}.idempotency_key`,
    { nonEmpty: true, maxBytes: MAX_MIGRATION_IDEMPOTENCY_KEY_BYTES },
  );

  if (status === "pending") {
    const request = parseM1BrowserMigrationRequest(record.request);
    const requestBody = expectString(record.request_body, `${path}.request_body`, {
      nonEmpty: true,
    });
    if (
      request.source_schema_version !== schemaVersion ||
      request.idempotency_key !== idempotencyKey ||
      JSON.stringify(request) !== requestBody
    ) {
      dataError(path, "a self-consistent exact pending request");
    }
    let parsedBody: unknown;
    try {
      parsedBody = JSON.parse(requestBody);
    } catch {
      return dataError(`${path}.request_body`, "valid JSON");
    }
    if (JSON.stringify(parseM1BrowserMigrationRequest(parsedBody)) !== requestBody) {
      dataError(`${path}.request_body`, "the canonical persisted request body");
    }
    return {
      status,
      source_schema_version: M1_BROWSER_SOURCE_SCHEMA_VERSION,
      idempotency_key: idempotencyKey,
      request,
      request_body: requestBody,
      created_at: expectString(record.created_at, `${path}.created_at`, {
        nonEmpty: true,
      }),
    };
  }

  const acknowledgement = parseM1BrowserMigrationResponse(
    record.acknowledgement,
  );
  if (
    acknowledgement.source_schema_version !== schemaVersion ||
    acknowledgement.idempotency_key !== idempotencyKey ||
    acknowledgement.receipt_id.trim().length === 0
  ) {
    dataError(path, "a self-consistent complete acknowledgement");
  }
  return {
    status,
    source_schema_version: M1_BROWSER_SOURCE_SCHEMA_VERSION,
    idempotency_key: idempotencyKey,
    acknowledgement,
    completed_at: expectString(record.completed_at, `${path}.completed_at`, {
      nonEmpty: true,
    }),
  };
}

export function readM1BrowserMigrationState(
  storage: MigrationStorage,
): M1BrowserMigrationState | null {
  const raw = storage.getItem(M1_BROWSER_MIGRATION_STATE_KEY);
  return raw === null ? null : parseMigrationState(raw);
}

function createPendingState(
  request: M1BrowserMigrationRequest,
  createdAt: string,
): PendingM1BrowserMigrationState {
  const requestBody = JSON.stringify(request);
  return {
    status: "pending",
    source_schema_version: M1_BROWSER_SOURCE_SCHEMA_VERSION,
    idempotency_key: request.idempotency_key,
    request,
    request_body: requestBody,
    created_at: createdAt,
  };
}

function validateMappingSourceIds(
  label: string,
  mappings: readonly { source_id: string }[],
  knownSourceIds: ReadonlySet<string>,
): void {
  const seen = new Set<string>();
  for (const mapping of mappings) {
    if (!knownSourceIds.has(mapping.source_id)) {
      throw new ProductApiSchemaError(
        `${label} contains an unknown source_id`,
      );
    }
    if (seen.has(mapping.source_id)) {
      throw new ProductApiSchemaError(
        `${label} contains a duplicate source_id`,
      );
    }
    seen.add(mapping.source_id);
  }
}

function normalizedIssueEntity(value: string): string {
  return value.trim().toLowerCase().replace(/[\s-]+/g, "_");
}

function issueCoversSource(
  issue: M1MigrationIssue,
  sourceId: string,
  entities: ReadonlySet<string>,
  codes: ReadonlySet<M1MigrationIssueCode>,
): boolean {
  return (
    issue.source_id === sourceId &&
    entities.has(normalizedIssueEntity(issue.entity)) &&
    codes.has(issue.code)
  );
}

function validateEntityCoverage(
  label: string,
  sourceIds: readonly string[],
  mappings: readonly { source_id: string }[],
  issues: readonly M1MigrationIssue[],
  issueEntities: readonly string[],
  issueCodes: readonly M1MigrationIssueCode[],
): void {
  const mapped = new Set(mappings.map((mapping) => mapping.source_id));
  const entities = new Set(issueEntities);
  const codes = new Set(issueCodes);
  for (const sourceId of sourceIds) {
    if (mapped.has(sourceId)) {
      continue;
    }
    if (
      issues.some((issue) =>
        issueCoversSource(issue, sourceId, entities, codes),
      )
    ) {
      continue;
    }
    throw new ProductApiSchemaError(
      `${label} does not map or explicitly issue every request source_id`,
    );
  }
}

export function validateM1MigrationAcknowledgement(
  pending: PendingM1BrowserMigrationState,
  value: unknown,
): M1BrowserMigrationResponse {
  const acknowledgement = parseM1BrowserMigrationResponse(value);
  if (
    acknowledgement.source_schema_version !==
    M1_BROWSER_SOURCE_SCHEMA_VERSION
  ) {
    throw new ProductApiSchemaError(
      "M1 migration acknowledgement has the wrong source schema version",
    );
  }
  if (acknowledgement.idempotency_key !== pending.idempotency_key) {
    throw new ProductApiSchemaError(
      "M1 migration acknowledgement has a different idempotency key",
    );
  }
  if (!acknowledgement.receipt_id.trim()) {
    throw new ProductApiSchemaError(
      "M1 migration acknowledgement receipt must not be empty",
    );
  }

  validateMappingSourceIds(
    "workspace_mappings",
    acknowledgement.workspace_mappings,
    new Set(pending.request.workspaces.map((entry) => entry.source_id)),
  );
  validateMappingSourceIds(
    "session_mappings",
    acknowledgement.session_mappings,
    new Set(pending.request.sessions.map((entry) => entry.source_id)),
  );
  validateMappingSourceIds(
    "provider_profile_mappings",
    acknowledgement.provider_profile_mappings,
    new Set(pending.request.provider_profiles.map((entry) => entry.source_id)),
  );
  validateEntityCoverage(
    "workspace_mappings",
    pending.request.workspaces.map((entry) => entry.source_id),
    acknowledgement.workspace_mappings,
    acknowledgement.issues,
    ["workspace", "product_workspace"],
    ["invalid_workspace"],
  );
  validateEntityCoverage(
    "session_mappings",
    pending.request.sessions.map((entry) => entry.source_id),
    acknowledgement.session_mappings,
    acknowledgement.issues,
    ["session", "product_session"],
    [
      "missing_workspace",
      "invalid_runtime_hint",
      "ambiguous_runtime_binding",
      "runtime_binding_not_found",
    ],
  );
  validateEntityCoverage(
    "provider_profile_mappings",
    pending.request.provider_profiles.map((entry) => entry.source_id),
    acknowledgement.provider_profile_mappings,
    acknowledgement.issues,
    ["provider_profile", "product_provider_profile"],
    [],
  );
  const preferences = pending.request.safe_preferences;
  if (preferences.source_active_workspace_id !== undefined) {
    validateEntityCoverage(
      "active workspace preference",
      [preferences.source_active_workspace_id],
      acknowledgement.workspace_mappings,
      acknowledgement.issues,
      ["active_workspace"],
      ["invalid_preference_reference"],
    );
  }
  if (preferences.source_active_session_id !== undefined) {
    validateEntityCoverage(
      "active session preference",
      [preferences.source_active_session_id],
      acknowledgement.session_mappings,
      acknowledgement.issues,
      ["active_session"],
      ["invalid_preference_reference"],
    );
  }
  const sourceProfileId =
    preferences.provider_selection?.source_profile_id;
  if (sourceProfileId !== undefined) {
    validateEntityCoverage(
      "provider selection preference",
      [sourceProfileId],
      acknowledgement.provider_profile_mappings,
      acknowledgement.issues,
      ["provider_selection"],
      ["invalid_preference_reference"],
    );
  }
  return acknowledgement;
}

function defaultIdGenerator(): string {
  if (!globalThis.crypto?.randomUUID) {
    throw new Error("crypto.randomUUID is required for browser migration");
  }
  return `web-m1-${globalThis.crypto.randomUUID()}`;
}

function defaultNow(): string {
  return new Date().toISOString();
}

function webLocksMigrationLock(): M1BrowserMigrationLock | null {
  const lockManager = globalThis.navigator?.locks;
  if (!lockManager) {
    return null;
  }
  return {
    runExclusive<T>(name: string, operation: () => Promise<T>): Promise<T> {
      return lockManager.request(name, { mode: "exclusive" }, operation);
    },
  };
}

function blocked(
  code: M1BrowserMigrationFailure["code"],
  message: string,
): M1BrowserMigrationRunResult {
  return { status: "blocked", failure: { code, message } };
}

class M1BrowserMigrationTimeoutError extends Error {
  constructor() {
    super("M1 browser migration request timed out");
    this.name = "M1BrowserMigrationTimeoutError";
  }
}

function requestFailure(error: unknown): M1BrowserMigrationFailure {
  if (error instanceof M1BrowserMigrationTimeoutError) {
    return { code: "request_failed", message: error.message };
  }
  if (error instanceof ProductApiError) {
    return {
      code: "request_failed",
      message: `${error.code}: ${error.message}`,
    };
  }
  if (error instanceof ProductApiSchemaError) {
    return { code: "invalid_acknowledgement", message: error.message };
  }
  return {
    code: "request_failed",
    message: "M1 browser migration request failed before acknowledgement",
  };
}

function hasM1BrowserSourceState(storage: MigrationStorage): boolean {
  return Object.values(M1_BROWSER_STORAGE_KEYS).some(
    (key) => storage.getItem(key) !== null,
  );
}

function isM1BrowserMigrationPayloadEmpty(
  payload: M1BrowserMigrationPayload,
): boolean {
  return (
    payload.workspaces.length === 0 &&
    payload.sessions.length === 0 &&
    payload.provider_profiles.length === 0 &&
    Object.keys(payload.safe_preferences).length === 0
  );
}

function migrationRequestTimeoutMs(configured: number | undefined): number {
  const timeout = configured ?? DEFAULT_M1_BROWSER_MIGRATION_TIMEOUT_MS;
  if (
    !Number.isSafeInteger(timeout) ||
    timeout < 1 ||
    timeout > MAX_M1_BROWSER_MIGRATION_TIMEOUT_MS
  ) {
    throw new Error(
      `M1 browser migration timeout must be between 1 and ${MAX_M1_BROWSER_MIGRATION_TIMEOUT_MS} milliseconds`,
    );
  }
  return timeout;
}

async function migrateM1BrowserStateWithTimeout(
  client: ProductApiClient,
  pending: PendingM1BrowserMigrationState,
  timeoutMs: number,
): Promise<M1BrowserMigrationResponse> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => {
      reject(new M1BrowserMigrationTimeoutError());
      controller.abort();
    }, timeoutMs);
  });
  const request = Promise.resolve().then(() =>
    client.migrateM1BrowserState(
      {
        request: pending.request,
        body: pending.request_body,
      },
      { signal: controller.signal },
    ),
  );
  try {
    return await Promise.race([request, timeout]);
  } finally {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
  }
}

function pendingStateMatches(
  current: M1BrowserMigrationState | null,
  pending: PendingM1BrowserMigrationState,
): current is PendingM1BrowserMigrationState {
  return (
    current?.status === "pending" &&
    current.idempotency_key === pending.idempotency_key &&
    current.request_body === pending.request_body
  );
}

function deterministicRejection(
  error: unknown,
): error is ProductApiError {
  return (
    error instanceof ProductApiError &&
    (error.status === 400 || error.status === 409)
  );
}

function clearDeterministicallyRejectedPending(
  storage: MigrationStorage,
  pending: PendingM1BrowserMigrationState,
  error: ProductApiError,
): M1BrowserMigrationRunResult {
  const failure = {
    code: "request_rejected" as const,
    message: `${error.code}: ${error.message}`,
  };
  let current: M1BrowserMigrationState | null;
  try {
    current = readM1BrowserMigrationState(storage);
  } catch (stateError) {
    return {
      status: "pending",
      state: pending,
      failure: {
        code: "invalid_migration_state",
        message:
          stateError instanceof Error
            ? stateError.message
            : "pending migration state could not be verified after rejection",
      },
    };
  }
  if (current?.status === "complete") {
    return { status: "complete", state: current, reused: true };
  }
  if (!pendingStateMatches(current, pending)) {
    return blocked(
      "request_rejected",
      `${failure.message}; pending migration state was superseded`,
    );
  }
  try {
    storage.removeItem(M1_BROWSER_MIGRATION_STATE_KEY);
  } catch {
    try {
      storage.setItem(M1_BROWSER_MIGRATION_STATE_KEY, JSON.stringify(pending));
    } catch {
      // The pending result below exposes that durable storage is unavailable.
    }
    return {
      status: "pending",
      state: pending,
      failure: {
        code: "storage_write_failed",
        message: "migration was rejected but its pending state could not be cleared",
      },
    };
  }
  let cleared: M1BrowserMigrationState | null;
  try {
    cleared = readM1BrowserMigrationState(storage);
  } catch {
    try {
      storage.setItem(M1_BROWSER_MIGRATION_STATE_KEY, JSON.stringify(pending));
    } catch {
      // The typed failure below remains authoritative when storage is unusable.
    }
    return {
      status: "pending",
      state: pending,
      failure: {
        code: "storage_write_failed",
        message: "migration was rejected but clearing its pending state could not be verified",
      },
    };
  }
  if (pendingStateMatches(cleared, pending)) {
    return {
      status: "pending",
      state: pending,
      failure: {
        code: "storage_write_failed",
        message: "migration was rejected but its pending state was not cleared",
      },
    };
  }
  if (cleared?.status === "complete") {
    return { status: "complete", state: cleared, reused: true };
  }
  if (cleared !== null) {
    return blocked(
      "request_rejected",
      `${failure.message}; pending migration state was superseded while clearing`,
    );
  }
  return { status: "rejected", failure };
}

/**
 * Replay-safe one-shot migration. A valid pending record always wins over a
 * fresh legacy snapshot, and a stale tab never overwrites another tab's state.
 */
async function runM1BrowserMigrationCriticalSection(
  dependencies: M1BrowserMigrationDependencies,
): Promise<M1BrowserMigrationRunResult> {
  const { storage } = dependencies;
  let existing: M1BrowserMigrationState | null;
  try {
    existing = readM1BrowserMigrationState(storage);
  } catch (error) {
    return blocked(
      "invalid_migration_state",
      error instanceof Error
        ? error.message
        : "browser migration state is invalid",
    );
  }

  if (existing?.status === "complete") {
    return { status: "complete", state: existing, reused: true };
  }

  let pending = existing;
  if (pending === null) {
    const idGenerator = dependencies.idGenerator ?? defaultIdGenerator;
    const now = dependencies.now ?? defaultNow;
    let request: M1BrowserMigrationRequest;
    try {
      if (!hasM1BrowserSourceState(storage)) {
        return { status: "not_needed" };
      }
      const payload = buildM1BrowserMigrationPayload(storage);
      if (isM1BrowserMigrationPayloadEmpty(payload)) {
        return { status: "not_needed" };
      }
      request = createM1BrowserMigrationRequest(payload, idGenerator());
      pending = createPendingState(request, now());
      storage.setItem(
        M1_BROWSER_MIGRATION_STATE_KEY,
        JSON.stringify(pending),
      );
    } catch (error) {
      if (
        error instanceof M1BrowserMigrationDataError ||
        error instanceof ProductApiSchemaError
      ) {
        return blocked("invalid_legacy_state", error.message);
      }
      return blocked(
        "storage_write_failed",
        "unable to persist the pending M1 browser migration",
      );
    }

    // A concurrently-started tab may have replaced our candidate. The stored
    // pending body is the only body either tab is allowed to post from here.
    try {
      const claimed = readM1BrowserMigrationState(storage);
      if (claimed?.status === "complete") {
        return { status: "complete", state: claimed, reused: true };
      }
      if (claimed?.status !== "pending") {
        return blocked(
          "invalid_migration_state",
          "pending migration state disappeared before the request",
        );
      }
      pending = claimed;
    } catch (error) {
      return blocked(
        "invalid_migration_state",
        error instanceof Error
          ? error.message
          : "pending migration state is invalid",
      );
    }
  }

  const client =
    dependencies.client ??
    createProductApiClient({
      fetch: dependencies.fetch,
      apiPrefix: dependencies.apiPrefix,
    });

  let acknowledgement: M1BrowserMigrationResponse;
  try {
    const response = await migrateM1BrowserStateWithTimeout(
      client,
      pending,
      migrationRequestTimeoutMs(dependencies.requestTimeoutMs),
    );
    acknowledgement = validateM1MigrationAcknowledgement(pending, response);
  } catch (error) {
    if (deterministicRejection(error)) {
      return clearDeterministicallyRejectedPending(storage, pending, error);
    }
    return {
      status: "pending",
      state: pending,
      failure: requestFailure(error),
    };
  }

  let current: M1BrowserMigrationState | null;
  try {
    current = readM1BrowserMigrationState(storage);
  } catch {
    return {
      status: "superseded",
      acknowledgement,
    };
  }
  if (current?.status === "complete") {
    if (
      current.idempotency_key === pending.idempotency_key &&
      current.acknowledgement.receipt_id === acknowledgement.receipt_id
    ) {
      return { status: "complete", state: current, reused: true };
    }
    return { status: "superseded", acknowledgement };
  }
  if (
    current?.status !== "pending" ||
    current.idempotency_key !== pending.idempotency_key ||
    current.request_body !== pending.request_body
  ) {
    return { status: "superseded", acknowledgement };
  }

  const complete: CompleteM1BrowserMigrationState = {
    status: "complete",
    source_schema_version: M1_BROWSER_SOURCE_SCHEMA_VERSION,
    idempotency_key: pending.idempotency_key,
    acknowledgement,
    completed_at: (dependencies.now ?? defaultNow)(),
  };
  try {
    storage.setItem(M1_BROWSER_MIGRATION_STATE_KEY, JSON.stringify(complete));
    const committed = readM1BrowserMigrationState(storage);
    if (
      committed?.status !== "complete" ||
      committed.idempotency_key !== complete.idempotency_key ||
      committed.acknowledgement.receipt_id !== acknowledgement.receipt_id
    ) {
      return { status: "superseded", acknowledgement };
    }
    return { status: "complete", state: committed, reused: false };
  } catch {
    return {
      status: "pending",
      state: pending,
      failure: {
        code: "storage_write_failed",
        message: "server migration succeeded but the completion receipt was not stored",
      },
    };
  }
}

/**
 * The same-origin lock covers the complete migration transaction, including
 * the network wait. Without mutual exclusion no localStorage compare-and-set
 * exists, so the only safe fallback is to leave all state untouched.
 */
export async function runM1BrowserMigration(
  dependencies: M1BrowserMigrationDependencies,
): Promise<M1BrowserMigrationRunResult> {
  const lock =
    dependencies.lock === undefined
      ? webLocksMigrationLock()
      : dependencies.lock;
  if (lock === null) {
    return blocked(
      "lock_unavailable",
      "M1 browser migration requires same-origin exclusive locking",
    );
  }
  try {
    return await lock.runExclusive(M1_BROWSER_MIGRATION_LOCK_NAME, () =>
      runM1BrowserMigrationCriticalSection(dependencies),
    );
  } catch {
    return blocked(
      "lock_failed",
      "M1 browser migration could not acquire the same-origin lock",
    );
  }
}
