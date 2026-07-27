import {
  PRODUCT_APPROVAL_PREFERENCES,
  ProductApiSchemaError,
  parseUpdateProductPreferencesRequest,
  type ProductApprovalPreference,
  type ProductPreferences,
  type ProductProviderSelection,
  type ProductSessionId,
  type ProductThemePreference,
  type ProductWorkspaceId,
} from "../product/product-api-types";

export const PRODUCT_MEMORY_TYPES = [
  "user",
  "feedback",
  "project",
  "reference",
] as const;
export type ProductMemoryType = (typeof PRODUCT_MEMORY_TYPES)[number];

export const PRODUCT_MEMORY_SCOPES = [
  "global",
  "project",
  "session",
] as const;
export type ProductMemoryScope = (typeof PRODUCT_MEMORY_SCOPES)[number];

export interface ProductMemoryTopic {
  slug: string;
  title: string;
  memory_type: ProductMemoryType;
  scope: ProductMemoryScope;
  confidence: number;
  created_at?: string;
  updated_at?: string;
  description: string;
  metadata_truncated: boolean;
}

export interface ProductMemoryTopicsResponse {
  topics: ProductMemoryTopic[];
  total: number;
}

export interface ProductMemoryTopicContentResponse {
  topic: ProductMemoryTopic;
  content: string;
  truncated: boolean;
}

export type ProductConnectionStatus = "connected";
export type ProductStoreStatus = "ready" | "unavailable";
export type ProductResumeHealthStatus = "healthy" | "needs_attention";

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
  resume_health?: ProductResumeHealth;
}

export interface SettingsPreferencesUpdateRequest {
  schema_version: number;
  expected_revision: number;
  theme: ProductThemePreference;
  default_approval_policy: ProductApprovalPreference;
  active_workspace_id?: ProductWorkspaceId;
  active_session_id?: ProductSessionId;
  provider_selection?: ProductProviderSelection;
}

const MAX_PRODUCT_TEXT_BYTES = 512;
const MAX_MEMORY_TOPIC_SLUG_BYTES = 80;
const MAX_MEMORY_TOPICS = 200;
const MAX_MEMORY_CONTENT_BYTES = 64 * 1_024;
const MEMORY_TOPIC_SLUG_PATTERN =
  /^[\p{Alphabetic}\p{Number}]+(?:-[\p{Alphabetic}\p{Number}]+)*$/u;
type UnknownRecord = Record<string, unknown>;

function schemaError(path: string, expectation: string): never {
  throw new ProductApiSchemaError(`${path} must be ${expectation}`);
}

function expectRecord(value: unknown, path: string): UnknownRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return schemaError(path, "an object");
  }
  return value as UnknownRecord;
}

function expectOnlyKeys(
  value: UnknownRecord,
  allowed: readonly string[],
  path: string,
): void {
  const allowedKeys = new Set(allowed);
  const unknown = Object.keys(value).find((key) => !allowedKeys.has(key));
  if (unknown !== undefined) {
    schemaError(`${path}.${unknown}`, "a known field");
  }
}

function utf8Length(value: string): number {
  return new TextEncoder().encode(value).length;
}

function expectString(
  value: unknown,
  path: string,
  options: { nonEmpty?: boolean; maxBytes?: number; noControls?: boolean } = {},
): string {
  if (typeof value !== "string") {
    return schemaError(path, "a string");
  }
  if (options.nonEmpty && value.trim().length === 0) {
    return schemaError(path, "a non-empty string");
  }
  if (options.maxBytes !== undefined && utf8Length(value) > options.maxBytes) {
    return schemaError(path, `at most ${options.maxBytes} UTF-8 bytes`);
  }
  if (options.noControls && /[\u0000-\u001f\u007f-\u009f]/u.test(value)) {
    return schemaError(path, "free of control characters");
  }
  return value;
}

function expectBoolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    return schemaError(path, "a boolean");
  }
  return value;
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

function expectNumber(
  value: unknown,
  path: string,
  options: { min?: number; max?: number } = {},
): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return schemaError(path, "a finite number");
  }
  if (options.min !== undefined && value < options.min) {
    return schemaError(path, `at least ${options.min}`);
  }
  if (options.max !== undefined && value > options.max) {
    return schemaError(path, `at most ${options.max}`);
  }
  return value;
}

function expectEnum<const T extends readonly string[]>(
  value: unknown,
  values: T,
  path: string,
): T[number] {
  if (
    typeof value !== "string" ||
    !values.some((candidate) => candidate === value)
  ) {
    return schemaError(path, `one of ${values.join(", ")}`);
  }
  return value as T[number];
}

function optionalMetadata(
  record: UnknownRecord,
  key: string,
  path: string,
): string | undefined {
  if (record[key] === undefined) {
    return undefined;
  }
  return expectString(record[key], `${path}.${key}`, {
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControls: true,
  });
}

function expectMemorySlug(value: unknown, path: string): string {
  const slug = expectString(value, path, {
    nonEmpty: true,
    maxBytes: MAX_MEMORY_TOPIC_SLUG_BYTES,
    noControls: true,
  });
  if (!MEMORY_TOPIC_SLUG_PATTERN.test(slug)) {
    return schemaError(path, "a safe memory topic slug");
  }
  return slug;
}

export function validateProductMemorySlug(slug: string): string {
  return expectMemorySlug(slug, "product memory topic slug");
}

export function validateProductMemoryWorkspaceId(
  workspaceId: ProductWorkspaceId,
): ProductWorkspaceId {
  return expectString(workspaceId, "product memory workspace id", {
    nonEmpty: true,
    maxBytes: MAX_PRODUCT_TEXT_BYTES,
    noControls: true,
  });
}

export function parseProductMemoryTopic(
  value: unknown,
  path = "product memory topic",
): ProductMemoryTopic {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "slug",
      "title",
      "memory_type",
      "scope",
      "confidence",
      "created_at",
      "updated_at",
      "description",
      "metadata_truncated",
    ],
    path,
  );
  const topic: ProductMemoryTopic = {
    slug: expectMemorySlug(record.slug, `${path}.slug`),
    title: expectString(record.title, `${path}.title`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControls: true,
    }),
    memory_type: expectEnum(
      record.memory_type,
      PRODUCT_MEMORY_TYPES,
      `${path}.memory_type`,
    ),
    scope: expectEnum(record.scope, PRODUCT_MEMORY_SCOPES, `${path}.scope`),
    confidence: expectNumber(record.confidence, `${path}.confidence`, {
      min: 0,
      max: 1,
    }),
    description: expectString(record.description, `${path}.description`, {
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControls: true,
    }),
    metadata_truncated: expectBoolean(
      record.metadata_truncated,
      `${path}.metadata_truncated`,
    ),
  };
  const createdAt = optionalMetadata(record, "created_at", path);
  if (createdAt !== undefined) {
    topic.created_at = createdAt;
  }
  const updatedAt = optionalMetadata(record, "updated_at", path);
  if (updatedAt !== undefined) {
    topic.updated_at = updatedAt;
  }
  return topic;
}

export function parseProductMemoryTopicsResponse(
  value: unknown,
  path = "product memory topics response",
): ProductMemoryTopicsResponse {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["topics", "total"], path);
  if (!Array.isArray(record.topics)) {
    return schemaError(`${path}.topics`, "an array");
  }
  if (record.topics.length > MAX_MEMORY_TOPICS) {
    return schemaError(
      `${path}.topics`,
      `an array with at most ${MAX_MEMORY_TOPICS} items`,
    );
  }
  const topics = record.topics.map((topic, index) =>
    parseProductMemoryTopic(topic, `${path}.topics[${index}]`),
  );
  const total = expectInteger(record.total, `${path}.total`, {
    min: 0,
    max: MAX_MEMORY_TOPICS,
  });
  if (total !== topics.length) {
    return schemaError(`${path}.total`, "equal to the number of topics");
  }
  return { topics, total };
}

export function parseProductMemoryTopicContentResponse(
  value: unknown,
  path = "product memory topic content response",
): ProductMemoryTopicContentResponse {
  const record = expectRecord(value, path);
  expectOnlyKeys(record, ["topic", "content", "truncated"], path);
  return {
    topic: parseProductMemoryTopic(record.topic, `${path}.topic`),
    content: expectString(record.content, `${path}.content`, {
      maxBytes: MAX_MEMORY_CONTENT_BYTES,
    }),
    truncated: expectBoolean(record.truncated, `${path}.truncated`),
  };
}

function parseProductResumeHealth(
  value: unknown,
  path: string,
): ProductResumeHealth {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    [
      "status",
      "workspace_count",
      "session_count",
      "bound_session_count",
      "running_session_count",
      "needs_attention_session_count",
    ],
    path,
  );
  const health: ProductResumeHealth = {
    status: expectEnum(
      record.status,
      ["healthy", "needs_attention"] as const,
      `${path}.status`,
    ),
    workspace_count: expectInteger(
      record.workspace_count,
      `${path}.workspace_count`,
      { min: 0 },
    ),
    session_count: expectInteger(
      record.session_count,
      `${path}.session_count`,
      { min: 0 },
    ),
    bound_session_count: expectInteger(
      record.bound_session_count,
      `${path}.bound_session_count`,
      { min: 0 },
    ),
    running_session_count: expectInteger(
      record.running_session_count,
      `${path}.running_session_count`,
      { min: 0 },
    ),
    needs_attention_session_count: expectInteger(
      record.needs_attention_session_count,
      `${path}.needs_attention_session_count`,
      { min: 0 },
    ),
  };
  if (
    health.bound_session_count > health.session_count ||
    health.running_session_count > health.session_count ||
    health.needs_attention_session_count > health.session_count ||
    health.running_session_count + health.needs_attention_session_count >
      health.session_count
  ) {
    return schemaError(path, "internally consistent session counts");
  }
  const expectedStatus =
    health.needs_attention_session_count === 0
      ? "healthy"
      : "needs_attention";
  if (health.status !== expectedStatus) {
    return schemaError(
      `${path}.status`,
      "consistent with needs_attention_session_count",
    );
  }
  return health;
}

export function parseProductRuntimeInfo(
  value: unknown,
  path = "product runtime info",
): ProductRuntimeInfo {
  const record = expectRecord(value, path);
  expectOnlyKeys(
    record,
    ["api_version", "connection", "product_store", "resume_health"],
    path,
  );
  const info: ProductRuntimeInfo = {
    api_version: expectString(record.api_version, `${path}.api_version`, {
      nonEmpty: true,
      maxBytes: MAX_PRODUCT_TEXT_BYTES,
      noControls: true,
    }),
    connection: expectEnum(
      record.connection,
      ["connected"] as const,
      `${path}.connection`,
    ),
    product_store: expectEnum(
      record.product_store,
      ["ready", "unavailable"] as const,
      `${path}.product_store`,
    ),
  };
  if (record.resume_health !== undefined) {
    info.resume_health = parseProductResumeHealth(
      record.resume_health,
      `${path}.resume_health`,
    );
  }
  if (
    (info.product_store === "ready") !== (info.resume_health !== undefined)
  ) {
    return schemaError(
      `${path}.resume_health`,
      info.product_store === "ready"
        ? "present when the product store is ready"
        : "omitted when the product store is unavailable",
    );
  }
  return info;
}

export function parseSettingsPreferencesUpdateRequest(
  value: unknown,
  path = "settings preferences update request",
): SettingsPreferencesUpdateRequest {
  const request = parseUpdateProductPreferencesRequest(value, path);
  if (request.expected_revision === undefined) {
    return schemaError(`${path}.expected_revision`, "a required CAS revision");
  }
  if (request.expected_revision >= Number.MAX_SAFE_INTEGER) {
    return schemaError(
      `${path}.expected_revision`,
      `at most ${Number.MAX_SAFE_INTEGER - 1}`,
    );
  }
  if (request.default_approval_policy === undefined) {
    return schemaError(
      `${path}.default_approval_policy`,
      `one of ${PRODUCT_APPROVAL_PREFERENCES.join(", ")}`,
    );
  }
  const settingsRequest: SettingsPreferencesUpdateRequest = {
    schema_version: request.schema_version,
    expected_revision: request.expected_revision,
    theme: request.theme,
    default_approval_policy: request.default_approval_policy,
  };
  if (request.active_workspace_id !== undefined) {
    settingsRequest.active_workspace_id = request.active_workspace_id;
  }
  if (request.active_session_id !== undefined) {
    settingsRequest.active_session_id = request.active_session_id;
  }
  if (request.provider_selection !== undefined) {
    settingsRequest.provider_selection = {
      ...request.provider_selection,
      approval: request.default_approval_policy,
    };
  }
  return settingsRequest;
}

export type { ProductApprovalPreference, ProductPreferences };
